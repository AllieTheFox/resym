#[cfg(target_arch = "wasm32")]
use instant::Instant;
use similar::{ChangeTag, TextDiff};

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
use std::{fmt::Write, io};

use crate::{
    error::{Result, ResymCoreError},
    pdb_file::PdbFile,
    pdb_types::{
        AccessSpecifierReconstructionFlavor, IndentationFlavor, PrimitiveReconstructionFlavor,
        SizePrintFlavor, VftableCommentFlavor,
    },
    PKG_VERSION,
};

pub type DiffChange = ChangeTag;
pub type DiffIndices = (Option<usize>, Option<usize>);

#[derive(Default)]
pub struct Diff {
    pub metadata: Vec<(DiffIndices, DiffChange)>,
    pub data: String,
}
pub struct DiffLine {
    pub indices: DiffIndices,
    pub change: DiffChange,
    pub line: String,
}

#[allow(clippy::too_many_arguments)]
pub fn diff_type_by_name<'p, T>(
    pdb_file_from: &PdbFile<'p, T>,
    pdb_file_to: &PdbFile<'p, T>,
    type_name: &str,
    primitives_flavor: PrimitiveReconstructionFlavor,
    use_typedef_names: bool,
    print_access_specifiers: AccessSpecifierReconstructionFlavor,
    size_print_flavor: SizePrintFlavor,
    print_header: bool,
    reconstruct_dependencies: bool,
    integers_as_hexadecimal: bool,
    print_offset_info: bool,
    print_brackets_new_line: bool,
    ignore_std_types: bool,
    ignore_compiler_generated_methods: bool,
    preserve_member_order: bool,
    reconstruct_namespaces: bool,
    unify_namespaces: bool,
    indent_namespaces: bool,
    group_namespaces: bool,
    print_override_specifiers: bool,
    print_argument_names: bool,
    print_method_rvas: bool,
    print_calling_conventions: bool,
    vftable_comment_flavor: VftableCommentFlavor,
    hide_base_class_offsets: bool,
    omit_virtual_on_overrides: bool,
    print_elaborated_type_specifiers: bool,
    indentation: IndentationFlavor,
    compact_output: bool,
    print_source_locations: bool,
) -> Result<Diff>
where
    T: io::Seek + io::Read + std::fmt::Debug + 'p,
{
    let diff_start = Instant::now();

    // Prepend header if needed
    let (mut reconstructed_type_from, mut reconstructed_type_to) = if print_header {
        let diff_header = generate_diff_header(pdb_file_from, pdb_file_to);
        (diff_header.clone(), diff_header)
    } else {
        (String::default(), String::default())
    };

    // Reconstruct types from both PDBs
    {
        let (reconstructed_type_from_tmp, _) = pdb_file_from
            .reconstruct_type_by_name(
                type_name,
                primitives_flavor,
                use_typedef_names,
                print_access_specifiers,
                size_print_flavor,
                reconstruct_dependencies,
                integers_as_hexadecimal,
                print_offset_info,
                print_brackets_new_line,
                ignore_std_types,
                ignore_compiler_generated_methods,
                preserve_member_order,
                reconstruct_namespaces,
                unify_namespaces,
                indent_namespaces,
                group_namespaces,
                print_override_specifiers,
                print_argument_names,
                print_method_rvas,
                print_calling_conventions,
                vftable_comment_flavor,
                hide_base_class_offsets,
                omit_virtual_on_overrides,
                print_elaborated_type_specifiers,
                indentation,
                compact_output,
                print_source_locations,
            )
            .unwrap_or_default();
        let (reconstructed_type_to_tmp, _) = pdb_file_to
            .reconstruct_type_by_name(
                type_name,
                primitives_flavor,
                use_typedef_names,
                print_access_specifiers,
                size_print_flavor,
                reconstruct_dependencies,
                integers_as_hexadecimal,
                print_offset_info,
                print_brackets_new_line,
                ignore_std_types,
                ignore_compiler_generated_methods,
                preserve_member_order,
                reconstruct_namespaces,
                unify_namespaces,
                indent_namespaces,
                group_namespaces,
                print_override_specifiers,
                print_argument_names,
                print_method_rvas,
                print_calling_conventions,
                vftable_comment_flavor,
                hide_base_class_offsets,
                omit_virtual_on_overrides,
                print_elaborated_type_specifiers,
                indentation,
                compact_output,
                print_source_locations,
            )
            .unwrap_or_default();
        if reconstructed_type_from_tmp.is_empty() && reconstructed_type_to_tmp.is_empty() {
            // Make it obvious an error occured
            return Err(ResymCoreError::TypeNameNotFoundError(type_name.to_owned()));
        }
        reconstructed_type_from.push_str(&reconstructed_type_from_tmp);
        reconstructed_type_to.push_str(&reconstructed_type_to_tmp);
    }

    // Diff reconstructed representations
    let diff = generate_diff(&reconstructed_type_from, &reconstructed_type_to)?;
    log::debug!("Type diffing took {} ms", diff_start.elapsed().as_millis());

    Ok(diff)
}

#[allow(clippy::too_many_arguments)]
pub fn diff_module_by_path<'p, T>(
    pdb_file_from: &PdbFile<'p, T>,
    pdb_file_to: &PdbFile<'p, T>,
    module_path: &str,
    primitives_flavor: PrimitiveReconstructionFlavor,
    use_typedef_names: bool,
    print_access_specifiers: AccessSpecifierReconstructionFlavor,
    print_header: bool,
    print_local_variables: bool,
    print_local_variable_locations: bool,
    print_inlined_call_sites: bool,
    indentation: IndentationFlavor,
    print_source_locations: bool,
) -> Result<Diff>
where
    T: io::Seek + io::Read + std::fmt::Debug + 'p,
{
    let diff_start = Instant::now();

    // Prepend header if needed
    let (mut reconstructed_module_from, mut reconstructed_module_to) = if print_header {
        let mut diff_header = generate_diff_header(pdb_file_from, pdb_file_to);
        diff_header.push('\n');

        (diff_header.clone(), diff_header)
    } else {
        (String::default(), String::default())
    };

    // Reconstruct modules from both PDBs
    {
        let reconstruct = |pdb_file: &PdbFile<'p, T>| {
            pdb_file
                .reconstruct_module_by_path(
                    module_path,
                    primitives_flavor,
                    use_typedef_names,
                    print_access_specifiers,
                    print_local_variables,
                    print_local_variable_locations,
                    print_inlined_call_sites,
                    indentation,
                    print_source_locations,
                )
                .unwrap_or_default()
        };
        let reconstructed_type_from_tmp = reconstruct(pdb_file_from);
        let reconstructed_type_to_tmp = reconstruct(pdb_file_to);
        if reconstructed_type_from_tmp.is_empty() && reconstructed_type_to_tmp.is_empty() {
            // Make it obvious an error occured
            return Err(ResymCoreError::ModuleNotFoundError(module_path.to_owned()));
        }
        reconstructed_module_from.push_str(&reconstructed_type_from_tmp);
        reconstructed_module_to.push_str(&reconstructed_type_to_tmp);
    }

    // Diff reconstructed representations
    let diff = generate_diff(&reconstructed_module_from, &reconstructed_module_to)?;
    log::debug!(
        "Module diffing took {} ms",
        diff_start.elapsed().as_millis()
    );

    Ok(diff)
}

#[allow(clippy::too_many_arguments)]
pub fn diff_source_file_by_path<'p, T>(
    pdb_file_from: &PdbFile<'p, T>,
    pdb_file_to: &PdbFile<'p, T>,
    source_file_path: &str,
    primitives_flavor: PrimitiveReconstructionFlavor,
    use_typedef_names: bool,
    print_access_specifiers: AccessSpecifierReconstructionFlavor,
    print_header: bool,
    size_print_flavor: SizePrintFlavor,
    integers_as_hexadecimal: bool,
    print_offset_info: bool,
    print_brackets_new_line: bool,
    ignore_std_types: bool,
    ignore_compiler_generated_methods: bool,
    preserve_member_order: bool,
    reconstruct_namespaces: bool,
    unify_namespaces: bool,
    indent_namespaces: bool,
    print_override_specifiers: bool,
    print_argument_names: bool,
    print_method_rvas: bool,
    print_calling_conventions: bool,
    vftable_comment_flavor: VftableCommentFlavor,
    hide_base_class_offsets: bool,
    omit_virtual_on_overrides: bool,
    print_elaborated_type_specifiers: bool,
    indentation: IndentationFlavor,
    compact_output: bool,
    print_source_locations: bool,
    print_local_variables: bool,
    print_local_variable_locations: bool,
    print_inlined_call_sites: bool,
) -> Result<Diff>
where
    T: io::Seek + io::Read + std::fmt::Debug + 'p,
{
    let diff_start = Instant::now();

    // Prepend header if needed
    let (mut reconstructed_file_from, mut reconstructed_file_to) = if print_header {
        let mut diff_header = generate_diff_header(pdb_file_from, pdb_file_to);
        diff_header.push('\n');

        (diff_header.clone(), diff_header)
    } else {
        (String::default(), String::default())
    };

    // Reconstruct the source file from both PDBs
    {
        let reconstruct = |pdb_file: &PdbFile<'p, T>| {
            pdb_file
                .reconstruct_source_file(
                    source_file_path,
                    primitives_flavor,
                    use_typedef_names,
                    print_access_specifiers,
                    size_print_flavor,
                    integers_as_hexadecimal,
                    print_offset_info,
                    print_brackets_new_line,
                    ignore_std_types,
                    ignore_compiler_generated_methods,
                    preserve_member_order,
                    reconstruct_namespaces,
                    unify_namespaces,
                    indent_namespaces,
                    print_override_specifiers,
                    print_argument_names,
                    print_method_rvas,
                    print_calling_conventions,
                    vftable_comment_flavor,
                    hide_base_class_offsets,
                    omit_virtual_on_overrides,
                    print_elaborated_type_specifiers,
                    indentation,
                    compact_output,
                    print_source_locations,
                    print_local_variables,
                    print_local_variable_locations,
                    print_inlined_call_sites,
                )
                .unwrap_or_default()
        };
        let reconstructed_file_from_tmp = reconstruct(pdb_file_from);
        let reconstructed_file_to_tmp = reconstruct(pdb_file_to);
        if reconstructed_file_from_tmp.is_empty() && reconstructed_file_to_tmp.is_empty() {
            // Make it obvious an error occured
            return Err(ResymCoreError::SourceFileNotFoundError(
                source_file_path.to_owned(),
            ));
        }
        reconstructed_file_from.push_str(&reconstructed_file_from_tmp);
        reconstructed_file_to.push_str(&reconstructed_file_to_tmp);
    }

    // Diff reconstructed representations
    let diff = generate_diff(&reconstructed_file_from, &reconstructed_file_to)?;
    log::debug!(
        "Source file diffing took {} ms",
        diff_start.elapsed().as_millis()
    );

    Ok(diff)
}

#[allow(clippy::too_many_arguments)]
pub fn diff_symbol_by_name<'p, T>(
    pdb_file_from: &PdbFile<'p, T>,
    pdb_file_to: &PdbFile<'p, T>,
    symbol_name: &str,
    primitives_flavor: PrimitiveReconstructionFlavor,
    use_typedef_names: bool,
    print_access_specifiers: AccessSpecifierReconstructionFlavor,
    print_header: bool,
    print_local_variables: bool,
    print_local_variable_locations: bool,
    print_inlined_call_sites: bool,
    size_print_flavor: SizePrintFlavor,
    reconstruct_dependencies: bool,
    integers_as_hexadecimal: bool,
    print_offset_info: bool,
    print_brackets_new_line: bool,
    ignore_std_types: bool,
    ignore_compiler_generated_methods: bool,
    preserve_member_order: bool,
    reconstruct_namespaces: bool,
    unify_namespaces: bool,
    indent_namespaces: bool,
    group_namespaces: bool,
    print_override_specifiers: bool,
    print_argument_names: bool,
    print_method_rvas: bool,
    print_calling_conventions: bool,
    vftable_comment_flavor: VftableCommentFlavor,
    hide_base_class_offsets: bool,
    omit_virtual_on_overrides: bool,
    print_elaborated_type_specifiers: bool,
    indentation: IndentationFlavor,
    compact_output: bool,
    print_source_locations: bool,
) -> Result<Diff>
where
    T: io::Seek + io::Read + std::fmt::Debug + 'p,
{
    let diff_start = Instant::now();

    // Prepend header if needed
    let (mut reconstructed_symbol_from, mut reconstructed_symbol_to) = if print_header {
        let diff_header = generate_diff_header(pdb_file_from, pdb_file_to);
        (diff_header.clone(), diff_header)
    } else {
        (String::default(), String::default())
    };

    // Reconstruct modules from both PDBs
    {
        let reconstruct = |pdb_file: &PdbFile<'p, T>| {
            pdb_file
                .reconstruct_symbol_by_name(
                    symbol_name,
                    primitives_flavor,
                    use_typedef_names,
                    print_access_specifiers,
                    print_local_variables,
                    print_local_variable_locations,
                    print_inlined_call_sites,
                    reconstruct_dependencies,
                    size_print_flavor,
                    integers_as_hexadecimal,
                    print_offset_info,
                    print_brackets_new_line,
                    ignore_std_types,
                    ignore_compiler_generated_methods,
                    preserve_member_order,
                    reconstruct_namespaces,
                    unify_namespaces,
                    indent_namespaces,
                    group_namespaces,
                    print_override_specifiers,
                    print_argument_names,
                    print_method_rvas,
                    print_calling_conventions,
                    vftable_comment_flavor,
                    hide_base_class_offsets,
                    omit_virtual_on_overrides,
                    print_elaborated_type_specifiers,
                    indentation,
                    compact_output,
                    print_source_locations,
                )
                .unwrap_or_default()
        };
        let reconstructed_symbol_from_tmp = reconstruct(pdb_file_from);
        let reconstructed_symbol_to_tmp = reconstruct(pdb_file_to);
        if reconstructed_symbol_from_tmp.is_empty() && reconstructed_symbol_to_tmp.is_empty() {
            // Make it obvious an error occured
            return Err(ResymCoreError::SymbolNotFoundError(symbol_name.to_owned()));
        }
        reconstructed_symbol_from.push_str(&reconstructed_symbol_from_tmp);
        reconstructed_symbol_to.push_str(&reconstructed_symbol_to_tmp);
    }

    // Diff reconstructed representations
    let diff = generate_diff(&reconstructed_symbol_from, &reconstructed_symbol_to)?;
    log::debug!(
        "Symbol diffing took {} ms",
        diff_start.elapsed().as_millis()
    );

    Ok(diff)
}

fn generate_diff_header<'p, T>(
    pdb_file_from: &PdbFile<'p, T>,
    pdb_file_to: &PdbFile<'p, T>,
) -> String
where
    T: io::Seek + io::Read + 'p,
{
    format!(
        concat!(
            "//\n",
            "// Showing differences between two PDB files:\n",
            "//\n",
            "// Reference PDB file: {}\n",
            "// Image architecture: {}\n",
            "//\n",
            "// New PDB file: {}\n",
            "// Image architecture: {}\n",
            "//\n",
            "// Information extracted with resym v{}\n",
            "//\n"
        ),
        pdb_file_from.file_path.display(),
        pdb_file_from.machine_type,
        pdb_file_to.file_path.display(),
        pdb_file_to.machine_type,
        PKG_VERSION,
    )
}

fn generate_diff(str_from: &str, str_to: &str) -> Result<Diff> {
    let mut diff_metadata = vec![];
    let mut diff_data = String::default();
    {
        let reconstructed_type_diff = TextDiff::from_lines(str_from, str_to);
        for change in reconstructed_type_diff.iter_all_changes() {
            diff_metadata.push(((change.old_index(), change.new_index()), change.tag()));
            let prefix = match change.tag() {
                ChangeTag::Insert => "+",
                ChangeTag::Delete => "-",
                ChangeTag::Equal => " ",
            };
            write!(&mut diff_data, "{prefix}{change}")?;
        }
    }

    Ok(Diff {
        metadata: diff_metadata,
        data: diff_data,
    })
}
