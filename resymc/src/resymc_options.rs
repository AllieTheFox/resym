use std::path::PathBuf;

use resym_core::pdb_types::{
    AccessSpecifierReconstructionFlavor, PrimitiveReconstructionFlavor, SizePrintFlavor,
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
        /// Print header
        #[structopt(short = "h", long)]
        print_header: bool,
        /// Print C++ access specifiers
        #[structopt(short = "a", long)]
        print_access_specifiers: Option<AccessSpecifierReconstructionFlavor>,
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
        /// Print header
        #[structopt(short = "h", long)]
        print_header: bool,
        /// Print C++ access specifiers
        #[structopt(short = "a", long)]
        print_access_specifiers: Option<AccessSpecifierReconstructionFlavor>,
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
        /// Print header
        #[structopt(short = "h", long)]
        print_header: bool,
        /// Print C++ access specifiers
        #[structopt(short = "a", long)]
        print_access_specifiers: Option<AccessSpecifierReconstructionFlavor>,
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
        /// Print header
        #[structopt(short = "h", long)]
        print_header: bool,
        /// Print C++ access specifiers
        #[structopt(short = "a", long)]
        print_access_specifiers: Option<AccessSpecifierReconstructionFlavor>,
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
        /// Print header
        #[structopt(short = "h", long)]
        print_header: bool,
        /// Print C++ access specifiers
        #[structopt(short = "a", long)]
        print_access_specifiers: Option<AccessSpecifierReconstructionFlavor>,
        /// Highlight C++ output and add/deleted lines
        #[structopt(short = "H", long)]
        highlight_syntax: bool,
    },
}
