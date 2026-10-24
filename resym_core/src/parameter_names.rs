//! Parameter names of functions. Function types don't record them, so they are
//! read from the symbols describing a function's implementation.
//!
//! MSVC describes the parameters of a procedure with records directly inside
//! the procedure's scope, `this` first for instance methods:
//! - optimized code: `S_LOCAL` records flagged as parameters,
//! - non-optimized code: `S_REGREL32` (x64) or `S_BPREL32` (x86) records,
//!   which have no parameter flag and are followed by the local variables.
//!
//! Unnamed parameters are named `__formal`.

use pdb::FallibleIterator;

/// Name MSVC gives to unnamed parameters
const UNNAMED_PARAMETER: &str = "__formal";

/// Shape of a function's parameter list, read from the function's type
pub struct ParameterLayout {
    /// `LF_ARGLIST` describing the declared parameters
    pub argument_list: pdb::TypeIndex,
    /// Number of declared parameters, without `this` and `...`
    pub parameter_count: usize,
    /// Indicates if the function is an instance method (has a `this` parameter)
    pub has_this: bool,
    /// Indicates if the function is a method (static or not)
    pub is_member_function: bool,
}

impl ParameterLayout {
    pub fn from_type(
        type_finder: &pdb::TypeFinder<'_>,
        type_index: pdb::TypeIndex,
    ) -> Option<ParameterLayout> {
        let (argument_list, has_this, is_member_function) =
            match type_finder.find(type_index).ok()?.parse().ok()? {
                pdb::TypeData::MemberFunction(data) => {
                    (data.argument_list, data.this_pointer_type.is_some(), true)
                }
                pdb::TypeData::Procedure(data) => (data.argument_list, false, false),
                _ => return None,
            };
        let pdb::TypeData::ArgumentList(arguments) =
            type_finder.find(argument_list).ok()?.parse().ok()?
        else {
            return None;
        };
        // `...` is a `T_NOTYPE` (0) argument and has no symbol
        let parameter_count = arguments
            .arguments
            .iter()
            .filter(|argument| argument.0 != 0)
            .count();

        Some(ParameterLayout {
            argument_list,
            parameter_count,
            has_this,
            is_member_function,
        })
    }
}

/// Returns the names of the declared parameters (without `this`) of the
/// procedure ending at `procedure_end`, whose symbols `symbols` iterates
/// starting at the procedure symbol itself. Unnamed parameters are `None`.
///
/// Returns `None` when the symbols don't match the function's type, rather
/// than risking names being attributed to the wrong parameters.
pub fn procedure_parameter_names(
    mut symbols: pdb::SymbolIter<'_>,
    procedure_end: pdb::SymbolIndex,
    layout: &ParameterLayout,
) -> Option<Vec<Option<String>>> {
    // Skip the procedure symbol
    symbols.next().ok()?;

    let mut flagged_parameters = Vec::new();
    let mut frame_variables = Vec::new();
    // Depth of nested scopes (blocks, inlined call sites, ...), whose
    // variables aren't the procedure's parameters
    let mut depth: usize = 0;
    while let Ok(Some(symbol)) = symbols.next() {
        if symbol.index() >= procedure_end {
            break;
        }
        let Ok(symbol_data) = symbol.parse() else {
            continue;
        };
        match symbol_data {
            pdb::SymbolData::Block(_)
            | pdb::SymbolData::InlineSite(_)
            | pdb::SymbolData::Thunk(_)
            | pdb::SymbolData::SeparatedCode(_)
            | pdb::SymbolData::Procedure(_) => depth += 1,
            pdb::SymbolData::ProcedureEnd | pdb::SymbolData::InlineSiteEnd => {
                depth = depth.saturating_sub(1)
            }
            _ if depth > 0 => {}
            pdb::SymbolData::Local(local) if local.flags.isparam => {
                flagged_parameters.push(local.name.to_string().into_owned())
            }
            pdb::SymbolData::RegisterRelative(variable) => {
                frame_variables.push(variable.name.to_string().into_owned())
            }
            pdb::SymbolData::BasePointerRelative(variable) => {
                frame_variables.push(variable.name.to_string().into_owned())
            }
            pdb::SymbolData::RegisterVariable(variable) => {
                frame_variables.push(variable.name.to_string().into_owned())
            }
            _ => {}
        }
    }

    let expected_count = layout.parameter_count + usize::from(layout.has_this);
    let mut names = if !flagged_parameters.is_empty() {
        if flagged_parameters.len() != expected_count {
            return None;
        }
        flagged_parameters
    } else {
        // Parameters are the first frame variables, before the locals
        if frame_variables.len() < expected_count {
            return None;
        }
        frame_variables.truncate(expected_count);
        frame_variables
    };
    if layout.has_this {
        if names.first().map(String::as_str) != Some("this") {
            return None;
        }
        names.remove(0);
    }

    Some(
        names
            .into_iter()
            .map(|name| (name != UNNAMED_PARAMETER).then_some(name))
            .collect(),
    )
}
