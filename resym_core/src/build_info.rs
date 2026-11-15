//! Compiler build metadata recorded for an individual module, not inferred source paths.

use std::{collections::HashMap, fmt};

use pdb::FallibleIterator;

use crate::error::{Result, ResymCoreError};

pub const BUILD_INFO_ARGUMENT_NAMES: [&str; 5] = [
    "Working directory",
    "Compiler",
    "Source argument",
    "Compiler PDB",
    "Compiler options",
];

/// The arguments of a module's `LF_BUILDINFO`, in their recorded order.
///
/// Zero references are unavailable (`None`); empty strings remain available. The
/// vector retains the recorded count, including arguments beyond the five known fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleBuildInfo {
    pub item_index: pdb::IdIndex,
    pub arguments: Vec<Option<String>>,
}

impl fmt::Display for ModuleBuildInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Build-info item: 0x{:08X}", self.item_index.0)?;
        for index in 0..self.arguments.len().max(BUILD_INFO_ARGUMENT_NAMES.len()) {
            if let Some(name) = BUILD_INFO_ARGUMENT_NAMES.get(index) {
                write!(f, "{name}: ")?;
            } else {
                write!(f, "Argument {}: ", index + 1)?;
            }
            match self.arguments.get(index).and_then(Option::as_ref) {
                Some(value) => writeln!(f, "{value}")?,
                None => writeln!(f, "<unavailable>")?,
            }
        }
        Ok(())
    }
}

// Bound both output and traversal: a DAG of empty strings can expand
// exponentially without producing a single byte. These limits apply to the
// complete build-info record, not just to an individual argument.
const MAX_BUILD_INFO_BYTES: usize = 16 * 1024 * 1024;
const MAX_EXPANDED_ITEMS: usize = 1_000_000;
const MAX_GRAPH_NODES: usize = 65_536;

fn invalid(message: impl Into<String>) -> ResymCoreError {
    ResymCoreError::InvalidBuildInfoError(message.into())
}

struct Fragment<'a> {
    prefix: Option<pdb::IdIndex>,
    // Keep the parser's list allocation; convert each index to IPI at lookup.
    children: Vec<pdb::TypeIndex>,
    suffix: &'a str,
    // None while on the explicit DFS stack, Some after every child is validated.
    expansion: Option<(usize, usize)>,
}

impl Fragment<'_> {
    fn child(&self, mut index: usize) -> Option<pdb::IdIndex> {
        if let Some(prefix) = self.prefix {
            if index == 0 {
                return Some(prefix);
            }
            index -= 1;
        }
        self.children.get(index).map(|child| pdb::IdIndex(child.0))
    }

    fn dependencies(&self) -> impl Iterator<Item = pdb::IdIndex> + '_ {
        self.prefix
            .into_iter()
            .chain(self.children.iter().map(|child| pdb::IdIndex(child.0)))
    }
}

fn fragment<'a>(finder: &pdb::IdFinder<'a>, index: pdb::IdIndex) -> Result<Fragment<'a>> {
    let (prefix, children, suffix) = match finder.find(index)?.parse()? {
        pdb::IdData::String(string) => (
            string.substrings,
            Vec::new(),
            std::str::from_utf8(string.name.as_bytes())
                .map_err(|_| invalid(format!("item 0x{:X} is not UTF-8", index.0)))?,
        ),
        pdb::IdData::StringList(list) => (None, list.substrings, ""),
        _ => {
            return Err(invalid(format!(
                "item 0x{:X} is not a string or substring list",
                index.0
            )))
        }
    };
    Ok(Fragment {
        prefix,
        children,
        suffix,
        expansion: None,
    })
}

/// Resolve only the graph reached from the module's actual S_BUILDINFO item.
/// No partial argument strings escape when any reference is malformed.
pub(crate) fn resolve_build_info(
    information: &pdb::IdInformation<'_>,
    item_index: pdb::IdIndex,
) -> Result<ModuleBuildInfo> {
    let mut finder = information.finder();
    let mut items = information.iter();
    let mut count = 0;
    while items.next()?.is_some() {
        count += 1;
        if count > information.len() {
            return Err(invalid("IPI record count exceeds the declared index range"));
        }
        finder.update(&items);
    }
    if count != information.len() {
        return Err(invalid(
            "IPI record count does not match the declared index range",
        ));
    }
    let arguments = match finder.find(item_index)?.parse()? {
        pdb::IdData::BuildInfo(build_info) => build_info.arguments,
        _ => {
            return Err(invalid(format!(
                "item 0x{:X} is not LF_BUILDINFO",
                item_index.0
            )))
        }
    };

    let mut fragments = HashMap::new();
    let mut total_bytes = 0usize;
    let mut total_items = 0usize;
    // Validate and size the entire graph first. Cache graph nodes and sizes,
    // not expanded strings, avoiding quadratic copying along long prefixes.
    for &argument in &arguments {
        if argument.0 == 0 {
            continue;
        }
        if !fragments.contains_key(&argument) {
            if fragments.len() >= MAX_GRAPH_NODES {
                return Err(invalid("too many build-info string fragments"));
            }
            fragments.insert(argument, fragment(&finder, argument)?);
            let mut stack = vec![(argument, 0usize)];
            while let Some(&(index, next_child)) = stack.last() {
                let node = &fragments[&index];
                if let Some(child) = node.child(next_child) {
                    stack.last_mut().expect("nonempty stack").1 += 1;
                    if let Some(existing) = fragments.get(&child) {
                        if existing.expansion.is_none() {
                            return Err(invalid(format!(
                                "cycle at build-info item 0x{:X}",
                                child.0
                            )));
                        }
                    } else {
                        if fragments.len() >= MAX_GRAPH_NODES {
                            return Err(invalid("too many build-info string fragments"));
                        }
                        fragments.insert(child, fragment(&finder, child)?);
                        stack.push((child, 0));
                    }
                } else {
                    let mut bytes = node.suffix.len();
                    let mut expanded_items = 1usize;
                    for child in node.dependencies() {
                        let (child_bytes, child_items) = fragments[&child]
                            .expansion
                            .expect("children validated before their parent");
                        bytes = bytes
                            .checked_add(child_bytes)
                            .filter(|&size| size <= MAX_BUILD_INFO_BYTES)
                            .ok_or_else(|| invalid("build-info string expansion is too large"))?;
                        expanded_items = expanded_items
                            .checked_add(child_items)
                            .filter(|&size| size <= MAX_EXPANDED_ITEMS)
                            .ok_or_else(|| {
                                invalid("build-info string expansion has too many fragments")
                            })?;
                    }
                    fragments
                        .get_mut(&index)
                        .expect("visited fragment")
                        .expansion = Some((bytes, expanded_items));
                    stack.pop();
                }
            }
        }
        let (bytes, expanded_items) = fragments[&argument].expansion.expect("validated argument");
        total_bytes = total_bytes
            .checked_add(bytes)
            .filter(|&size| size <= MAX_BUILD_INFO_BYTES)
            .ok_or_else(|| invalid("build-info arguments are too large"))?;
        total_items = total_items
            .checked_add(expanded_items)
            .filter(|&size| size <= MAX_EXPANDED_ITEMS)
            .ok_or_else(|| invalid("build-info arguments have too many fragments"))?;
    }

    let mut resolved = Vec::with_capacity(arguments.len());
    for argument in arguments {
        if argument.0 == 0 {
            resolved.push(None);
            continue;
        }
        let mut value = String::with_capacity(
            fragments[&argument]
                .expansion
                .expect("validated argument")
                .0,
        );
        let mut stack = vec![(argument, 0usize)];
        while let Some(&(index, next_child)) = stack.last() {
            let node = &fragments[&index];
            if let Some(child) = node.child(next_child) {
                stack.last_mut().expect("nonempty stack").1 += 1;
                stack.push((child, 0));
            } else {
                value.push_str(node.suffix);
                stack.pop();
            }
        }
        resolved.push(Some(value));
    }
    Ok(ModuleBuildInfo {
        item_index,
        arguments: resolved,
    })
}

#[cfg(test)]
mod tests {
    use super::{resolve_build_info, ModuleBuildInfo};

    fn string(prefix: u32, text: &str) -> Vec<u8> {
        let mut record = 0x1605u16.to_le_bytes().to_vec();
        record.extend(prefix.to_le_bytes());
        record.extend(text.as_bytes());
        record.push(0);
        record
    }

    fn list(children: &[u32]) -> Vec<u8> {
        let mut record = 0x1604u16.to_le_bytes().to_vec();
        record.extend((children.len() as u32).to_le_bytes());
        for child in children {
            record.extend(child.to_le_bytes());
        }
        record
    }

    fn build(arguments: &[u32]) -> Vec<u8> {
        let mut record = 0x1603u16.to_le_bytes().to_vec();
        record.extend((arguments.len() as u16).to_le_bytes());
        for argument in arguments {
            record.extend(argument.to_le_bytes());
        }
        record
    }

    // A tiny synthetic MSF container lets the existing public parser frame,
    // validate, iterate and find ID records, without another parser or a private PDB.
    fn fixture(records: &[Vec<u8>]) -> Vec<u8> {
        const PAGE: usize = 4096;
        let mut ipi = Vec::new();
        for field in [20040203u32, 56, 0x1000, 0x1000 + records.len() as u32, 0] {
            ipi.extend(field.to_le_bytes());
        }
        ipi.resize(56, 0);
        for record in records {
            ipi.extend((record.len() as u16).to_le_bytes());
            ipi.extend(record);
        }
        let record_bytes = (ipi.len() - 56) as u32;
        ipi[16..20].copy_from_slice(&record_bytes.to_le_bytes());
        let stream_pages = ipi.len().div_ceil(PAGE);
        let mut file = vec![0; (5 + stream_pages) * PAGE];
        file[..32].copy_from_slice(b"Microsoft C/C++ MSF 7.00\r\n\x1a\x44\x53\x00\x00\x00");
        for (offset, field) in [
            (32, PAGE as u32),
            (36, 1),
            (40, (5 + stream_pages) as u32),
            (44, (24 + stream_pages * 4) as u32),
            (48, 0),
            (52, 3),
            (3 * PAGE, 4),
            (4 * PAGE, 5),
        ] {
            file[offset..offset + 4].copy_from_slice(&field.to_le_bytes());
        }
        for index in 0..4 {
            let offset = 4 * PAGE + 4 + index * 4;
            file[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        }
        file[4 * PAGE + 20..4 * PAGE + 24].copy_from_slice(&(ipi.len() as u32).to_le_bytes());
        for page in 0..stream_pages {
            let offset = 4 * PAGE + 24 + page * 4;
            file[offset..offset + 4].copy_from_slice(&((5 + page) as u32).to_le_bytes());
        }
        file[5 * PAGE..5 * PAGE + ipi.len()].copy_from_slice(&ipi);
        file
    }

    fn resolve(records: &[Vec<u8>], index: u32) -> crate::Result<ModuleBuildInfo> {
        let mut pdb = pdb::PDB::open(std::io::Cursor::new(fixture(records)))?;
        let information = pdb.id_information()?;
        resolve_build_info(&information, pdb::IdIndex(index))
    }

    #[test]
    fn nested_fragments_preserve_split_quotes_spaces_and_argument_order() {
        let records = [
            string(0, "-DNAME=\""),
            string(0, "two "),
            list(&[0x1000, 0x1001]),
            string(0x1002, "words\" "),
            list(&[0x1003, 0x1001]),
            string(0x1004, "-I\"inc dir\""),
            build(&[0, 0x1001, 0, 0, 0x1005, 0x1000]),
        ];
        let info = resolve(&records, 0x1006).expect("resolve complete graph");
        assert_eq!(info.item_index, pdb::IdIndex(0x1006));
        assert_eq!(
            info.arguments,
            vec![
                None,
                Some("two ".into()),
                None,
                None,
                Some("-DNAME=\"two words\" two -I\"inc dir\"".into()),
                Some("-DNAME=\"".into()),
            ]
        );
    }

    #[test]
    fn missing_arguments_stay_missing_and_empty_strings_stay_available() {
        let info = resolve(&[string(0, ""), build(&[0, 0x1000])], 0x1001).unwrap();
        assert_eq!(info.arguments, vec![None, Some(String::new())]);
        let empty = resolve(&[build(&[])], 0x1000).unwrap();
        assert!(empty.arguments.is_empty());
    }

    #[test]
    fn dangling_wrong_kind_unknown_and_truncated_records_fail_the_whole_result() {
        let cases = [
            vec![string(0, "valid"), build(&[0x1000, 0x2000])],
            vec![build(&[]), build(&[0x1000])],
            vec![0x7777u16.to_le_bytes().to_vec(), build(&[0x1000])],
            vec![vec![0x05, 0x16, 0, 0], build(&[0x1000])],
            vec![list(&[0]), build(&[0x1000])],
            vec![vec![0x05, 0x16, 0, 0, 0, 0, b'x'], build(&[0x1000])],
            vec![vec![0x04, 0x16, 3, 0, 0, 0], build(&[0x1000])],
        ];
        for records in cases {
            assert!(resolve(&records, 0x1001).is_err());
        }
        assert!(resolve(&[string(0, "not build info")], 0x1000).is_err());
    }

    #[test]
    fn invalid_encoding_is_an_error_instead_of_lossy_command_text() {
        let record = vec![0x05, 0x16, 0, 0, 0, 0, 0xff, 0];
        let error = resolve(&[record, build(&[0x1000])], 0x1001).unwrap_err();
        assert!(matches!(
            error,
            crate::ResymCoreError::InvalidBuildInfoError(_)
        ));
    }

    #[test]
    fn malformed_ipi_header_and_incomplete_index_range_are_errors() {
        let mut data = fixture(&[build(&[])]);
        // HeaderSize is structurally invalid: the named parser must reject it.
        data[5 * 4096 + 4..5 * 4096 + 8].copy_from_slice(&4u32.to_le_bytes());
        let mut pdb = pdb::PDB::open(std::io::Cursor::new(data)).unwrap();
        assert!(pdb.id_information().is_err());

        let mut data = fixture(&[build(&[])]);
        data[5 * 4096 + 12..5 * 4096 + 16].copy_from_slice(&0x1002u32.to_le_bytes());
        let mut pdb = pdb::PDB::open(std::io::Cursor::new(data)).unwrap();
        let information = pdb.id_information().unwrap();
        let error = resolve_build_info(&information, pdb::IdIndex(0x1000)).unwrap_err();
        assert!(matches!(
            error,
            crate::ResymCoreError::InvalidBuildInfoError(_)
        ));
    }

    #[test]
    fn direct_and_indirect_cycles_fail_without_partial_arguments() {
        assert!(resolve(&[string(0x1000, "tail"), build(&[0x1000])], 0x1001).is_err());
        assert!(resolve(
            &[
                string(0x1001, "tail"),
                list(&[0x1002]),
                list(&[0x1000]),
                build(&[0x1000]),
            ],
            0x1003
        )
        .is_err());
    }

    #[test]
    fn deep_prefix_graphs_do_not_use_the_call_stack() {
        let mut records = vec![string(0, "start")];
        for index in 1..5000 {
            records.push(string(0x1000 + index - 1, "x"));
        }
        records.push(build(&[0x1000 + 4999]));
        let info = resolve(&records, 0x1000 + 5000).unwrap();
        assert_eq!(
            info.arguments[0].as_deref(),
            Some(format!("start{}", "x".repeat(4999)).as_str())
        );
    }

    #[test]
    fn exponential_empty_and_nonempty_expansions_are_bounded() {
        for text in [
            "",
            "a long repeated compiler option of at least sixty-four bytes ... ",
        ] {
            let mut records = vec![string(0, text)];
            for index in 1..26 {
                let previous = 0x1000 + index - 1;
                records.push(list(&[previous, previous]));
            }
            records.push(build(&[0x1000 + 25]));
            let error = resolve(&records, 0x1000 + 26).unwrap_err();
            assert!(matches!(
                error,
                crate::ResymCoreError::InvalidBuildInfoError(_)
            ));
        }
    }

    #[test]
    fn graph_size_is_bounded_even_without_recursive_expansion() {
        let mut records = vec![string(0, "")];
        for index in 1..=super::MAX_GRAPH_NODES as u32 {
            records.push(string(0x1000 + index - 1, ""));
        }
        let last = 0x1000 + super::MAX_GRAPH_NODES as u32;
        records.push(build(&[last]));
        let error = resolve(&records, last + 1).unwrap_err();
        assert!(matches!(
            error,
            crate::ResymCoreError::InvalidBuildInfoError(_)
        ));
    }
}
