use std::path::Path;

use resym_core::{
    pdb_file::PdbFile, pdb_types::AccessSpecifierReconstructionFlavor,
    pdb_types::PrimitiveReconstructionFlavor, pdb_types::SizePrintFlavor,
};

const TEST_PDB_FILE_PATH: &str = "tests/data/test.pdb";
const TEST_CASES: &[&str] = &[
    "resym_test::PrimitiveTypesTest",
    "resym_test::ArrayTest",
    "resym_test::BitFieldsTest1",
    "resym_test::BitFieldsTest2",
    "resym_test::UnionTest",
    "resym_test::StructTest",
    "resym_test::EnumTest1",
    "resym_test::EnumTest2",
    "resym_test::StructUnnamedUdtTest1",
    "resym_test::StructUnnamedUdtTest2",
    "resym_test::StructUnnamedUdtTest3",
    "resym_test::UnionUnnamedUdtTest1",
    "resym_test::PureVirtualClassSpecialized",
    "resym_test::InterfaceImplClass",
    "resym_test::SpecializedInterfaceImplClass",
    "resym_test::ClassWithRefsAndStaticsTest",
    "resym_test::BigOffsetsStruct",
    "resym_test::BitFieldsTest3",
    "resym_test::BitFieldsTest4",
    "resym_test::BitFieldsTest5",
    "resym_test::BitFieldsTest6",
    "resym_test::BitFieldsTest7",
    "resym_test::NestedStructUnionRegression1",
    "resym_test::NtdllRegression1",
];

#[test]
fn test_type_reconstruction_portable_access_specifiers() {
    test_type_reconstruction_internal(
        "type_reconstruction_portable_access_specifiers",
        PrimitiveReconstructionFlavor::Portable,
        AccessSpecifierReconstructionFlavor::Always,
        SizePrintFlavor::Comment,
        false,
        true,
        true,
        false,
        false,
    );
}

#[test]
fn test_type_reconstruction_microsoft_access_specifiers() {
    test_type_reconstruction_internal(
        "type_reconstruction_microsoft_access_specifiers",
        PrimitiveReconstructionFlavor::Microsoft,
        AccessSpecifierReconstructionFlavor::Always,
        SizePrintFlavor::Comment,
        false,
        true,
        true,
        false,
        false,
    );
}

#[test]
fn test_type_reconstruction_raw_access_specifiers() {
    test_type_reconstruction_internal(
        "type_reconstruction_raw_access_specifiers",
        PrimitiveReconstructionFlavor::Raw,
        AccessSpecifierReconstructionFlavor::Always,
        SizePrintFlavor::Comment,
        false,
        true,
        true,
        false,
        false,
    );
}

#[test]
fn test_type_reconstruction_msvc_access_specifiers() {
    test_type_reconstruction_internal(
        "type_reconstruction_msvc_access_specifiers",
        PrimitiveReconstructionFlavor::Msvc,
        AccessSpecifierReconstructionFlavor::Always,
        SizePrintFlavor::Comment,
        false,
        true,
        true,
        false,
        false,
    );
}

#[test]
fn test_type_reconstruction_automatic_access_specifiers() {
    test_type_reconstruction_internal(
        "type_reconstruction_automatic_access_specifiers",
        PrimitiveReconstructionFlavor::Portable,
        AccessSpecifierReconstructionFlavor::Automatic,
        SizePrintFlavor::Comment,
        false,
        true,
        true,
        false,
        false,
    );
}

#[test]
fn test_type_reconstruction_no_size_info() {
    test_type_reconstruction_internal(
        "test_type_reconstruction_no_size_info",
        PrimitiveReconstructionFlavor::Portable,
        AccessSpecifierReconstructionFlavor::Automatic,
        SizePrintFlavor::Disabled,
        false,
        true,
        true,
        false,
        false,
    );
}

#[test]
fn test_type_reconstruction_no_offset_info() {
    test_type_reconstruction_internal(
        "test_type_reconstruction_no_offset_info",
        PrimitiveReconstructionFlavor::Portable,
        AccessSpecifierReconstructionFlavor::Automatic,
        SizePrintFlavor::Comment,
        false,
        true,
        false,
        false,
        false,
    );
}

#[test]
fn test_type_reconstruction_no_comments() {
    test_type_reconstruction_internal(
        "test_type_reconstruction_no_comments",
        PrimitiveReconstructionFlavor::Portable,
        AccessSpecifierReconstructionFlavor::Automatic,
        SizePrintFlavor::Disabled,
        false,
        true,
        false,
        false,
        false,
    );
}

fn test_type_reconstruction_internal(
    test_name: &str,
    primitives_flavor: PrimitiveReconstructionFlavor,
    print_access_specifiers: AccessSpecifierReconstructionFlavor,
    print_size_info: SizePrintFlavor,
    reconstruct_dependencies: bool,
    integers_as_hexadecimal: bool,
    print_offset_info: bool,
    print_brackets_new_line: bool,
    ignore_std_types: bool,
) {
    let pdb_file = PdbFile::load_from_file(Path::new(TEST_PDB_FILE_PATH)).expect("load test.pdb");
    for (i, test_case_type_name) in TEST_CASES.iter().enumerate() {
        let (reconstructed_type, _) = pdb_file
            .reconstruct_type_by_name(
                test_case_type_name,
                primitives_flavor,
                print_access_specifiers,
                print_size_info,
                reconstruct_dependencies,
                integers_as_hexadecimal,
                print_offset_info,
                print_brackets_new_line,
                ignore_std_types,
                false, // ignore_compiler_generated_methods
                false, // preserve_member_order
            )
            .unwrap_or_else(|_| panic!("reconstruct type: {test_case_type_name}"));

        let snapshot_name = format!("{test_name}-{i}");
        insta::assert_snapshot!(snapshot_name, reconstructed_type);
    }
}
