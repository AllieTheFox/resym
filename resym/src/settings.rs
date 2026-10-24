use resym_core::pdb_types::AccessSpecifierReconstructionFlavor;
use resym_core::pdb_types::IndentationFlavor;
use resym_core::pdb_types::PrimitiveReconstructionFlavor;
use resym_core::pdb_types::SizePrintFlavor;
use resym_core::pdb_types::VftableCommentFlavor;
use serde::{Deserialize, Serialize};

/// This struct represents the persistent settings of the application.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
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
    // Keep namespace blocks open across types and nest inner namespaces
    pub unify_namespaces: bool,
    // Indent the contents of namespace blocks
    pub indent_namespaces: bool,
    // With dependencies, write the types of a namespace together when possible
    pub group_namespaces: bool,
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
    // Write tags and handles with their typedef name (e.g., `GUID`, `HWND`)
    pub use_typedef_names: bool,
    #[serde(with = "IndentationFlavorDef")]
    pub indentation: IndentationFlavor,
    // Don't separate groups of members with blank lines
    pub compact_output: bool,
    // Print where types and functions are defined (source file and line),
    // when known
    pub print_source_locations: bool,
    // Print the local variables of functions in symbol and module views
    pub print_local_variables: bool,
    // Print where local variables are stored (registers, stack offsets)
    pub print_local_variable_locations: bool,
    // Print the local variables of inlined call sites
    pub print_inlined_call_sites: bool,
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
            unify_namespaces: true,
            indent_namespaces: true,
            group_namespaces: true,
            print_override_specifiers: true,
            print_argument_names: true,
            print_method_rvas: true,
            print_calling_conventions: false,
            vftable_comment_flavor: VftableCommentFlavor::Offset,
            hide_base_class_offsets: false,
            omit_virtual_on_overrides: false,
            print_elaborated_type_specifiers: false,
            use_typedef_names: true,
            indentation: IndentationFlavor::TwoSpaces,
            compact_output: false,
            print_source_locations: true,
            print_local_variables: true,
            print_local_variable_locations: true,
            print_inlined_call_sites: false,
        }
    }
}

impl ResymAppSettings {
    /// Indicates if reconstructing with these settings can produce a different
    /// output than with `other`. Settings applied by the UI only (theme, font,
    /// search, syntax highlighting, line numbers) are ignored.
    pub fn affects_reconstruction_differently(&self, other: &Self) -> bool {
        let without_ui_settings = |settings: &Self| Self {
            use_light_theme: false,
            font_size: 0,
            search_case_insensitive: false,
            search_use_regex: false,
            enable_syntax_hightlighting: false,
            print_line_numbers: false,
            ..settings.clone()
        };
        without_ui_settings(self) != without_ui_settings(other)
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
