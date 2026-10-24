//! Local variables of functions, read from the symbols inside a procedure's
//! scope:
//! - non-optimized code: `S_REGREL32` (x64), `S_BPREL32` (x86) or
//!   `S_REGISTER` records, with a single location,
//! - optimized code: `S_LOCAL` records followed by `S_DEFRANGE_*` records
//!   giving the variable's location for ranges of code.
//!
//! Variables of nested blocks (`S_BLOCK32`) and inlined call sites
//! (`S_INLINESITE`) are recorded in nested scopes.

use std::collections::HashSet;

use pdb::FallibleIterator;

use crate::pdb_types::{type_name, NeededTypeSet, TypeForwarder, TypeNameFlavor};
use crate::registers::{register_name, CV_ALLREG_VFRAME};

/// x86 `CV_REG_EBP`, the register `S_BPREL32` offsets are relative to
const CV_REG_EBP: u16 = 22;
/// x86 `CV_REG_EBX`
const CV_REG_EBX: u16 = 20;
/// x64 `CV_AMD64_RSP`
const CV_AMD64_RSP: u16 = 335;
/// x64 `CV_AMD64_RBP`
const CV_AMD64_RBP: u16 = 334;
/// x64 `CV_AMD64_R13`
const CV_AMD64_R13: u16 = 341;

// `CV_LVARFLAGS` bits of `S_LOCAL` records. The `pdb` crate's
// `LocalVariableFlags` omits `fIsAggregated` (0x10) and reads every flag after
// it from the bit below, so the flags are decoded here.
const CV_LVARFLAG_ISOPTIMIZEDOUT: u16 = 0x100;
const CV_LVARFLAG_ISENREG_GLOB: u16 = 0x200;
const CV_LVARFLAG_ISENREG_STAT: u16 = 0x400;

/// Returns the `CV_LVARFLAGS` of an `S_LOCAL` record, which follow the record
/// kind (2 bytes) and the type index (4 bytes)
fn local_variable_flags(symbol: &pdb::Symbol<'_>) -> u16 {
    symbol
        .raw_bytes()
        .get(6..8)
        .map_or(0, |bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
}

/// Where a variable is stored
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VariableStorage {
    /// In a register
    Register(u16),
    /// A field of the variable, at `field_offset`, in a register
    FieldRegister { register: u16, field_offset: u32 },
    /// In memory at `register + offset`
    RegisterRelative { register: u16, offset: i32 },
    /// In memory at the function's frame pointer + `offset`, when the frame
    /// pointer register isn't known
    FrameRelative(i32),
}

/// Range of code, relative to the start of the function
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeRange {
    pub start: u32,
    pub end: u32,
    /// Indicates if the variable isn't in this location for parts of the range
    pub has_gaps: bool,
}

/// Location of a variable, valid in the whole scope or in a range of code
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariableLocation {
    pub storage: VariableStorage,
    pub range: Option<CodeRange>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalVariable {
    pub name: String,
    pub type_index: pdb::TypeIndex,
    pub is_parameter: bool,
    pub is_optimized_out: bool,
    /// Copy of a global (or static) variable kept in a register for part of
    /// the function, which isn't a variable of the function
    pub enregistered: Option<EnregisteredVariable>,
    pub locations: Vec<VariableLocation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnregisteredVariable {
    Global,
    Static,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeKind {
    Procedure,
    Block,
    InlineSite(pdb::IdIndex),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub kind: ScopeKind,
    pub variables: Vec<LocalVariable>,
    pub scopes: Vec<Scope>,
}

impl Scope {
    fn new(kind: ScopeKind) -> Self {
        Self {
            kind,
            variables: Vec::new(),
            scopes: Vec::new(),
        }
    }
}

/// Frame pointer registers declared by `S_FRAMEPROC`, for locals and
/// parameters. `None` when not declared or unknown for the machine.
#[derive(Debug, Clone, Copy, Default)]
struct FramePointers {
    locals: Option<u16>,
    parameters: Option<u16>,
}

impl FramePointers {
    /// Decodes the 2-bit frame pointer encodings of the `S_FRAMEPROC` flags
    /// (bits 14-15 for locals, 16-17 for parameters). The register of each
    /// encoding is defined for x86 and x64 only (see LLVM's
    /// `codeview::decodeFramePtrReg`).
    fn from_frame_procedure(symbol: &pdb::Symbol<'_>, machine_type: pdb::MachineType) -> Self {
        // Record kind (2 bytes), 4 `u32` counts and the exception handler's
        // address (6 bytes) precede the flags
        const FLAGS_OFFSET: usize = 2 + 4 * 4 + 6;
        let Some(flags) = symbol
            .raw_bytes()
            .get(FLAGS_OFFSET..FLAGS_OFFSET + 4)
            .map(|bytes| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        else {
            return Self::default();
        };
        let decode = |encoded: u32| match (machine_type, encoded) {
            (_, 0) => None,
            (pdb::MachineType::X86, 1) => Some(CV_ALLREG_VFRAME),
            (pdb::MachineType::X86, 2) => Some(CV_REG_EBP),
            (pdb::MachineType::X86, 3) => Some(CV_REG_EBX),
            (pdb::MachineType::Amd64, 1) => Some(CV_AMD64_RSP),
            (pdb::MachineType::Amd64, 2) => Some(CV_AMD64_RBP),
            (pdb::MachineType::Amd64, 3) => Some(CV_AMD64_R13),
            _ => None,
        };
        Self {
            locals: decode((flags >> 14) & 3),
            parameters: decode((flags >> 16) & 3),
        }
    }

    fn storage(&self, is_parameter: bool, offset: i32) -> VariableStorage {
        let register = if is_parameter {
            self.parameters
        } else {
            self.locals
        };
        match register {
            Some(register) => VariableStorage::RegisterRelative { register, offset },
            None => VariableStorage::FrameRelative(offset),
        }
    }
}

/// Returns the variables and nested scopes of the procedure starting at
/// `procedure_offset` and ending at `procedure_end`. `procedure_symbols`
/// returns an iterator over the symbols of the procedure, starting at the
/// procedure symbol itself.
///
/// `parameter_count` is the number of parameters of the function, `this`
/// included. Non-optimized code doesn't flag parameters: they are the first
/// variables of the procedure's scope.
pub fn procedure_local_variables<'s>(
    procedure_symbols: impl Fn() -> Option<pdb::SymbolIter<'s>>,
    procedure_offset: pdb::PdbInternalSectionOffset,
    procedure_end: pdb::SymbolIndex,
    parameter_count: usize,
    machine_type: pdb::MachineType,
) -> Scope {
    // `S_FRAMEPROC` follows the `S_LOCAL` records whose frame relative
    // locations it defines, so it's read first. Optimized code (`S_LOCAL`)
    // also repeats stack variables as unflagged `S_REGREL32` or `S_BPREL32`
    // records.
    let mut frame_pointers = None;
    let mut uses_local_records = false;
    // Indicates if `S_LOCAL` records of the procedure's scope flag its
    // parameters. Otherwise, its first unflagged variables are.
    let mut has_flagged_parameters = false;
    if let Some(mut symbols) = procedure_symbols() {
        // Skip the procedure symbol
        let _ = symbols.next();
        let mut depth: usize = 0;
        while let Ok(Some(symbol)) = symbols.next() {
            if symbol.index() >= procedure_end {
                break;
            }
            match symbol.parse() {
                Ok(pdb::SymbolData::Block(_))
                | Ok(pdb::SymbolData::InlineSite(_))
                | Ok(pdb::SymbolData::Thunk(_))
                | Ok(pdb::SymbolData::SeparatedCode(_))
                | Ok(pdb::SymbolData::Procedure(_)) => depth += 1,
                Ok(pdb::SymbolData::ScopeEnd)
                | Ok(pdb::SymbolData::ProcedureEnd)
                | Ok(pdb::SymbolData::InlineSiteEnd) => depth = depth.saturating_sub(1),
                Ok(pdb::SymbolData::FrameProcedure(_)) if frame_pointers.is_none() => {
                    frame_pointers =
                        Some(FramePointers::from_frame_procedure(&symbol, machine_type));
                }
                Ok(pdb::SymbolData::Local(local)) => {
                    uses_local_records = true;
                    has_flagged_parameters |= depth == 0 && local.flags.isparam;
                }
                _ => {}
            }
        }
    }
    let frame_pointers = frame_pointers.unwrap_or_default();

    // Scopes being read, the procedure's first
    let mut scope_stack = vec![Scope::new(ScopeKind::Procedure)];
    let Some(mut symbols) = procedure_symbols() else {
        return scope_stack.remove(0);
    };
    // Skip the procedure symbol
    let _ = symbols.next();
    // Depth of nested scopes whose variables aren't recorded (code separated
    // from the function, thunks, ...)
    let mut ignored_depth: usize = 0;
    // Unflagged variables of the procedure's scope read so far
    let mut unflagged_variable_count: usize = 0;
    let code_range = |range: &pdb::AddressRange, gaps: &[pdb::AddressGap]| {
        (range.offset.section == procedure_offset.section).then(|| {
            let start = range.offset.offset.wrapping_sub(procedure_offset.offset);
            CodeRange {
                start,
                end: start + u32::from(range.cb_range),
                has_gaps: !gaps.is_empty(),
            }
        })
    };

    while let Ok(Some(symbol)) = symbols.next() {
        if symbol.index() >= procedure_end {
            break;
        }
        let Ok(symbol_data) = symbol.parse() else {
            continue;
        };

        if ignored_depth > 0 {
            match symbol_data {
                pdb::SymbolData::Block(_)
                | pdb::SymbolData::InlineSite(_)
                | pdb::SymbolData::Thunk(_)
                | pdb::SymbolData::SeparatedCode(_)
                | pdb::SymbolData::Procedure(_) => ignored_depth += 1,
                pdb::SymbolData::ScopeEnd
                | pdb::SymbolData::ProcedureEnd
                | pdb::SymbolData::InlineSiteEnd => ignored_depth -= 1,
                _ => {}
            }
            continue;
        }

        let scope_index = scope_stack.len() - 1;
        let is_procedure_scope = scope_index == 0;
        let scope = &mut scope_stack[scope_index];
        // Unflagged frame variables (`S_REGREL32`, `S_BPREL32`, `S_REGISTER`)
        let mut push_frame_variable =
            |name: pdb::RawString<'_>, type_index: pdb::TypeIndex, storage: VariableStorage| {
                let name = name.to_string().into_owned();
                // Copy of a stack variable described by `S_LOCAL`
                if uses_local_records && scope.variables.iter().any(|v| v.name == name) {
                    return;
                }
                // Parameters are the first unflagged variables of the procedure
                // (with an offset of 0 when not stored on the stack)
                let is_parameter = is_procedure_scope && !has_flagged_parameters && {
                    unflagged_variable_count += 1;
                    unflagged_variable_count <= parameter_count
                };
                scope.variables.push(LocalVariable {
                    name,
                    type_index,
                    is_parameter,
                    is_optimized_out: false,
                    enregistered: None,
                    locations: vec![VariableLocation {
                        storage,
                        range: None,
                    }],
                });
            };

        match symbol_data {
            pdb::SymbolData::Block(_) => scope_stack.push(Scope::new(ScopeKind::Block)),
            pdb::SymbolData::InlineSite(site) => {
                scope_stack.push(Scope::new(ScopeKind::InlineSite(site.inlinee)))
            }
            pdb::SymbolData::Thunk(_)
            | pdb::SymbolData::SeparatedCode(_)
            | pdb::SymbolData::Procedure(_) => ignored_depth += 1,
            pdb::SymbolData::ScopeEnd
            | pdb::SymbolData::ProcedureEnd
            | pdb::SymbolData::InlineSiteEnd => {
                if scope_stack.len() > 1 {
                    let ended_scope = scope_stack.pop().expect("stack has 2 scopes or more");
                    scope_stack
                        .last_mut()
                        .expect("stack has 1 scope or more")
                        .scopes
                        .push(ended_scope);
                }
            }

            // Non-optimized code
            pdb::SymbolData::RegisterRelative(variable) => push_frame_variable(
                variable.name,
                variable.type_index,
                VariableStorage::RegisterRelative {
                    register: variable.register.0,
                    offset: variable.offset,
                },
            ),
            pdb::SymbolData::BasePointerRelative(variable) => push_frame_variable(
                variable.name,
                variable.type_index,
                // Offset from the base pointer register (EBP on x86)
                match machine_type {
                    pdb::MachineType::X86 => VariableStorage::RegisterRelative {
                        register: CV_REG_EBP,
                        offset: variable.offset,
                    },
                    _ => VariableStorage::FrameRelative(variable.offset),
                },
            ),
            pdb::SymbolData::RegisterVariable(variable) => push_frame_variable(
                variable.name,
                variable.type_index,
                VariableStorage::Register(variable.register.0),
            ),

            // Optimized code: the location follows in `S_DEFRANGE_*` records
            pdb::SymbolData::Local(local) => {
                let flags = local_variable_flags(&symbol);
                scope.variables.push(LocalVariable {
                    name: local.name.to_string().into_owned(),
                    type_index: local.type_index,
                    is_parameter: local.flags.isparam,
                    is_optimized_out: flags & CV_LVARFLAG_ISOPTIMIZEDOUT != 0,
                    enregistered: if flags & CV_LVARFLAG_ISENREG_GLOB != 0 {
                        Some(EnregisteredVariable::Global)
                    } else if flags & CV_LVARFLAG_ISENREG_STAT != 0 {
                        Some(EnregisteredVariable::Static)
                    } else {
                        None
                    },
                    locations: Vec::new(),
                })
            }
            pdb::SymbolData::DefRangeRegister(range) => {
                add_location(
                    scope,
                    VariableStorage::Register(range.register.0),
                    code_range(&range.range, &range.gaps),
                );
            }
            pdb::SymbolData::DefRangeSubFieldRegister(range) => {
                add_location(
                    scope,
                    VariableStorage::FieldRegister {
                        register: range.register.0,
                        field_offset: range.offset,
                    },
                    code_range(&range.range, &range.gaps),
                );
            }
            pdb::SymbolData::DefRangeRegisterRelative(range) => {
                add_location(
                    scope,
                    VariableStorage::RegisterRelative {
                        register: range.base_register.0,
                        offset: range.offset_base_pointer,
                    },
                    code_range(&range.range, &range.gaps),
                );
            }
            pdb::SymbolData::DefRangeFramePointerRelative(range) => {
                let is_parameter = last_variable_is_parameter(scope);
                add_location(
                    scope,
                    frame_pointers.storage(is_parameter, range.offset),
                    code_range(&range.range, &range.gaps),
                );
            }
            pdb::SymbolData::DefRangeFramePointerRelativeFullScope(range) => {
                let is_parameter = last_variable_is_parameter(scope);
                add_location(
                    scope,
                    frame_pointers.storage(is_parameter, range.offset),
                    None,
                );
            }
            _ => {}
        }
    }

    // Close scopes left open by truncated symbol streams
    while scope_stack.len() > 1 {
        let ended_scope = scope_stack.pop().expect("stack has 2 scopes or more");
        scope_stack
            .last_mut()
            .expect("stack has 1 scope or more")
            .scopes
            .push(ended_scope);
    }
    scope_stack.pop().expect("stack has the procedure's scope")
}

/// Adds a location to the last variable of `scope`, which `S_DEFRANGE_*`
/// records follow.
fn add_location(scope: &mut Scope, storage: VariableStorage, range: Option<CodeRange>) {
    if let Some(variable) = scope.variables.last_mut() {
        variable.locations.push(VariableLocation { storage, range });
    }
}

fn last_variable_is_parameter(scope: &Scope) -> bool {
    scope
        .variables
        .last()
        .is_some_and(|variable| variable.is_parameter)
}

/// What to print about local variables, and how
pub struct LocalVariableFormat<'a> {
    /// Print where each variable is stored
    pub print_locations: bool,
    /// Print the variables of inlined call sites
    pub print_inlined_call_sites: bool,
    /// One level of indentation
    pub indent: &'a str,
    pub machine_type: pdb::MachineType,
}

/// Types and IDs used to name variables' types and inlined functions
pub struct NameContext<'a, 't> {
    pub type_finder: &'a pdb::TypeFinder<'t>,
    pub type_forwarder: &'a TypeForwarder,
    pub type_name_flavor: &'a TypeNameFlavor,
    /// Needed to name inlined functions
    pub id_finder: Option<&'a pdb::IdFinder<'t>>,
    /// Turn cross module references of the procedure's module (link-time
    /// code generation) into indices of the global type and ID streams
    pub resolve_type_index: &'a dyn Fn(pdb::TypeIndex) -> pdb::TypeIndex,
    pub resolve_id_index: &'a dyn Fn(pdb::IdIndex) -> pdb::IdIndex,
}

/// Returns the lines of the body of a procedure listing its local variables
/// (without its parameters), each scope in a `{ }` block. Returns no lines
/// when there's nothing to print. The types the variables use are added to
/// `needed_types`.
pub fn local_variable_lines(
    procedure_scope: &Scope,
    names: &NameContext<'_, '_>,
    format: &LocalVariableFormat<'_>,
    needed_types: &mut NeededTypeSet,
) -> Vec<String> {
    let mut lines = Vec::new();
    scope_lines(procedure_scope, 1, names, format, needed_types, &mut lines);
    lines
}

fn scope_lines(
    scope: &Scope,
    depth: usize,
    names: &NameContext<'_, '_>,
    format: &LocalVariableFormat<'_>,
    needed_types: &mut NeededTypeSet,
    lines: &mut Vec<String>,
) {
    // Optimized code doesn't record the nested blocks of a function, so
    // variables of separate blocks can have the same name in one scope. Each
    // repeated name opens a block to keep declarations valid.
    let mut declared_names = HashSet::new();
    let mut repeated_name_blocks = 0;
    for variable in &scope.variables {
        // The procedure's parameters are in its signature, unlike the
        // parameters of inlined functions
        if variable.is_parameter && scope.kind == ScopeKind::Procedure {
            continue;
        }
        if variable.enregistered.is_none() && !declared_names.insert(variable.name.as_str()) {
            lines.push(format!(
                "{}{{ // separate block (repeated names)",
                format.indent.repeat(depth + repeated_name_blocks)
            ));
            repeated_name_blocks += 1;
            declared_names.clear();
            declared_names.insert(variable.name.as_str());
        }
        let indentation = format.indent.repeat(depth + repeated_name_blocks);
        let type_index = (names.resolve_type_index)(variable.type_index);
        let declaration = match type_name(
            names.type_finder,
            names.type_forwarder,
            type_index,
            names.type_name_flavor,
            needed_types,
        ) {
            Ok((type_left, type_right)) => {
                format!("{type_left} {}{type_right};", variable.name)
            }
            Err(_) => format!("/* unknown type 0x{:x} */ {};", type_index.0, variable.name),
        };

        let mut comments = Vec::new();
        if variable.is_parameter {
            comments.push("parameter".to_string());
        }
        // Not a variable of the function: commented out, as the name of a
        // global can be qualified
        let declaration = match variable.enregistered {
            None => declaration,
            Some(EnregisteredVariable::Global) => {
                comments.insert(0, "global held in a register".to_string());
                format!("// {declaration}")
            }
            Some(EnregisteredVariable::Static) => {
                comments.insert(0, "static held in a register".to_string());
                format!("// {declaration}")
            }
        };
        if format.print_locations {
            if variable.is_optimized_out {
                comments.push("optimized out".to_string());
            }
            comments.extend(
                variable
                    .locations
                    .iter()
                    .map(|location| location_string(location, format.machine_type)),
            );
        }
        if comments.is_empty() {
            lines.push(format!("{indentation}{declaration}"));
        } else {
            lines.push(format!(
                "{indentation}{declaration} // {}",
                comments.join(", ")
            ));
        }
    }
    for block in (0..repeated_name_blocks).rev() {
        lines.push(format!("{}}}", format.indent.repeat(depth + block)));
    }

    let indentation = format.indent.repeat(depth);
    for nested_scope in &scope.scopes {
        let opening_line = match nested_scope.kind {
            ScopeKind::InlineSite(_) if !format.print_inlined_call_sites => continue,
            ScopeKind::InlineSite(inlinee) => format!(
                "{indentation}{{ // inlined: {}",
                inlinee_name(names, inlinee).unwrap_or_else(|| format!("0x{:x}", inlinee.0))
            ),
            ScopeKind::Block | ScopeKind::Procedure => format!("{indentation}{{"),
        };
        let mut nested_lines = Vec::new();
        scope_lines(
            nested_scope,
            depth + 1,
            names,
            format,
            needed_types,
            &mut nested_lines,
        );
        if nested_lines.is_empty() {
            continue;
        }
        lines.push(opening_line);
        lines.append(&mut nested_lines);
        lines.push(format!("{indentation}}}"));
    }
}

fn location_string(location: &VariableLocation, machine_type: pdb::MachineType) -> String {
    let register = |register: u16| {
        register_name(machine_type, register)
            .map(str::to_string)
            .unwrap_or_else(|| format!("reg{register}"))
    };
    let signed_hex = |offset: i32| {
        if offset < 0 {
            format!("-0x{:x}", offset.unsigned_abs())
        } else {
            format!("+0x{offset:x}")
        }
    };
    let storage = match &location.storage {
        VariableStorage::Register(r) => register(*r),
        VariableStorage::FieldRegister {
            register: r,
            field_offset,
        } => format!("{} (field +0x{field_offset:x})", register(*r)),
        VariableStorage::RegisterRelative {
            register: r,
            offset,
        } => format!("[{}{}]", register(*r), signed_hex(*offset)),
        VariableStorage::FrameRelative(offset) => format!("[frame{}]", signed_hex(*offset)),
    };
    match &location.range {
        None => storage,
        Some(range) => format!(
            "{storage} @+0x{:x}..+0x{:x}{}",
            range.start,
            range.end,
            if range.has_gaps { " (with gaps)" } else { "" }
        ),
    }
}

/// Returns the qualified name of an inlined function
fn inlinee_name(names: &NameContext<'_, '_>, inlinee: pdb::IdIndex) -> Option<String> {
    let id_finder = names.id_finder?;
    match id_finder
        .find((names.resolve_id_index)(inlinee))
        .ok()?
        .parse()
        .ok()?
    {
        pdb::IdData::Function(function) => {
            let scope = function.scope.and_then(|scope| {
                match id_finder.find(scope).ok()?.parse().ok()? {
                    pdb::IdData::String(scope) => Some(scope.name.to_string().into_owned()),
                    _ => None,
                }
            });
            Some(match scope {
                Some(scope) => format!("{scope}::{}", function.name),
                None => function.name.to_string().into_owned(),
            })
        }
        pdb::IdData::MemberFunction(function) => {
            let class_name = match names.type_finder.find(function.parent).ok()?.parse().ok()? {
                pdb::TypeData::Class(class) => class.name.to_string().into_owned(),
                pdb::TypeData::Union(union) => union.name.to_string().into_owned(),
                _ => return Some(function.name.to_string().into_owned()),
            };
            Some(format!("{class_name}::{}", function.name))
        }
        _ => None,
    }
}
