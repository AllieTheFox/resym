use std::path::Path;

use resym_core::{
    diffing::diff_type_by_name,
    pdb_file::PdbFile,
    pdb_types::{
        AccessSpecifierReconstructionFlavor, PrimitiveReconstructionFlavor, SizePrintFlavor,
    },
};

const TEST_PDB_FROM_FILE_PATH: &str = "tests/data/test_diff_from.pdb";
const TEST_PDB_TO_FILE_PATH: &str = "tests/data/test_diff_to.pdb";
const TEST_CASES: &[&str] = &[
    "UserStructAddAndReplace",
    "UserStructRemove",
    "UserStructAdd",
    "RemovedStruct",
    "NewStruct",
];

#[test]
fn test_struct_diffing() {
    let pdb_file_from = PdbFile::load_from_file(Path::new(TEST_PDB_FROM_FILE_PATH))
        .expect("load test_diff_from.pdb");
    let pdb_file_to =
        PdbFile::load_from_file(Path::new(TEST_PDB_TO_FILE_PATH)).expect("load test_diff_to.pdb");

    for test_case_type_name in TEST_CASES {
        let diffed_type = diff_type_by_name(
            &pdb_file_from,
            &pdb_file_to,
            test_case_type_name,
            PrimitiveReconstructionFlavor::Portable,
            AccessSpecifierReconstructionFlavor::Disabled,
            SizePrintFlavor::Comment,
            false, // print_header
            false, // reconstruct_dependencies
            false, // integers_as_hexadecimal
            true,  // print_offset_info
            false, // print_brackets_new_line
            false, // ignore_std_types
            false, // ignore_compiler_generated_methods
            false, // preserve_member_order
            false, // reconstruct_namespaces
            false, // print_override_specifiers
            false, // print_argument_names
        )
        .expect("diff generation");
        insta::assert_snapshot!(diffed_type.data);
    }
}

#[test]
fn test_struct_diffing_inexistent_type() {
    const INEXISTENT_TYPE_NAME: &str = "TypeNotFound";
    let pdb_file_from = PdbFile::load_from_file(Path::new(TEST_PDB_FROM_FILE_PATH))
        .expect("load test_diff_from.pdb");
    let pdb_file_to =
        PdbFile::load_from_file(Path::new(TEST_PDB_TO_FILE_PATH)).expect("load test_diff_to.pdb");
    assert!(diff_type_by_name(
        &pdb_file_from,
        &pdb_file_to,
        INEXISTENT_TYPE_NAME,
        PrimitiveReconstructionFlavor::Portable,
        AccessSpecifierReconstructionFlavor::Always,
        SizePrintFlavor::Comment,
        false, // print_header
        false, // reconstruct_dependencies
        false, // integers_as_hexadecimal
        true,  // print_offset_info
        false, // print_brackets_new_line
        false, // ignore_std_types
        false, // ignore_compiler_generated_methods
        false, // preserve_member_order
        false, // reconstruct_namespaces
        false, // print_override_specifiers
        false, // print_argument_names
    )
    .is_err());
}
