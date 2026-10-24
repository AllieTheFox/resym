pub mod backend;
pub mod diffing;
mod error;
pub mod frontend;
mod local_variables;
mod parameter_names;
pub mod pdb_file;
pub mod pdb_types;
pub mod rayon_utils;
mod registers;
pub mod syntax_highlighting;

pub use error::*;

const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");
