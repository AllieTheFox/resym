use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    ops::Range,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, LazyLock, Mutex,
    },
};

use eframe::{
    egui,
    epaint::text::{LayoutJob, LayoutSection, TextFormat, TextWrapping},
};
use syntect::{easy::HighlightLines, highlighting::FontStyle, util::LinesWithEndings};

use resym_core::{diffing::DiffChange, syntax_highlighting::CodeTheme};

pub type LineDescriptions = Vec<DiffChange>;

/// Syntax definitions and themes, loaded once
static CODE_HIGHLIGHTER: LazyLock<CodeHighlighter> = LazyLock::new(|| CodeHighlighter {
    ts: syntect::highlighting::ThemeSet::load_defaults(),
});

/// The highlighted text shown by the code view
#[derive(Default)]
struct HighlightedCode {
    /// Hash of the highlighted text and of how it's highlighted
    key: Option<u64>,
    /// `None` while the text is being highlighted
    layout_job: Option<LayoutJob>,
    /// Set when the text isn't shown anymore, to stop highlighting it
    cancelled: Arc<AtomicBool>,
}

/// Returns the layout of `code`, highlighted in the background (on all the
/// CPU's cores if `use_all_cores` is set): the text is returned without
/// highlighting until then. wasm32 targets can't start threads and highlight
/// synchronously.
pub fn highlight_code(
    ctx: &egui::Context,
    theme: &CodeTheme,
    code: &str,
    enabled: bool,
    line_descriptions: Option<&LineDescriptions>,
    use_all_cores: bool,
) -> LayoutJob {
    if !enabled {
        return plain_layout_job(theme, code);
    }

    let mut hasher = DefaultHasher::new();
    (theme, code, line_descriptions).hash(&mut hasher);
    let key = hasher.finish();
    let highlighted_code = ctx.memory_mut(|memory| {
        memory
            .data
            .get_temp_mut_or_default::<Arc<Mutex<HighlightedCode>>>(egui::Id::new(
                "highlighted_code",
            ))
            .clone()
    });
    let mut highlighted = highlighted_code.lock().expect("lock shouldn't be poisoned");
    if highlighted.key != Some(key) {
        highlighted.cancelled.store(true, Ordering::Relaxed);
        let cancelled = Arc::new(AtomicBool::new(false));
        *highlighted = HighlightedCode {
            key: Some(key),
            layout_job: None,
            cancelled: cancelled.clone(),
        };

        #[cfg(not(target_arch = "wasm32"))]
        {
            let ctx = ctx.clone();
            let highlighted_code = highlighted_code.clone();
            let theme = theme.clone();
            let code = code.to_owned();
            let line_descriptions = line_descriptions.cloned();
            let thread_count = if use_all_cores {
                std::thread::available_parallelism().map_or(1, |count| count.get())
            } else {
                1
            };
            std::thread::spawn(move || {
                let layout_job = CODE_HIGHLIGHTER.highlight(
                    &theme,
                    &code,
                    line_descriptions.as_ref(),
                    thread_count,
                    &cancelled,
                );
                if let Some(layout_job) = layout_job {
                    let mut highlighted =
                        highlighted_code.lock().expect("lock shouldn't be poisoned");
                    if highlighted.key == Some(key) {
                        highlighted.layout_job = Some(layout_job);
                        ctx.request_repaint();
                    }
                }
            });
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = use_all_cores;
            highlighted.layout_job =
                CODE_HIGHLIGHTER.highlight(theme, code, line_descriptions, 1, &cancelled);
        }
    }

    highlighted
        .layout_job
        .clone()
        .unwrap_or_else(|| plain_layout_job(theme, code))
}

/// Layout of `code` without highlighting
fn plain_layout_job(theme: &CodeTheme, code: &str) -> LayoutJob {
    LayoutJob::simple(
        code.into(),
        egui::FontId::monospace(theme.font_size as f32),
        if theme.dark_mode {
            egui::Color32::LIGHT_GRAY
        } else {
            egui::Color32::DARK_GRAY
        },
        f32::INFINITY,
    )
}

struct CodeHighlighter {
    ts: syntect::highlighting::ThemeSet,
}

/// Lines highlighted together, starting from the state syntect is in after
/// the `prefix` lines
struct Chunk {
    lines: Range<usize>,
    prefix: Vec<usize>,
}

impl CodeHighlighter {
    /// Highlights `text` with `thread_count` threads. Returns `None` if
    /// `cancelled` gets set.
    fn highlight(
        &self,
        theme: &CodeTheme,
        text: &str,
        line_descriptions: Option<&LineDescriptions>,
        thread_count: usize,
        cancelled: &AtomicBool,
    ) -> Option<LayoutJob> {
        let syntect_theme = &self.ts.themes[theme.syntect_theme.syntect_key_name()];
        let lines: Vec<&str> = LinesWithEndings::from(text).collect();
        let chunks = definition_chunks(&lines);
        let next_chunk = AtomicUsize::new(0);
        let chunk_sections: Mutex<Vec<Option<Vec<LayoutSection>>>> =
            Mutex::new(vec![None; chunks.len()]);
        let failed = AtomicBool::new(false);

        let highlight_chunks = || {
            // Compiled regexes share caches between threads: each thread has
            // its own syntax definitions (they load in a few milliseconds)
            let syntax_set = syntect::parsing::SyntaxSet::load_defaults_newlines();
            let Some(syntax) = syntax_set
                .find_syntax_by_name(&theme.language_syntax)
                .or_else(|| syntax_set.find_syntax_by_extension(&theme.language_syntax))
            else {
                failed.store(true, Ordering::Relaxed);
                return;
            };
            loop {
                let chunk_index = next_chunk.fetch_add(1, Ordering::Relaxed);
                let Some(chunk) = chunks.get(chunk_index) else {
                    return;
                };
                let mut highlighter = HighlightLines::new(syntax, syntect_theme);
                for prefix_line in &chunk.prefix {
                    if highlighter
                        .highlight_line(lines[*prefix_line], &syntax_set)
                        .is_err()
                    {
                        failed.store(true, Ordering::Relaxed);
                        return;
                    }
                }
                let mut sections = Vec::new();
                for line_index in chunk.lines.clone() {
                    if cancelled.load(Ordering::Relaxed) || failed.load(Ordering::Relaxed) {
                        return;
                    }
                    let Ok(ranges) = highlighter.highlight_line(lines[line_index], &syntax_set)
                    else {
                        failed.store(true, Ordering::Relaxed);
                        return;
                    };
                    // Changed lines of diffs have a colored background
                    let background = match line_descriptions.and_then(|d| d.get(line_index)) {
                        Some(DiffChange::Insert) => egui::Color32::from_rgb(0x10, 0x50, 0x10),
                        Some(DiffChange::Delete) => egui::Color32::from_rgb(0x50, 0x10, 0x10),
                        Some(DiffChange::Equal) | None => egui::Color32::TRANSPARENT,
                    };
                    for (style, range) in ranges {
                        let fg = style.foreground;
                        let color = egui::Color32::from_rgb(fg.r, fg.g, fg.b);
                        let italics = style.font_style.contains(FontStyle::ITALIC);
                        let underline = if italics {
                            egui::Stroke::new(1.0, color)
                        } else {
                            egui::Stroke::NONE
                        };
                        sections.push(LayoutSection {
                            leading_space: 0.0,
                            byte_range: as_byte_range(text, range),
                            format: TextFormat {
                                background,
                                font_id: egui::FontId::monospace(theme.font_size as f32),
                                color,
                                italics,
                                underline,
                                ..Default::default()
                            },
                        });
                    }
                }
                chunk_sections.lock().expect("lock shouldn't be poisoned")[chunk_index] =
                    Some(sections);
            }
        };
        if thread_count > 1 {
            std::thread::scope(|scope| {
                for _ in 1..thread_count {
                    scope.spawn(highlight_chunks);
                }
                highlight_chunks();
            });
        } else {
            highlight_chunks();
        }

        if cancelled.load(Ordering::Relaxed) {
            return None;
        }
        if failed.load(Ordering::Relaxed) {
            return Some(plain_layout_job(theme, text));
        }
        Some(LayoutJob {
            text: text.into(),
            sections: chunk_sections
                .into_inner()
                .expect("lock shouldn't be poisoned")
                .into_iter()
                .flatten()
                .flatten()
                .collect(),
            // Disable wrapping forcefully
            wrap: TextWrapping {
                max_width: f32::INFINITY,
                ..Default::default()
            },
            ..Default::default()
        })
    }
}

/// Splits `lines` into chunks starting at each top-level definition (after a
/// blank line, outside of any block but namespace blocks). Highlighting each
/// chunk from the state after the opening lines of its namespace blocks lets
/// chunks be highlighted in parallel, and keeps a definition syntect parses
/// wrongly from changing the highlighting of the following ones.
fn definition_chunks(lines: &[&str]) -> Vec<Chunk> {
    let is_namespace_opening = |line: &str| {
        line == "namespace" || line.starts_with("namespace ") || line.starts_with("namespace{")
    };
    // Open namespace blocks: their opening lines, and the brace depth of
    // their content
    let mut namespaces: Vec<(Vec<usize>, usize)> = Vec::new();
    let mut depth = 0usize;
    let mut chunks = Vec::new();
    let mut chunk_start = 0;
    let mut chunk_prefix = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let namespace_content_depth = namespaces.last().map_or(0, |(_, depth)| *depth);
        if index > chunk_start
            && lines[index - 1].trim().is_empty()
            && depth == namespace_content_depth
        {
            chunks.push(Chunk {
                lines: chunk_start..index,
                prefix: std::mem::take(&mut chunk_prefix),
            });
            chunk_start = index;
            chunk_prefix = namespaces
                .iter()
                .flat_map(|(opening_lines, _)| opening_lines.iter().copied())
                .collect();
        }

        let line = line.trim();
        let opened = line.matches('{').count();
        let closed = line.matches('}').count();
        if is_namespace_opening(line) && opened > closed {
            namespaces.push((vec![index], depth + opened - closed));
        } else if line == "{" && index > 0 {
            // Opening brace on its own line
            let previous_line = lines[index - 1].trim();
            if is_namespace_opening(previous_line) && !previous_line.contains('{') {
                namespaces.push((vec![index - 1, index], depth + 1));
            }
        }
        depth = (depth + opened).saturating_sub(closed);
        while namespaces
            .last()
            .is_some_and(|(_, content_depth)| depth < *content_depth)
        {
            namespaces.pop();
        }
    }
    chunks.push(Chunk {
        lines: chunk_start..lines.len(),
        prefix: chunk_prefix,
    });
    chunks
}

fn as_byte_range(whole: &str, range: &str) -> Range<usize> {
    let whole_start = whole.as_ptr() as usize;
    let range_start = range.as_ptr() as usize;
    assert!(whole_start <= range_start);
    assert!(range_start + range.len() <= whole_start + whole.len());
    let offset = range_start - whole_start;
    offset..(offset + range.len())
}
