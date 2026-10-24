use std::path::PathBuf;

use resym_core::pdb_types::{
    AccessSpecifierReconstructionFlavor, IndentationFlavor, PrimitiveReconstructionFlavor,
    SizePrintFlavor, VftableCommentFlavor,
};
use structopt::StructOpt;

const PKG_NAME: &str = env!("CARGO_PKG_NAME");

#[derive(Debug, StructOpt)]
#[structopt(
    name = PKG_NAME,
    about = "resymc is a utility that allows browsing and extracting types from PDB files."
)]
pub enum ResymcOptions {
    /// List types from a given PDB file
    List {
        /// Path to the PDB file
        pdb_path: PathBuf,
        /// Search filter
        type_name_filter: String,
        /// Path of the output file
        output_file_path: Option<PathBuf>,
        /// Do not match case
        #[structopt(short = "i", long)]
        case_insensitive: bool,
        /// Use regular expressions
        #[structopt(short = "r", long)]
        use_regex: bool,
        /// Filter out types in the `std` namespace
        #[structopt(short = "s", long)]
        ignore_std_types: bool,
    },
    /// Dump type from a given PDB file
    Dump {
        /// Path to the PDB file
        pdb_path: PathBuf,
        /// Name of the type to extract
        type_name: String,
        /// Path of the output file
        output_file_path: Option<PathBuf>,
        /// Representation of primitive types
        #[structopt(short = "f", long)]
        primitive_types_flavor: Option<PrimitiveReconstructionFlavor>,
        /// Write tags and handles with their typedef name (e.g., `GUID`, `HWND`)
        #[structopt(long)]
        use_typedef_names: bool,
        /// Print header
        #[structopt(short = "h", long)]
        print_header: bool,
        /// Print declarations of referenced types
        #[structopt(short = "d", long)]
        print_dependencies: bool,
        /// Print C++ access specifiers
        #[structopt(short = "a", long)]
        print_access_specifiers: Option<AccessSpecifierReconstructionFlavor>,
        /// Use hexadecimal formatting
        #[structopt(short = "x", long)]
        integers_as_hexadecimal: bool,
        /// Print Size Info
        #[structopt(short = "psi", long)]
        size_print_flavor: Option<SizePrintFlavor>,
        /// Print Offset Info
        #[structopt(short = "poi", long)]
        print_offset_info: bool,
        /// Print Brackets On New Line
        #[structopt(short = "pnl", long)]
        print_brackets_new_line: bool,
        /// Filter out types in the `std` namespace
        #[structopt(short = "s", long)]
        ignore_std_types: bool,
        /// Ignore compiler generated methods
        #[structopt(short = "g", long)]
        ignore_compiler_generated_methods: bool,
        /// Print fields and methods in PDB declaration order
        #[structopt(short = "m", long)]
        preserve_member_order: bool,
        /// Wrap types in `namespace` blocks instead of using qualified names
        #[structopt(short = "n", long)]
        reconstruct_namespaces: bool,
        /// Keep namespace blocks open across types and nest inner namespaces
        #[structopt(long)]
        unify_namespaces: bool,
        /// Indent the contents of namespace blocks
        #[structopt(long)]
        indent_namespaces: bool,
        /// With dependencies, write the types of a namespace together when possible
        #[structopt(long)]
        group_namespaces: bool,
        /// Print `override` on virtual methods overriding a base class method
        #[structopt(short = "o", long)]
        print_override_specifiers: bool,
        /// Print parameter names of methods when the PDB has them
        #[structopt(short = "A", long)]
        print_argument_names: bool,
        /// Print the RVA of methods implemented in the PDB
        #[structopt(long)]
        print_method_rvas: bool,
        /// Print the calling convention of methods
        #[structopt(long)]
        print_calling_conventions: bool,
        /// Vftable entry comments: offset, index or disabled
        #[structopt(long)]
        vftable_comment_flavor: Option<VftableCommentFlavor>,
        /// Don't print the base class offset comments
        #[structopt(long)]
        hide_base_class_offsets: bool,
        /// Don't print `virtual` on methods printed with `override`
        #[structopt(long)]
        omit_virtual_on_overrides: bool,
        /// Prefix type names with their keyword (e.g., `struct IUnknown*`)
        #[structopt(long)]
        print_elaborated_type_specifiers: bool,
        /// Indentation: 2, 4 or tab
        #[structopt(long)]
        indentation: Option<IndentationFlavor>,
        /// Don't separate groups of members with blank lines
        #[structopt(long)]
        compact_output: bool,
        /// Print where types are defined (source file and line), when known
        #[structopt(long)]
        print_source_locations: bool,
        /// Highlight C++ output
        #[structopt(short = "H", long)]
        highlight_syntax: bool,
    },
    /// Dump all types from a given PDB file
    DumpAll {
        /// Path to the PDB file
        pdb_path: PathBuf,
        /// Path of the output file
        output_file_path: Option<PathBuf>,
        /// Representation of primitive types
        #[structopt(short = "f", long)]
        primitive_types_flavor: Option<PrimitiveReconstructionFlavor>,
        /// Write tags and handles with their typedef name (e.g., `GUID`, `HWND`)
        #[structopt(long)]
        use_typedef_names: bool,
        /// Print header
        #[structopt(short = "h", long)]
        print_header: bool,
        /// Print C++ access specifiers
        #[structopt(short = "a", long)]
        print_access_specifiers: Option<AccessSpecifierReconstructionFlavor>,
        /// Use hexadecimal formatting
        #[structopt(short = "x", long)]
        integers_as_hexadecimal: bool,
        /// Print Size Info
        #[structopt(short = "psi", long)]
        size_print_flavor: Option<SizePrintFlavor>,
        /// Print Offset Info
        #[structopt(short = "poi", long)]
        print_offset_info: bool,
        /// Print Brackets On New Line
        #[structopt(short = "pnl", long)]
        print_brackets_new_line: bool,
        /// Filter out types in the `std` namespace
        #[structopt(short = "s", long)]
        ignore_std_types: bool,
        /// Ignore compiler generated methods
        #[structopt(short = "g", long)]
        ignore_compiler_generated_methods: bool,
        /// Print fields and methods in PDB declaration order
        #[structopt(short = "m", long)]
        preserve_member_order: bool,
        /// Wrap types in `namespace` blocks instead of using qualified names
        #[structopt(short = "n", long)]
        reconstruct_namespaces: bool,
        /// Keep namespace blocks open across types and nest inner namespaces
        #[structopt(long)]
        unify_namespaces: bool,
        /// Indent the contents of namespace blocks
        #[structopt(long)]
        indent_namespaces: bool,
        /// Write the types of a namespace together when possible
        #[structopt(long)]
        group_namespaces: bool,
        /// Print `override` on virtual methods overriding a base class method
        #[structopt(short = "o", long)]
        print_override_specifiers: bool,
        /// Print parameter names of methods when the PDB has them
        #[structopt(short = "A", long)]
        print_argument_names: bool,
        /// Print the RVA of methods implemented in the PDB
        #[structopt(long)]
        print_method_rvas: bool,
        /// Print the calling convention of methods
        #[structopt(long)]
        print_calling_conventions: bool,
        /// Vftable entry comments: offset, index or disabled
        #[structopt(long)]
        vftable_comment_flavor: Option<VftableCommentFlavor>,
        /// Don't print the base class offset comments
        #[structopt(long)]
        hide_base_class_offsets: bool,
        /// Don't print `virtual` on methods printed with `override`
        #[structopt(long)]
        omit_virtual_on_overrides: bool,
        /// Prefix type names with their keyword (e.g., `struct IUnknown*`)
        #[structopt(long)]
        print_elaborated_type_specifiers: bool,
        /// Indentation: 2, 4 or tab
        #[structopt(long)]
        indentation: Option<IndentationFlavor>,
        /// Don't separate groups of members with blank lines
        #[structopt(long)]
        compact_output: bool,
        /// Print where types are defined (source file and line), when known
        #[structopt(long)]
        print_source_locations: bool,
        /// Highlight C++ output
        #[structopt(short = "H", long)]
        highlight_syntax: bool,
    },
    /// Compute diff for a type between two given PDB files
    Diff {
        /// Path of the PDB file to compute the diff from
        from_pdb_path: PathBuf,
        /// Path of the PDB file to compute the diff to
        to_pdb_path: PathBuf,
        /// Name of the type to diff
        type_name: String,
        /// Path of the output file
        output_file_path: Option<PathBuf>,
        /// Representation of primitive types
        #[structopt(short = "f", long)]
        primitive_types_flavor: Option<PrimitiveReconstructionFlavor>,
        /// Write tags and handles with their typedef name (e.g., `GUID`, `HWND`)
        #[structopt(long)]
        use_typedef_names: bool,
        /// Print header
        #[structopt(short = "h", long)]
        print_header: bool,
        /// Print declarations of referenced types
        #[structopt(short = "d", long)]
        print_dependencies: bool,
        /// Print C++ access specifiers
        #[structopt(short = "a", long)]
        print_access_specifiers: Option<AccessSpecifierReconstructionFlavor>,
        /// Use hexadecimal formatting
        #[structopt(short = "x", long)]
        integers_as_hexadecimal: bool,
        /// Print Size Info
        #[structopt(short = "psi", long)]
        size_print_flavor: Option<SizePrintFlavor>,
        /// Print Offset Info
        #[structopt(short = "poi", long)]
        print_offset_info: bool,
        /// Print Brackets On New Line
        #[structopt(short = "pnl", long)]
        print_brackets_new_line: bool,
        /// Filter out types in the `std` namespace
        #[structopt(short = "s", long)]
        ignore_std_types: bool,
        /// Ignore compiler generated methods
        #[structopt(short = "g", long)]
        ignore_compiler_generated_methods: bool,
        /// Print fields and methods in PDB declaration order
        #[structopt(short = "m", long)]
        preserve_member_order: bool,
        /// Wrap types in `namespace` blocks instead of using qualified names
        #[structopt(short = "n", long)]
        reconstruct_namespaces: bool,
        /// Keep namespace blocks open across types and nest inner namespaces
        #[structopt(long)]
        unify_namespaces: bool,
        /// Indent the contents of namespace blocks
        #[structopt(long)]
        indent_namespaces: bool,
        /// With dependencies, write the types of a namespace together when possible
        #[structopt(long)]
        group_namespaces: bool,
        /// Print `override` on virtual methods overriding a base class method
        #[structopt(short = "o", long)]
        print_override_specifiers: bool,
        /// Print parameter names of methods when the PDB has them
        #[structopt(short = "A", long)]
        print_argument_names: bool,
        /// Print the RVA of methods implemented in the PDB
        #[structopt(long)]
        print_method_rvas: bool,
        /// Print the calling convention of methods
        #[structopt(long)]
        print_calling_conventions: bool,
        /// Vftable entry comments: offset, index or disabled
        #[structopt(long)]
        vftable_comment_flavor: Option<VftableCommentFlavor>,
        /// Don't print the base class offset comments
        #[structopt(long)]
        hide_base_class_offsets: bool,
        /// Don't print `virtual` on methods printed with `override`
        #[structopt(long)]
        omit_virtual_on_overrides: bool,
        /// Prefix type names with their keyword (e.g., `struct IUnknown*`)
        #[structopt(long)]
        print_elaborated_type_specifiers: bool,
        /// Indentation: 2, 4 or tab
        #[structopt(long)]
        indentation: Option<IndentationFlavor>,
        /// Don't separate groups of members with blank lines
        #[structopt(long)]
        compact_output: bool,
        /// Print where types are defined (source file and line), when known
        #[structopt(long)]
        print_source_locations: bool,
        /// Highlight C++ output and add/deleted lines
        #[structopt(short = "H", long)]
        highlight_syntax: bool,
    },
    /// List modules from a given PDB file
    ListModules {
        /// Path to the PDB file
        pdb_path: PathBuf,
        /// Search filter
        module_path_filter: String,
        /// Path of the output file
        output_file_path: Option<PathBuf>,
        /// Do not match case
        #[structopt(short = "i", long)]
        case_insensitive: bool,
        /// Use regular expressions
        #[structopt(short = "r", long)]
        use_regex: bool,
    },
    /// Dump module from a given PDB file
    DumpModule {
        /// Path to the PDB file
        pdb_path: PathBuf,
        /// ID of the module to dump
        module_id: usize,
        /// Path of the output file
        output_file_path: Option<PathBuf>,
        /// Representation of primitive types
        #[structopt(short = "f", long)]
        primitive_types_flavor: Option<PrimitiveReconstructionFlavor>,
        /// Write tags and handles with their typedef name (e.g., `GUID`, `HWND`)
        #[structopt(long)]
        use_typedef_names: bool,
        /// Print header
        #[structopt(short = "h", long)]
        print_header: bool,
        /// Print C++ access specifiers
        #[structopt(short = "a", long)]
        print_access_specifiers: Option<AccessSpecifierReconstructionFlavor>,
        /// Print the local variables of functions
        #[structopt(long)]
        print_local_variables: bool,
        /// Print where local variables are stored (registers, stack offsets)
        #[structopt(long)]
        print_local_variable_locations: bool,
        /// Print the local variables of inlined call sites
        #[structopt(long)]
        print_inlined_call_sites: bool,
        /// Indentation: 2, 4 or tab
        #[structopt(long)]
        indentation: Option<IndentationFlavor>,
        /// Print where functions are defined (source file and line), when known
        #[structopt(long)]
        print_source_locations: bool,
        /// Highlight C++ output
        #[structopt(short = "H", long)]
        highlight_syntax: bool,
    },
    /// Compute diff for a module between two given PDB files
    DiffModule {
        /// Path of the PDB file to compute the diff from
        from_pdb_path: PathBuf,
        /// Path of the PDB file to compute the diff to
        to_pdb_path: PathBuf,
        /// Path of the module to diff
        module_path: String,
        /// Path of the output file
        output_file_path: Option<PathBuf>,
        /// Representation of primitive types
        #[structopt(short = "f", long)]
        primitive_types_flavor: Option<PrimitiveReconstructionFlavor>,
        /// Write tags and handles with their typedef name (e.g., `GUID`, `HWND`)
        #[structopt(long)]
        use_typedef_names: bool,
        /// Print header
        #[structopt(short = "h", long)]
        print_header: bool,
        /// Print C++ access specifiers
        #[structopt(short = "a", long)]
        print_access_specifiers: Option<AccessSpecifierReconstructionFlavor>,
        /// Print the local variables of functions
        #[structopt(long)]
        print_local_variables: bool,
        /// Print where local variables are stored (registers, stack offsets)
        #[structopt(long)]
        print_local_variable_locations: bool,
        /// Print the local variables of inlined call sites
        #[structopt(long)]
        print_inlined_call_sites: bool,
        /// Indentation: 2, 4 or tab
        #[structopt(long)]
        indentation: Option<IndentationFlavor>,
        /// Print where functions are defined (source file and line), when known
        #[structopt(long)]
        print_source_locations: bool,
        /// Highlight C++ output and add/deleted lines
        #[structopt(short = "H", long)]
        highlight_syntax: bool,
    },
    /// List symbols from a given PDB file
    ListSymbols {
        /// Path to the PDB file
        pdb_path: PathBuf,
        /// Search filter
        symbol_name_filter: String,
        /// Path of the output file
        output_file_path: Option<PathBuf>,
        /// Do not match case
        #[structopt(short = "i", long)]
        case_insensitive: bool,
        /// Use regular expressions
        #[structopt(short = "r", long)]
        use_regex: bool,
        /// Filter out types in the `std` namespace
        #[structopt(short = "s", long)]
        ignore_std_types: bool,
    },
    /// Dump symbol from a given PDB file
    DumpSymbol {
        /// Path to the PDB file
        pdb_path: PathBuf,
        /// Name of the symbol to dump
        symbol_name: String,
        /// Path of the output file
        output_file_path: Option<PathBuf>,
        /// Representation of primitive types
        #[structopt(short = "f", long)]
        primitive_types_flavor: Option<PrimitiveReconstructionFlavor>,
        /// Write tags and handles with their typedef name (e.g., `GUID`, `HWND`)
        #[structopt(long)]
        use_typedef_names: bool,
        /// Print header
        #[structopt(short = "h", long)]
        print_header: bool,
        /// Print C++ access specifiers
        #[structopt(short = "a", long)]
        print_access_specifiers: Option<AccessSpecifierReconstructionFlavor>,
        /// Print the local variables of functions
        #[structopt(long)]
        print_local_variables: bool,
        /// Print where local variables are stored (registers, stack offsets)
        #[structopt(long)]
        print_local_variable_locations: bool,
        /// Print the local variables of inlined call sites
        #[structopt(long)]
        print_inlined_call_sites: bool,
        /// Print declarations of the types the symbol uses
        #[structopt(short = "d", long)]
        print_dependencies: bool,
        /// Indentation: 2, 4 or tab
        #[structopt(long)]
        indentation: Option<IndentationFlavor>,
        /// Print where functions and types are defined (source file and line),
        /// when known
        #[structopt(long)]
        print_source_locations: bool,
        /// Highlight C++ output
        #[structopt(short = "H", long)]
        highlight_syntax: bool,
    },
    /// Dump all symbols from a given PDB file
    DumpAllSymbols {
        /// Path to the PDB file
        pdb_path: PathBuf,
        /// Path of the output file
        output_file_path: Option<PathBuf>,
        /// Representation of primitive types
        #[structopt(short = "f", long)]
        primitive_types_flavor: Option<PrimitiveReconstructionFlavor>,
        /// Write tags and handles with their typedef name (e.g., `GUID`, `HWND`)
        #[structopt(long)]
        use_typedef_names: bool,
        /// Print header
        #[structopt(short = "h", long)]
        print_header: bool,
        /// Print C++ access specifiers
        #[structopt(short = "a", long)]
        print_access_specifiers: Option<AccessSpecifierReconstructionFlavor>,
        /// Print the local variables of functions
        #[structopt(long)]
        print_local_variables: bool,
        /// Print where local variables are stored (registers, stack offsets)
        #[structopt(long)]
        print_local_variable_locations: bool,
        /// Print the local variables of inlined call sites
        #[structopt(long)]
        print_inlined_call_sites: bool,
        /// Indentation: 2, 4 or tab
        #[structopt(long)]
        indentation: Option<IndentationFlavor>,
        /// Print where functions are defined (source file and line), when known
        #[structopt(long)]
        print_source_locations: bool,
        /// Highlight C++ output
        #[structopt(short = "H", long)]
        highlight_syntax: bool,
    },
    /// Compute diff for a symbol between two given PDB files
    DiffSymbol {
        /// Path of the PDB file to compute the diff from
        from_pdb_path: PathBuf,
        /// Path of the PDB file to compute the diff to
        to_pdb_path: PathBuf,
        /// Name of the module to diff
        symbol_name: String,
        /// Path of the output file
        output_file_path: Option<PathBuf>,
        /// Representation of primitive types
        #[structopt(short = "f", long)]
        primitive_types_flavor: Option<PrimitiveReconstructionFlavor>,
        /// Write tags and handles with their typedef name (e.g., `GUID`, `HWND`)
        #[structopt(long)]
        use_typedef_names: bool,
        /// Print header
        #[structopt(short = "h", long)]
        print_header: bool,
        /// Print C++ access specifiers
        #[structopt(short = "a", long)]
        print_access_specifiers: Option<AccessSpecifierReconstructionFlavor>,
        /// Print the local variables of functions
        #[structopt(long)]
        print_local_variables: bool,
        /// Print where local variables are stored (registers, stack offsets)
        #[structopt(long)]
        print_local_variable_locations: bool,
        /// Print the local variables of inlined call sites
        #[structopt(long)]
        print_inlined_call_sites: bool,
        /// Print declarations of the types the symbol uses
        #[structopt(short = "d", long)]
        print_dependencies: bool,
        /// Indentation: 2, 4 or tab
        #[structopt(long)]
        indentation: Option<IndentationFlavor>,
        /// Print where functions and types are defined (source file and line),
        /// when known
        #[structopt(long)]
        print_source_locations: bool,
        /// Highlight C++ output and add/deleted lines
        #[structopt(short = "H", long)]
        highlight_syntax: bool,
    },
    /// List source files (that define types or functions) from a given PDB file
    ListSourceFiles {
        /// Path to the PDB file
        pdb_path: PathBuf,
        /// Search filter
        source_file_path_filter: String,
        /// Path of the output file
        output_file_path: Option<PathBuf>,
        /// Do not match case
        #[structopt(short = "i", long)]
        case_insensitive: bool,
        /// Use regular expressions
        #[structopt(short = "r", long)]
        use_regex: bool,
    },
    /// Dump the types and functions defined in a source file, in line order
    DumpSourceFile {
        /// Path to the PDB file
        pdb_path: PathBuf,
        /// Path of the source file to dump
        source_file_path: String,
        /// Path of the output file
        output_file_path: Option<PathBuf>,
        #[structopt(flatten)]
        options: SourceFileOptions,
        /// Highlight C++ output
        #[structopt(short = "H", long)]
        highlight_syntax: bool,
    },
    /// Compute diff for a source file between two given PDB files
    DiffSourceFile {
        /// Path of the PDB file to compute the diff from
        from_pdb_path: PathBuf,
        /// Path of the PDB file to compute the diff to
        to_pdb_path: PathBuf,
        /// Path of the source file to diff
        source_file_path: String,
        /// Path of the output file
        output_file_path: Option<PathBuf>,
        #[structopt(flatten)]
        options: SourceFileOptions,
        /// Highlight C++ output and add/deleted lines
        #[structopt(short = "H", long)]
        highlight_syntax: bool,
    },
}

// Options of the source file subcommands. Not a doc comment: structopt would
// use it as the description of the subcommands that flatten it.
#[derive(Debug, StructOpt)]
pub struct SourceFileOptions {
    /// Representation of primitive types
    #[structopt(short = "f", long)]
    pub primitive_types_flavor: Option<PrimitiveReconstructionFlavor>,
    /// Write tags and handles with their typedef name (e.g., `GUID`, `HWND`)
    #[structopt(long)]
    pub use_typedef_names: bool,
    /// Print header
    #[structopt(short = "h", long)]
    pub print_header: bool,
    /// Print C++ access specifiers
    #[structopt(short = "a", long)]
    pub print_access_specifiers: Option<AccessSpecifierReconstructionFlavor>,
    /// Use hexadecimal formatting
    #[structopt(short = "x", long)]
    pub integers_as_hexadecimal: bool,
    /// Print Size Info
    #[structopt(long)]
    pub size_print_flavor: Option<SizePrintFlavor>,
    /// Print Offset Info
    #[structopt(long)]
    pub print_offset_info: bool,
    /// Print Brackets On New Line
    #[structopt(long)]
    pub print_brackets_new_line: bool,
    /// Filter out types in the `std` namespace
    #[structopt(short = "s", long)]
    pub ignore_std_types: bool,
    /// Ignore compiler generated methods
    #[structopt(short = "g", long)]
    pub ignore_compiler_generated_methods: bool,
    /// Print fields and methods in PDB declaration order
    #[structopt(short = "m", long)]
    pub preserve_member_order: bool,
    /// Wrap types in `namespace` blocks instead of using qualified names
    #[structopt(short = "n", long)]
    pub reconstruct_namespaces: bool,
    /// Keep namespace blocks open across types and nest inner namespaces
    #[structopt(long)]
    pub unify_namespaces: bool,
    /// Indent the contents of namespace blocks
    #[structopt(long)]
    pub indent_namespaces: bool,
    /// Print `override` on virtual methods overriding a base class method
    #[structopt(short = "o", long)]
    pub print_override_specifiers: bool,
    /// Print parameter names of methods when the PDB has them
    #[structopt(short = "A", long)]
    pub print_argument_names: bool,
    /// Print the RVA of methods implemented in the PDB
    #[structopt(long)]
    pub print_method_rvas: bool,
    /// Print the calling convention of methods
    #[structopt(long)]
    pub print_calling_conventions: bool,
    /// Vftable entry comments: offset, index or disabled
    #[structopt(long)]
    pub vftable_comment_flavor: Option<VftableCommentFlavor>,
    /// Don't print the base class offset comments
    #[structopt(long)]
    pub hide_base_class_offsets: bool,
    /// Don't print `virtual` on methods printed with `override`
    #[structopt(long)]
    pub omit_virtual_on_overrides: bool,
    /// Prefix type names with their keyword (e.g., `struct IUnknown*`)
    #[structopt(long)]
    pub print_elaborated_type_specifiers: bool,
    /// Indentation: 2, 4 or tab
    #[structopt(long)]
    pub indentation: Option<IndentationFlavor>,
    /// Don't separate groups of members with blank lines
    #[structopt(long)]
    pub compact_output: bool,
    /// Print where types and functions are defined (source file and line)
    #[structopt(long)]
    pub print_source_locations: bool,
    /// Print the local variables of functions
    #[structopt(long)]
    pub print_local_variables: bool,
    /// Print where local variables are stored (registers, stack offsets)
    #[structopt(long)]
    pub print_local_variable_locations: bool,
    /// Print the local variables of inlined call sites
    #[structopt(long)]
    pub print_inlined_call_sites: bool,
}
