use resym_core::pdb_types::AccessSpecifierReconstructionFlavor;
use resym_core::pdb_types::IndentationFlavor;
use resym_core::pdb_types::PrimitiveReconstructionFlavor;
use resym_core::pdb_types::SizePrintFlavor;
use resym_core::pdb_types::VftableCommentFlavor;
use serde::{Deserialize, Serialize};

/// This struct represents the persistent settings of the application.
#[derive(Serialize, Deserialize)]
// Fields missing from older saved settings take their `Default` value
#[serde(default)]
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
    // Print fields and methods in PDB declaration order
    pub preserve_member_order: bool,
    // Wrap types in `namespace` blocks instead of using qualified names
    pub reconstruct_namespaces: bool,
    // Print `override` on virtual methods overriding a base class method
    pub print_override_specifiers: bool,
    // Print parameter names of methods when the PDB has them
    pub print_argument_names: bool,
    // Print the RVA of methods implemented in the PDB
    pub print_method_rvas: bool,
    // Print the calling convention of methods
    pub print_calling_conventions: bool,
    #[serde(with = "VftableCommentFlavorDef")]
    pub vftable_comment_flavor: VftableCommentFlavor,
    // Don't print the `/* offset: fields for Base */` comments
    pub hide_base_class_offsets: bool,
    // Don't print `virtual` on methods printed with `override`
    pub omit_virtual_on_overrides: bool,
    // Prefix type names with their keyword (e.g., `struct IUnknown*`)
    pub print_elaborated_type_specifiers: bool,
    #[serde(with = "IndentationFlavorDef")]
    pub indentation: IndentationFlavor,
    // Don't separate groups of members with blank lines
    pub compact_output: bool,
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
            preserve_member_order: true,
            reconstruct_namespaces: true,
            print_override_specifiers: true,
            print_argument_names: true,
            print_method_rvas: true,
            print_calling_conventions: false,
            vftable_comment_flavor: VftableCommentFlavor::Offset,
            hide_base_class_offsets: false,
            omit_virtual_on_overrides: false,
            print_elaborated_type_specifiers: false,
            indentation: IndentationFlavor::TwoSpaces,
            compact_output: false,
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

// Definition of the remote enum so that serde can its traits
#[derive(Serialize, Deserialize)]
#[serde(remote = "VftableCommentFlavor")]
enum VftableCommentFlavorDef {
    Disabled,
    Offset,
    Index,
}

// Definition of the remote enum so that serde can its traits
#[derive(Serialize, Deserialize)]
#[serde(remote = "IndentationFlavor")]
enum IndentationFlavorDef {
    TwoSpaces,
    FourSpaces,
    Tab,
}
