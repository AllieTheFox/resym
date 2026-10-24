use eframe::egui;
use resym_core::pdb_types::{
    AccessSpecifierReconstructionFlavor, IndentationFlavor, PrimitiveReconstructionFlavor,
    SizePrintFlavor, VftableCommentFlavor,
};

use crate::settings::ResymAppSettings;

/// Tabs available for the settings window
#[derive(Clone, Copy, PartialEq)]
enum SettingsTab {
    General,
    Types,
    Methods,
    Output,
}

impl SettingsTab {
    const ALL: [SettingsTab; 4] = [
        SettingsTab::General,
        SettingsTab::Types,
        SettingsTab::Methods,
        SettingsTab::Output,
    ];

    fn title(self) -> &'static str {
        match self {
            SettingsTab::General => "General",
            SettingsTab::Types => "Types",
            SettingsTab::Methods => "Methods",
            SettingsTab::Output => "Output",
        }
    }
}

pub struct SettingsComponent {
    settings_tab: SettingsTab,
    window_open: bool,
    pub app_settings: ResymAppSettings,
}

impl SettingsComponent {
    pub fn new(app_settings: ResymAppSettings) -> Self {
        Self {
            settings_tab: SettingsTab::General,
            window_open: false,
            app_settings,
        }
    }

    pub fn open(&mut self) {
        self.window_open = true;
    }

    pub fn update(&mut self, ctx: &egui::Context) {
        egui::Window::new("Settings")
            .anchor(egui::Align2::CENTER_CENTER, [0.0; 2])
            .open(&mut self.window_open)
            .auto_sized()
            .collapsible(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for tab in SettingsTab::ALL {
                        ui.selectable_value(&mut self.settings_tab, tab, tab.title());
                    }
                });
                // Note: no separator, it would stretch the auto-sized window
                // to the available width
                ui.add_space(4.0);

                // Keep the window the same size on every tab: lay out every
                // tab invisibly (and without interaction) to measure it, then
                // show the selected tab in an area the size of the largest one.
                let mut body_size = egui::Vec2::ZERO;
                for tab in SettingsTab::ALL {
                    let mut hidden_ui = ui.new_child(
                        egui::UiBuilder::new()
                            .id_salt(tab.title())
                            .max_rect(ui.available_rect_before_wrap())
                            .invisible(),
                    );
                    tab_ui(&mut hidden_ui, tab, &mut self.app_settings);
                    body_size = body_size.max(hidden_ui.min_rect().size());
                }
                let (body_rect, _) = ui.allocate_exact_size(body_size, egui::Sense::hover());
                let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(body_rect));
                tab_ui(&mut body_ui, self.settings_tab, &mut self.app_settings);
            });
    }
}

/// Lays out the settings of the given tab.
fn tab_ui(ui: &mut egui::Ui, tab: SettingsTab, settings: &mut ResymAppSettings) {
    const INTER_SECTION_SPACING: f32 = 10.0;
    match tab {
        SettingsTab::General => {
            egui::Grid::new("general_settings")
                .num_columns(2)
                .show(ui, |ui| {
                    ui.label("Theme");
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut settings.use_light_theme, true, "☀ Light");
                        ui.selectable_value(&mut settings.use_light_theme, false, "🌙 Dark");
                    });
                    ui.end_row();

                    ui.label("Font size");
                    egui::ComboBox::from_id_salt("font_size")
                        .selected_text(settings.font_size.to_string())
                        .show_ui(ui, |ui| {
                            for font_size in 8..=20 {
                                ui.selectable_value(
                                    &mut settings.font_size,
                                    font_size,
                                    font_size.to_string(),
                                );
                            }
                        });
                    ui.end_row();
                });

            ui.add_space(INTER_SECTION_SPACING);
            ui.label("Code view");
            ui.checkbox(
                &mut settings.enable_syntax_hightlighting,
                "Enable C++ syntax highlighting",
            );
            ui.checkbox(&mut settings.print_line_numbers, "Print line numbers");

            ui.add_space(INTER_SECTION_SPACING);
            ui.label("Search");
            ui.checkbox(&mut settings.search_case_insensitive, "Case insensitive");
            ui.checkbox(&mut settings.search_use_regex, "Enable regular expressions");
        }
        SettingsTab::Types => {
            egui::Grid::new("type_settings")
                .num_columns(2)
                .show(ui, |ui| {
                    combo_row(
                        ui,
                        "Primitive types",
                        &mut settings.primitive_types_flavor,
                        &[
                            (PrimitiveReconstructionFlavor::Portable, "Portable"),
                            (PrimitiveReconstructionFlavor::Microsoft, "Microsoft"),
                            (PrimitiveReconstructionFlavor::Raw, "Raw"),
                            (PrimitiveReconstructionFlavor::Msvc, "MSVC"),
                        ],
                    );
                    combo_row(
                        ui,
                        "Access specifiers",
                        &mut settings.print_access_specifiers,
                        &[
                            (AccessSpecifierReconstructionFlavor::Automatic, "Automatic"),
                            (AccessSpecifierReconstructionFlavor::Always, "Always"),
                            (AccessSpecifierReconstructionFlavor::Disabled, "Disabled"),
                        ],
                    );
                    combo_row(
                        ui,
                        "Size comments",
                        &mut settings.size_print_flavor,
                        &[
                            (SizePrintFlavor::Comment, "Comment"),
                            (SizePrintFlavor::StaticAssert, "Static assert"),
                            (SizePrintFlavor::Disabled, "Disabled"),
                        ],
                    );
                });

            ui.add_space(INTER_SECTION_SPACING);
            ui.checkbox(
                &mut settings.preserve_member_order,
                "Preserve PDB member order",
            );
            ui.checkbox(
                &mut settings.reconstruct_namespaces,
                "Reconstruct namespaces",
            );
            ui.checkbox(
                &mut settings.print_elaborated_type_specifiers,
                "Print struct/class/union/enum before type names",
            );
            ui.checkbox(&mut settings.print_offset_info, "Print offset comments");
            ui.checkbox(
                &mut settings.hide_base_class_offsets,
                "Hide base class offset comments",
            );
            ui.checkbox(
                &mut settings.integers_as_hexadecimal,
                "Print integer values as hexadecimal",
            );
        }
        SettingsTab::Methods => {
            egui::Grid::new("method_settings")
                .num_columns(2)
                .show(ui, |ui| {
                    combo_row(
                        ui,
                        "Vftable entry comments",
                        &mut settings.vftable_comment_flavor,
                        &[
                            (VftableCommentFlavor::Offset, "Offset"),
                            (VftableCommentFlavor::Index, "Index"),
                            (VftableCommentFlavor::Disabled, "Disabled"),
                        ],
                    );
                });

            ui.add_space(INTER_SECTION_SPACING);
            ui.checkbox(&mut settings.print_argument_names, "Print argument names");
            ui.checkbox(&mut settings.print_method_rvas, "Print method RVAs");
            ui.checkbox(
                &mut settings.print_calling_conventions,
                "Print calling conventions",
            );
            ui.checkbox(
                &mut settings.print_override_specifiers,
                "Print override specifiers",
            );
            // Only applies to methods printed with `override`
            ui.indent("omit_virtual_on_overrides", |ui| {
                ui.add_enabled_ui(settings.print_override_specifiers, |ui| {
                    ui.checkbox(
                        &mut settings.omit_virtual_on_overrides,
                        "Omit virtual on overrides",
                    );
                });
            });
            ui.checkbox(
                &mut settings.ignore_compiler_generated_methods,
                "Ignore compiler generated methods",
            );
        }
        SettingsTab::Output => {
            egui::Grid::new("output_settings")
                .num_columns(2)
                .show(ui, |ui| {
                    combo_row(
                        ui,
                        "Indentation",
                        &mut settings.indentation,
                        &[
                            (IndentationFlavor::TwoSpaces, "2 spaces"),
                            (IndentationFlavor::FourSpaces, "4 spaces"),
                            (IndentationFlavor::Tab, "Tab"),
                        ],
                    );
                });

            ui.add_space(INTER_SECTION_SPACING);
            ui.checkbox(&mut settings.print_header, "Print header");
            ui.checkbox(
                &mut settings.reconstruct_dependencies,
                "Print definitions of referenced types",
            );
            ui.checkbox(
                &mut settings.ignore_std_types,
                "Ignore types from the std namespace",
            );
            ui.checkbox(
                &mut settings.print_brackets_new_line,
                "Print all brackets on new line",
            );
            ui.checkbox(
                &mut settings.compact_output,
                "Compact output (no blank lines between members)",
            );
        }
    }
}

/// Adds a grid row with a label and a combo box selecting one of `options`.
fn combo_row<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut T,
    options: &[(T, &str)],
) {
    ui.label(label);
    let selected_text = options
        .iter()
        .find(|(option, _)| option == value)
        .map_or("", |(_, text)| *text);
    egui::ComboBox::from_id_salt(label)
        .selected_text(selected_text)
        .show_ui(ui, |ui| {
            for (option, text) in options {
                ui.selectable_value(value, *option, *text);
            }
        });
    ui.end_row();
}
