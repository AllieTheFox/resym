use resym_core::pdb_types::AccessSpecifierReconstructionFlavor;
use resym_core::pdb_types::PrimitiveReconstructionFlavor;
use resym_core::pdb_types::SizePrintFlavor;
use serde::{Deserialize, Serialize};

/// This struct represents the persistent settings of the application.
#[derive(Serialize, Deserialize)]
pub struct ResymAppSettings {
    pub use_light_theme: bool,
    pub font_size: u16,
    pub search_case_insensitive: bool,
    pub search_use_regex: bool,
    pub enable_syntax_hightlighting: bool,
    pub integers_as_hexadecimal: bool,
    pub print_offset_info: bool,
    pub print_brackets_new_line: bool,
    #[serde(with = "PrimitiveReconstructionFlavorDef")]
    pub primitive_types_flavor: PrimitiveReconstructionFlavor,
    #[serde(with = "AccessSpecifierReconstructionFlavorDef")]
    pub print_access_specifiers: AccessSpecifierReconstructionFlavor,
    #[serde(with = "SizePrintFlavorDef")]
    pub size_print_flavor: SizePrintFlavor,
    pub print_header: bool,
    pub reconstruct_dependencies: bool,
    // Ignore types in the `std` namespace (e.g., STL-generated types)
    pub ignore_std_types: bool,
    pub print_line_numbers: bool,
    pub ignore_compiler_generated_methods: bool,
}

impl Default for ResymAppSettings {
    fn default() -> Self {
        Self {
            use_light_theme: false,
            font_size: 14,
            search_case_insensitive: true,
            search_use_regex: false,
            enable_syntax_hightlighting: true,
            integers_as_hexadecimal: true,
            print_offset_info: true,
            print_brackets_new_line: false,
            primitive_types_flavor: PrimitiveReconstructionFlavor::Portable,
            print_access_specifiers: AccessSpecifierReconstructionFlavor::Always,
            size_print_flavor: SizePrintFlavor::Comment,
            print_header: true,
            reconstruct_dependencies: true,
            ignore_std_types: true,
            print_line_numbers: false,
            ignore_compiler_generated_methods: true,
        }
    }
}

// Definition of the remote enum so that serde can its traits
#[derive(Serialize, Deserialize)]
#[serde(remote = "PrimitiveReconstructionFlavor")]
enum PrimitiveReconstructionFlavorDef {
    Portable,
    Microsoft,
    Raw,
    Msvc,
}

// Definition of the remote enum so that serde can its traits
#[derive(Serialize, Deserialize)]
#[serde(remote = "AccessSpecifierReconstructionFlavor")]
enum AccessSpecifierReconstructionFlavorDef {
    Disabled,
    Always,
    Automatic,
}

// Definition of the remote enum so that serde can its traits
#[derive(Serialize, Deserialize)]
#[serde(remote = "SizePrintFlavor")]
enum SizePrintFlavorDef {
    Disabled,
    Comment,
    StaticAssert,
}
