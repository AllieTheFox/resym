mod class;
mod enumeration;
mod field;
mod forward_declaration;
mod method;
mod nested;
mod primitive_types;
mod union;

use std::cell::Cell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::fmt::Write as _;
use std::ops::Range;

use crate::error::{Result, ResymCoreError};
use class::Class;
use enumeration::Enum;
use field::{Field, FieldAccess};
use method::Method;
use primitive_types::primitive_kind_as_str;
use union::Union;

pub use primitive_types::{
    include_headers_for_flavor, AccessSpecifierReconstructionFlavor, IndentationFlavor,
    PrimitiveReconstructionFlavor, SizePrintFlavor, VftableCommentFlavor,
};

use self::forward_declaration::{ForwardDeclaration, ForwardDeclarationKind};

/// Set of (`TypeIndex`, bool) tuples.
///
/// The boolean value indicates whether the type was referenced via a pointer
/// or a C++ reference.
pub type NeededTypeSet = HashSet<(pdb::TypeIndex, bool)>;

pub type TypeForwarder = dashmap::DashMap<pdb::TypeIndex, pdb::TypeIndex>;

/// Where types are defined (`file(line)`), by type index
pub type TypeSourceLocations = HashMap<pdb::TypeIndex, String>;

/// Returns where the type with the given index is defined, if known
fn type_source_location(
    locations: &TypeSourceLocations,
    type_forwarder: &TypeForwarder,
    type_index: pdb::TypeIndex,
) -> Option<String> {
    locations
        .get(&type_index)
        .or_else(|| locations.get(&resolve_complete_type_index(type_forwarder, type_index)))
        .cloned()
}

/// Information read from the symbols of a method's implementation
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MethodImplementation {
    /// Names of the declared parameters (`None` for unnamed parameters), if
    /// they match the method's type
    pub parameter_names: Option<Vec<Option<String>>>,
    /// RVA of the method's code
    pub rva: Option<u32>,
}

/// Implementations of methods, by qualified method name and method type
/// (`LF_MFUNCTION`). Methods sharing a type can only be told apart by name.
pub type MethodImplementations = HashMap<(String, pdb::TypeIndex), MethodImplementation>;

/// Settings affecting how type names are written, applied while types are
/// collected (unlike `DataFormatConfiguration`, applied when writing them)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeNameFlavor {
    pub primitives: PrimitiveReconstructionFlavor,
    /// Prefix class, struct, union and enum names with their keyword
    /// (e.g., `struct IUnknown*`)
    pub elaborated_type_specifiers: bool,
}

impl TypeNameFlavor {
    /// Flavor with the given primitive types and no elaborated specifiers
    pub fn new(primitives: PrimitiveReconstructionFlavor) -> Self {
        Self {
            primitives,
            elaborated_type_specifiers: false,
        }
    }

    /// Same flavor without elaborated specifiers, for names that can't have
    /// one (e.g., base classes)
    pub fn without_elaborated_type_specifiers(self) -> Self {
        Self::new(self.primitives)
    }

    fn elaborate(&self, keyword: &str, name: String) -> String {
        if self.elaborated_type_specifiers {
            format!("{keyword} {name}")
        } else {
            name
        }
    }
}

/// Keyword declaring a class of the given kind
pub fn class_kind_keyword(kind: pdb::ClassKind) -> &'static str {
    match kind {
        pdb::ClassKind::Class => "class",
        pdb::ClassKind::Struct => "struct",
        // Not used C and C++ but well ...
        pdb::ClassKind::Interface => "interface",
    }
}

/// Return a pair of strings representing the given `type_index`.
pub fn type_name(
    type_finder: &pdb::TypeFinder,
    type_forwarder: &TypeForwarder,
    type_index: pdb::TypeIndex,
    type_name_flavor: &TypeNameFlavor,
    needed_types: &mut NeededTypeSet,
) -> Result<(String, String)> {
    let (type_left, type_right) = match type_finder.find(type_index)?.parse()? {
        pdb::TypeData::Primitive(data) => {
            let name = primitive_kind_as_str(
                &type_name_flavor.primitives,
                data.kind,
                data.indirection.is_some(),
            )?;

            (name, String::default())
        }

        pdb::TypeData::Class(data) => {
            needed_types.insert((type_index, false));
            // Rename unnamed anonymous tags to something unique
            let name = data.name.to_string();
            let name = if is_unnamed_type(&name) {
                format!("_unnamed_{type_index}")
            } else {
                name.into_owned()
            };
            (
                type_name_flavor.elaborate(class_kind_keyword(data.kind), name),
                String::default(),
            )
        }

        pdb::TypeData::Union(data) => {
            needed_types.insert((type_index, false));
            // Rename unnamed anonymous tags to something unique
            let name = data.name.to_string();
            let name = if is_unnamed_type(&name) {
                format!("_unnamed_{type_index}")
            } else {
                name.into_owned()
            };
            (type_name_flavor.elaborate("union", name), String::default())
        }

        pdb::TypeData::Enumeration(data) => {
            needed_types.insert((type_index, false));
            (
                type_name_flavor.elaborate("enum", data.name.to_string().into_owned()),
                String::default(),
            )
        }

        pdb::TypeData::Pointer(data) => {
            // Resolve the complete type's index, if present in the PDB
            let complete_underlying_type_index =
                resolve_complete_type_index(type_forwarder, data.underlying_type);
            let mut temporary_needed_types = HashSet::new();
            let (type_left, type_right) = type_name(
                type_finder,
                type_forwarder,
                complete_underlying_type_index,
                type_name_flavor,
                &mut temporary_needed_types,
            )?;

            if temporary_needed_types.len() < 2 {
                // "Simple" type (e.g., class, union, enum) -> add as pointer
                if let Some(needed_type) = temporary_needed_types.into_iter().next() {
                    needed_types.insert((needed_type.0, true));
                }
            } else {
                // "Complex" type (e.g., procedure) -> add as is
                needed_types.extend(temporary_needed_types);
            }

            let mut suffix = String::new();
            if data.attributes.is_mocom() {
                // how to tell the difference between ^ and %
                suffix.push_str("^ ");
            } else if data.attributes.pointer_mode() == pdb::PointerMode::RValueReference {
                suffix.push_str("&& ");
            } else if data.attributes.pointer_mode() == pdb::PointerMode::LValueReference {
                suffix.push_str("& ");
            } else {
                suffix.push_str("* ");
            }
            if data.attributes.is_unaligned() {
                // how to tell the difference between ^ and %
                suffix.push_str("__unaligned ");
            }
            if data.attributes.is_restrict() {
                // how to tell the difference between ^ and %
                suffix.push_str("__restrict ");
            }
            (format!("{}{}", type_left, suffix.trim_end()), type_right)
        }

        pdb::TypeData::Modifier(data) => {
            // Resolve the complete type's index, if present in the PDB
            let complete_underlying_type_index =
                resolve_complete_type_index(type_forwarder, data.underlying_type);
            let (type_left, type_right) = type_name(
                type_finder,
                type_forwarder,
                complete_underlying_type_index,
                type_name_flavor,
                needed_types,
            )?;

            let mut prefix = String::new();
            if data.constant {
                prefix.push_str("const ");
            }
            if data.volatile {
                prefix.push_str("volatile ");
            }
            if data.unaligned {
                prefix.push_str("__unaligned ");
            }
            (format!("{}{}", prefix, type_left), type_right)
        }

        pdb::TypeData::Array(data) => {
            // Resolve the complete type's index, if present in the PDB
            let complete_element_type_index =
                resolve_complete_type_index(type_forwarder, data.element_type);
            let ((type_left, type_right), mut dimensions) = array_base_name(
                type_finder,
                type_forwarder,
                complete_element_type_index,
                type_name_flavor,
                needed_types,
            )?;
            let type_size = u32::try_from(type_size(type_finder, complete_element_type_index)?)?;
            let mut divider = if type_size == 0 {
                log::warn!(
                    "'{}{}' has invalid size (0), array dimensions might be incorrect",
                    type_left,
                    type_right,
                );
                1
            } else {
                type_size
            };

            let mut dimensions_elem_count = data
                .dimensions
                .into_iter()
                .map(|dim_size| {
                    let result = dim_size / divider;
                    divider = dim_size;
                    result as usize
                })
                .collect::<Vec<_>>();
            dimensions.append(&mut dimensions_elem_count);

            let mut dimensions_str = String::default();
            // Note: Dimensions are collected in reverse order so we have to use
            // a reverse iterator
            for dim in dimensions.iter().rev() {
                dimensions_str = format!("{dimensions_str}[{dim}]");
            }

            (type_left, format!("{}{}", dimensions_str, type_right))
        }

        pdb::TypeData::Bitfield(data) => {
            // Resolve the complete type's index, if present in the PDB
            let complete_underlying_type_index =
                resolve_complete_type_index(type_forwarder, data.underlying_type);
            let (type_left, type_right) = type_name(
                type_finder,
                type_forwarder,
                complete_underlying_type_index,
                type_name_flavor,
                needed_types,
            )?;
            (type_left, format!("{} : {}", type_right, data.length))
        }

        pdb::TypeData::Procedure(data) => {
            // TODO: Parse and display attributes
            let (ret_type_left, ret_type_right) = if let Some(return_type) = data.return_type {
                // Resolve the complete type's index, if present in the PDB
                let complete_return_type_index =
                    resolve_complete_type_index(type_forwarder, return_type);
                type_name(
                    type_finder,
                    type_forwarder,
                    complete_return_type_index,
                    type_name_flavor,
                    needed_types,
                )?
            } else {
                ("void".to_string(), String::default())
            };
            let arg_list = argument_list(
                type_finder,
                type_forwarder,
                data.argument_list,
                type_name_flavor,
                needed_types,
            )?;

            (
                format!("{ret_type_left}{ret_type_right} ("),
                format!(
                    ")({})",
                    arg_list
                        .into_iter()
                        .map(|(type_left, type_right)| format!("{type_left}{type_right}"))
                        .collect::<Vec<String>>()
                        .join(", ")
                ),
            )
        }

        pdb::TypeData::MemberFunction(data) => {
            // Resolve the complete type's index, if present in the PDB
            let complete_return_type_index =
                resolve_complete_type_index(type_forwarder, data.return_type);
            // let complete_class_type_index =
            //     resolve_complete_type_index(type_forwarder, data.class_type);
            // // TODO: Parse and display attributes
            let (ret_type_left, ret_type_right) = type_name(
                type_finder,
                type_forwarder,
                complete_return_type_index,
                type_name_flavor,
                needed_types,
            )?;
            // let (class_type_left, _) = type_name(
            //     type_finder,
            //     type_forwarder,
            //     complete_class_type_index,
            //     primitive_flavor,
            //     needed_types,
            // )?;
            let arg_list = argument_list(
                type_finder,
                type_forwarder,
                data.argument_list,
                type_name_flavor,
                needed_types,
            )?;

            (
                format!(
                    "{ret_type_left}{ret_type_right} ({}",
                    /*format!("{}::", class_type_left)*/ String::new()
                ),
                format!(
                    ")({})",
                    arg_list
                        .into_iter()
                        .map(|(type_left, type_right)| format!("{type_left}{type_right}"))
                        .collect::<Vec<String>>()
                        .join(", ")
                ),
            )
        }

        type_data => {
            log::warn!(
                "FIXME: figure out how to name it: TypeIndex={}, TypeData={:?}",
                type_index,
                type_data
            );
            ("FIXME_UNKNOWN_TYPE".to_string(), String::default())
        }
    };

    // TODO: search and replace std:: patterns (see issue #4)

    Ok((type_left, type_right))
}

fn array_base_name(
    type_finder: &pdb::TypeFinder,
    type_forwarder: &TypeForwarder,
    type_index: pdb::TypeIndex,
    type_name_flavor: &TypeNameFlavor,
    needed_types: &mut NeededTypeSet,
) -> Result<((String, String), Vec<usize>)> {
    match type_finder.find(type_index)?.parse()? {
        pdb::TypeData::Array(data) => {
            // Resolve the complete type's index, if present in the PDB
            let complete_element_type_index =
                resolve_complete_type_index(type_forwarder, data.element_type);
            let ((type_left, type_right), mut base_dimensions) = array_base_name(
                type_finder,
                type_forwarder,
                complete_element_type_index,
                type_name_flavor,
                needed_types,
            )?;
            let type_size = u32::try_from(type_size(type_finder, complete_element_type_index)?)?;
            let mut divider = if type_size == 0 {
                log::warn!(
                    "'{}{}' has invalid size (0), array dimensions might be incorrect",
                    type_left,
                    type_right,
                );
                1
            } else {
                type_size
            };

            let mut dimensions_elem_count = data
                .dimensions
                .into_iter()
                .map(|dim_size| {
                    let result = dim_size / divider;
                    divider = dim_size;
                    result as usize
                })
                .collect::<Vec<_>>();
            base_dimensions.append(&mut dimensions_elem_count);

            Ok(((type_left, type_right), base_dimensions))
        }
        _ => Ok((
            type_name(
                type_finder,
                type_forwarder,
                type_index,
                type_name_flavor,
                needed_types,
            )?,
            vec![],
        )),
    }
}

pub fn argument_list(
    type_finder: &pdb::TypeFinder,
    type_forwarder: &TypeForwarder,
    type_index: pdb::TypeIndex,
    type_name_flavor: &TypeNameFlavor,
    needed_types: &mut NeededTypeSet,
) -> Result<Vec<(String, String)>> {
    match type_finder.find(type_index)?.parse()? {
        pdb::TypeData::ArgumentList(data) => {
            let mut args = Vec::new();
            for arg_type in data.arguments {
                args.push(type_name(
                    type_finder,
                    type_forwarder,
                    arg_type,
                    type_name_flavor,
                    needed_types,
                )?);
            }
            Ok(args)
        }
        _ => Err(ResymCoreError::InvalidParameterError(
            "argument list of non-argument-list type".to_owned(),
        )),
    }
}

/// Return the type's offset in bits, if the type is a bitfield.
pub fn type_bitfield_info(
    type_finder: &pdb::TypeFinder,
    type_index: pdb::TypeIndex,
) -> Result<Option<(u8, u8)>> {
    let bitfield_info = match type_finder.find(type_index)?.parse()? {
        pdb::TypeData::Bitfield(data) => Some((data.position, data.length)),
        _ => None,
    };

    Ok(bitfield_info)
}

/// Return the type's size in bytes.
pub fn type_size(type_finder: &pdb::TypeFinder, type_index: pdb::TypeIndex) -> Result<usize> {
    let size = match type_finder.find(type_index)?.parse()? {
        pdb::TypeData::Primitive(data) => {
            let mut size = match data.kind {
                pdb::PrimitiveKind::Char
                | pdb::PrimitiveKind::RChar
                | pdb::PrimitiveKind::UChar
                | pdb::PrimitiveKind::I8
                | pdb::PrimitiveKind::U8
                | pdb::PrimitiveKind::Bool8 => 1,

                pdb::PrimitiveKind::WChar
                | pdb::PrimitiveKind::RChar16
                | pdb::PrimitiveKind::I16
                | pdb::PrimitiveKind::Short
                | pdb::PrimitiveKind::U16
                | pdb::PrimitiveKind::UShort
                | pdb::PrimitiveKind::Bool16 => 2,

                pdb::PrimitiveKind::RChar32
                | pdb::PrimitiveKind::I32
                | pdb::PrimitiveKind::Long
                | pdb::PrimitiveKind::U32
                | pdb::PrimitiveKind::ULong
                | pdb::PrimitiveKind::F32
                | pdb::PrimitiveKind::Bool32 => 4,

                pdb::PrimitiveKind::I64
                | pdb::PrimitiveKind::Quad
                | pdb::PrimitiveKind::U64
                | pdb::PrimitiveKind::UQuad
                | pdb::PrimitiveKind::F64
                | pdb::PrimitiveKind::Bool64 => 8,

                _ => 0,
            };

            if let Some(indirection) = data.indirection {
                size = match indirection {
                    pdb::Indirection::Near16
                    | pdb::Indirection::Far16
                    | pdb::Indirection::Huge16 => 2,
                    pdb::Indirection::Near32 | pdb::Indirection::Far32 => 4,
                    pdb::Indirection::Near64 => 8,
                    pdb::Indirection::Near128 => 16,
                };
            }

            size
        }

        pdb::TypeData::Class(data) => data.size as usize,

        pdb::TypeData::Enumeration(data) => type_size(type_finder, data.underlying_type)?,

        pdb::TypeData::Union(data) => data.size as usize,

        pdb::TypeData::Pointer(data) => match data.attributes.pointer_kind() {
            pdb::PointerKind::Near16 | pdb::PointerKind::Far16 | pdb::PointerKind::Huge16 => 2,

            pdb::PointerKind::Near32
            | pdb::PointerKind::Far32
            | pdb::PointerKind::BaseSeg
            | pdb::PointerKind::BaseVal
            | pdb::PointerKind::BaseSegVal
            | pdb::PointerKind::BaseAddr
            | pdb::PointerKind::BaseSegAddr
            | pdb::PointerKind::BaseType
            | pdb::PointerKind::BaseSelf => 4,

            pdb::PointerKind::Ptr64 => 8,
        },

        pdb::TypeData::Modifier(data) => type_size(type_finder, data.underlying_type)?,

        pdb::TypeData::Array(data) => *data.dimensions.iter().last().unwrap_or(&0) as usize,

        _ => 0,
    };

    Ok(size)
}

/// Indicate if the given `type_name` is the name of an anonymous type.
pub fn is_unnamed_type(type_name: &str) -> bool {
    type_name.contains("<anonymous-")
        || type_name.contains("<unnamed-")
        || type_name.contains("__unnamed")
}

/// Trait for type data that can be reconstructed to C++
pub trait ReconstructibleTypeData {
    fn reconstruct(
        &self,
        fmt_configuration: &DataFormatConfiguration,
        f: &mut impl std::fmt::Write,
    ) -> fmt::Result;
}

/// Struct that represent a set of reconstructed types (forward declarations,
/// classes/structs, enums and unions)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Data<'p> {
    /// Do not reconstruct types in the `std` namespace
    ignore_std_types: bool,
    /// Forward-declared types which are defined in this PDB
    forward_declarations: BTreeMap<pdb::TypeIndex, ForwardDeclaration>,
    /// Enum types
    enums: BTreeMap<pdb::TypeIndex, Enum<'p>>,
    /// Class/struct types
    classes: BTreeMap<pdb::TypeIndex, Class<'p>>,
    /// Union types
    unions: BTreeMap<pdb::TypeIndex, Union<'p>>,
    /// Unique type names
    type_names: HashSet<String>,
    /// Types declared inside another type (their qualifier isn't a namespace)
    nested_type_indices: HashSet<pdb::TypeIndex>,
}

impl Data<'_> {
    pub fn reconstruct(
        &self,
        fmt_configuration: &DataFormatConfiguration,
        type_depth_map: &BTreeMap<usize, Vec<pdb::TypeIndex>>,
        output_writer: &mut impl std::fmt::Write,
    ) -> Result<()> {
        // Types defined inside another reconstructed type are written there
        let inlined_nested_types = self.inlined_nested_types();
        let mut namespace_scope = NamespaceScope::new(
            fmt_configuration.reconstruct_namespaces,
            fmt_configuration.unify_namespaces,
            fmt_configuration.indent_namespaces,
            fmt_configuration.print_brackets_new_line,
            &self.nested_type_indices,
        );
        // The contents of namespace blocks are indented through this writer
        let output_writer = &mut IndentingWriter::new(output_writer, fmt_configuration.indent());

        // Forward declarations (they don't depend on each other: sorting them
        // by namespace keeps the order of each namespace's declarations)
        let mut forward_declarations: Vec<_> = self.forward_declarations.iter().collect();
        if fmt_configuration.reconstruct_namespaces && fmt_configuration.group_namespaces {
            forward_declarations
                .sort_by_key(|(type_index, _)| self.definition_namespace(**type_index));
        }
        let mut is_first_forward_declaration = true;
        for (type_index, e) in forward_declarations {
            if inlined_nested_types.contains_key(type_index) {
                continue;
            }
            if self.ignore_std_types && e.name.starts_with("std::") {
                // Type is in the `std` namespace and should be ignored
                continue;
            }
            let (name, opened_namespace) =
                namespace_scope.enter(*type_index, &e.name, output_writer)?;
            if is_first_forward_declaration && !opened_namespace {
                writeln!(output_writer)?;
            }
            is_first_forward_declaration = false;
            e.reconstruct_as(name, output_writer)?;
        }

        if !type_depth_map.is_empty() {
            // Follow type depth map order
            for type_indices in type_depth_map.values().rev() {
                for type_index in type_indices.iter() {
                    if inlined_nested_types.contains_key(type_index) {
                        continue;
                    }
                    self.reconstruct_definition(
                        *type_index,
                        fmt_configuration,
                        &mut namespace_scope,
                        output_writer,
                    )?;
                }
            }
        } else {
            // Follow type index order: enums, then classes/structs, then unions
            let type_indices = self
                .enums
                .keys()
                .chain(self.classes.keys())
                .chain(self.unions.keys());
            for type_index in type_indices {
                if inlined_nested_types.contains_key(type_index) {
                    continue;
                }
                self.reconstruct_definition(
                    *type_index,
                    fmt_configuration,
                    &mut namespace_scope,
                    output_writer,
                )?;
            }
        }
        namespace_scope.close(output_writer)?;

        Ok(())
    }

    /// Writes the definition of the enum, class/struct or union with the
    /// given index, if any.
    fn reconstruct_definition(
        &self,
        type_index: pdb::TypeIndex,
        fmt_configuration: &DataFormatConfiguration,
        namespace_scope: &mut NamespaceScope,
        output_writer: &mut IndentingWriter<'_, impl std::fmt::Write>,
    ) -> fmt::Result {
        // Enum definitions
        if let Some(e) = self.enums.get(&type_index) {
            if self.ignore_std_types && e.name.starts_with("std::") {
                // Type is in the `std` namespace and should be ignored
                return Ok(());
            }
            let (name, _) = namespace_scope.enter(type_index, &e.name, output_writer)?;
            writeln!(output_writer)?;
            e.reconstruct_as(name, fmt_configuration, output_writer)?;
        }
        // Class definitions
        else if let Some(c) = self.classes.get(&type_index) {
            if self.ignore_std_types && c.name.starts_with("std::") {
                // Type is in the `std` namespace and should be ignored
                return Ok(());
            }
            let (name, _) = namespace_scope.enter(type_index, &c.name, output_writer)?;
            writeln!(output_writer)?;
            c.reconstruct_as(name, fmt_configuration, output_writer)?;
        }
        // Union definitions
        else if let Some(u) = self.unions.get(&type_index) {
            if self.ignore_std_types && u.name.starts_with("std::") {
                // Type is in the `std` namespace and should be ignored
                return Ok(());
            }
            let (name, _) = namespace_scope.enter(type_index, &u.name, output_writer)?;
            writeln!(output_writer)?;
            u.reconstruct_as(name, fmt_configuration, output_writer)?;
        }

        Ok(())
    }

    /// Sets what is known about the implementations of the methods of every
    /// type of this set (including nested types): parameter names and/or
    /// RVAs.
    pub fn assign_method_implementations(
        &mut self,
        implementations: &MethodImplementations,
        parameter_names: bool,
        rvas: bool,
    ) {
        for class in self.classes.values_mut() {
            class.assign_method_implementations(implementations, parameter_names, rvas);
        }
        for u in self.unions.values_mut() {
            u.assign_method_implementations(implementations, parameter_names, rvas);
        }
    }

    /// Sets where every type of this set (including nested types) is defined.
    pub fn assign_source_locations(
        &mut self,
        locations: &TypeSourceLocations,
        type_forwarder: &TypeForwarder,
    ) {
        for e in self.enums.values_mut() {
            e.source_location = type_source_location(locations, type_forwarder, e.index);
        }
        for class in self.classes.values_mut() {
            class.assign_source_locations(locations, type_forwarder);
        }
        for u in self.unions.values_mut() {
            u.assign_source_locations(locations, type_forwarder);
        }
    }

    /// Returns the namespace the definition of the given type is written in
    /// when namespaces are reconstructed, if any.
    pub fn definition_namespace(&self, type_index: pdb::TypeIndex) -> Option<&str> {
        if self.nested_type_indices.contains(&type_index) {
            return None;
        }
        let name = self
            .enums
            .get(&type_index)
            .map(|e| &e.name)
            .or_else(|| self.classes.get(&type_index).map(|c| &c.name))
            .or_else(|| self.unions.get(&type_index).map(|u| &u.name))
            .or_else(|| self.forward_declarations.get(&type_index).map(|f| &f.name))?;
        split_qualified_name(name)
            .map(|(namespace, _)| namespace)
            .filter(|namespace| is_namespace_path(namespace))
    }

    /// Returns the types that are also defined inside another type of this
    /// set, mapped to the top-level type whose reconstruction contains them.
    pub fn inlined_nested_types(&self) -> HashMap<pdb::TypeIndex, pdb::TypeIndex> {
        let mut defining_type_by_name: HashMap<String, pdb::TypeIndex> = HashMap::new();
        for (type_index, class) in &self.classes {
            for nested in &class.nested_types {
                nested.visit_type_names(&mut |name| {
                    defining_type_by_name.insert(name.to_owned(), *type_index);
                });
            }
        }
        for (type_index, u) in &self.unions {
            for nested in &u.nested_types {
                nested.visit_type_names(&mut |name| {
                    defining_type_by_name.insert(name.to_owned(), *type_index);
                });
            }
        }
        if defining_type_by_name.is_empty() {
            return HashMap::new();
        }

        let names = self
            .forward_declarations
            .iter()
            .map(|(type_index, e)| (type_index, &e.name))
            .chain(
                self.enums
                    .iter()
                    .map(|(type_index, e)| (type_index, &e.name)),
            )
            .chain(
                self.classes
                    .iter()
                    .map(|(type_index, c)| (type_index, &c.name)),
            )
            .chain(
                self.unions
                    .iter()
                    .map(|(type_index, u)| (type_index, &u.name)),
            );
        names
            .filter_map(|(type_index, name)| {
                defining_type_by_name
                    .get(name)
                    .filter(|defining_type_index| *defining_type_index != type_index)
                    .map(|defining_type_index| (*type_index, *defining_type_index))
            })
            .collect()
    }

    /// Returns the names of the types declaring nested type definitions of
    /// this set (other than `excluded_type_index`) when they aren't part of
    /// the set. Nested types can only be defined inside their containing type.
    pub fn missing_containing_type_names(
        &self,
        excluded_type_index: Option<pdb::TypeIndex>,
    ) -> HashSet<String> {
        let inlined_nested_types = self.inlined_nested_types();
        let names = self
            .enums
            .iter()
            .map(|(type_index, e)| (type_index, &e.name))
            .chain(
                self.classes
                    .iter()
                    .map(|(type_index, c)| (type_index, &c.name)),
            )
            .chain(
                self.unions
                    .iter()
                    .map(|(type_index, u)| (type_index, &u.name)),
            );
        names
            .filter(|(type_index, _)| {
                Some(**type_index) != excluded_type_index
                    && self.nested_type_indices.contains(type_index)
                    && !inlined_nested_types.contains_key(type_index)
            })
            .filter_map(|(_, name)| split_qualified_name(name))
            .map(|(containing_type_name, _)| containing_type_name.to_owned())
            .filter(|containing_type_name| !self.type_names.contains(containing_type_name))
            .collect()
    }
}

impl<'p> Data<'p> {
    pub fn new(ignore_std_types: bool) -> Self {
        Self {
            ignore_std_types,
            forward_declarations: BTreeMap::new(),
            classes: BTreeMap::new(),
            enums: BTreeMap::new(),
            unions: BTreeMap::new(),
            type_names: HashSet::new(),
            nested_type_indices: HashSet::new(),
        }
    }

    pub fn add(
        &mut self,
        type_finder: &pdb::TypeFinder<'p>,
        type_forwarder: &TypeForwarder,
        type_index: pdb::TypeIndex,
        type_name_flavor: &TypeNameFlavor,
        needed_types: &mut NeededTypeSet,
    ) -> Result<()> {
        match type_finder.find(type_index)?.parse()? {
            pdb::TypeData::Class(data) => {
                let name_str = data.name.to_string();
                // Rename unnamed anonymous tags to something unique
                let name = if is_unnamed_type(&name_str) {
                    format!("_unnamed_{type_index}")
                } else {
                    name_str.into_owned()
                };

                if self.type_names.contains(&name) {
                    // Type has already been added, return
                    return Ok(());
                }
                if data.properties.forward_reference() {
                    // Type isn't a complete type, return
                    return Ok(());
                }

                let mut class = Class::new(type_index, data.kind, name.clone(), data.size);

                if let Some(derived_from) = data.derived_from {
                    class.add_derived_from(type_finder, derived_from, needed_types)?;
                }

                if let Some(fields) = data.fields {
                    // Note: Do not propagate the error, this allows the
                    // reconstruction to go through even when LF_TYPESERVER_ST
                    // types are encountered. Alert the user that the result
                    // might be incomplete.
                    if let Err(err) = class.add_fields(
                        type_finder,
                        type_forwarder,
                        fields,
                        type_name_flavor,
                        needed_types,
                    ) {
                        log::error!(
                            "Error encountered while reconstructing '{}': {}",
                            class.name,
                            err
                        );
                    }
                }

                if data.properties.is_nested_type() {
                    self.nested_type_indices.insert(type_index);
                }
                self.type_names.insert(name);
                self.classes.insert(type_index, class);
            }

            pdb::TypeData::Union(data) => {
                let name_str = data.name.to_string();
                // Rename unnamed anonymous tags to something unique
                let name = if is_unnamed_type(&name_str) {
                    format!("_unnamed_{type_index}")
                } else {
                    name_str.into_owned()
                };

                if self.type_names.contains(&name) {
                    // Type has already been added, return
                    return Ok(());
                }

                let mut u = Union::new(type_index, name.clone(), data.size);

                if let Err(err) = u.add_fields(
                    type_finder,
                    type_forwarder,
                    data.fields,
                    type_name_flavor,
                    needed_types,
                ) {
                    log::error!(
                        "Error encountered while reconstructing '{}': {}",
                        u.name,
                        err
                    );
                }

                if data.properties.is_nested_type() {
                    self.nested_type_indices.insert(type_index);
                }
                self.type_names.insert(name);
                self.unions.insert(type_index, u);
            }

            pdb::TypeData::Enumeration(data) => {
                let name_str = data.name.to_string();
                // Rename unnamed anonymous tags to something unique
                let name = if is_unnamed_type(&name_str) {
                    format!("_unnamed_{type_index}")
                } else {
                    name_str.into_owned()
                };

                if self.type_names.contains(&name) {
                    // Type has already been added, return
                    return Ok(());
                }

                let mut e = Enum {
                    index: type_index,
                    name: name.clone(),
                    underlying_type_name: type_name(
                        type_finder,
                        type_forwarder,
                        data.underlying_type,
                        type_name_flavor,
                        needed_types,
                    )?
                    .0,
                    values: Vec::new(),
                    source_location: None,
                };

                if let Err(err) = e.add_fields(type_finder, data.fields, needed_types) {
                    log::error!(
                        "Error encountered while reconstructing '{}': {}",
                        e.name,
                        err
                    );
                }

                if data.properties.is_nested_type() {
                    self.nested_type_indices.insert(type_index);
                }
                self.type_names.insert(name.clone());
                self.enums.insert(type_index, e);
            }

            // ignore
            other => log::debug!("don't know how to add {:?}", other),
        }

        Ok(())
    }

    pub fn add_as_forward_declaration(
        &mut self,
        type_finder: &pdb::TypeFinder<'p>,
        type_index: pdb::TypeIndex,
    ) -> Result<()> {
        match type_finder.find(type_index)?.parse()? {
            pdb::TypeData::Class(data) => {
                let name_str = data.name.to_string();
                // Rename unnamed anonymous tags to something unique
                let name = if is_unnamed_type(&name_str) {
                    format!("_unnamed_{type_index}")
                } else {
                    name_str.into_owned()
                };

                if data.properties.is_nested_type() {
                    self.nested_type_indices.insert(type_index);
                }
                self.forward_declarations.insert(
                    type_index,
                    ForwardDeclaration {
                        index: type_index,
                        kind: ForwardDeclarationKind::from_class_kind(data.kind),
                        name,
                    },
                );
            }

            pdb::TypeData::Union(data) => {
                let name_str = data.name.to_string();
                // Rename unnamed anonymous tags to something unique
                let name = if is_unnamed_type(&name_str) {
                    format!("_unnamed_{type_index}")
                } else {
                    name_str.into_owned()
                };

                if data.properties.is_nested_type() {
                    self.nested_type_indices.insert(type_index);
                }
                self.forward_declarations.insert(
                    type_index,
                    ForwardDeclaration {
                        index: type_index,
                        kind: ForwardDeclarationKind::Union,
                        name,
                    },
                );
            }

            _ => {}
        }

        Ok(())
    }
}

/// Writes top-level declarations inside `namespace` blocks matching their
/// qualified names, opening and closing blocks when the namespace changes.
struct NamespaceScope<'n> {
    enabled: bool,
    /// Open a block per namespace component and keep the blocks of enclosing
    /// namespaces open, instead of a block per qualified namespace
    unify: bool,
    /// Indent the contents of each open block by one level
    indent: bool,
    /// Opening braces go on their own line
    brackets_new_line: bool,
    /// Types whose qualifier is their containing type, not a namespace
    nested_type_indices: &'n HashSet<pdb::TypeIndex>,
    /// Open blocks, with the namespace components each block opens
    open_blocks: Vec<Vec<String>>,
}

impl<'n> NamespaceScope<'n> {
    fn new(
        enabled: bool,
        unify: bool,
        indent: bool,
        brackets_new_line: bool,
        nested_type_indices: &'n HashSet<pdb::TypeIndex>,
    ) -> Self {
        Self {
            enabled,
            unify,
            indent,
            brackets_new_line,
            nested_type_indices,
            open_blocks: Vec::new(),
        }
    }

    /// Moves to the namespace of the type named `name`. Returns the name the
    /// type should be declared with there, and whether a namespace block was
    /// opened (preceded by a blank line).
    ///
    /// Types whose qualifier can't be written as a namespace (nested types and
    /// types declared in a function) keep their qualified name and are written
    /// outside of any namespace block.
    fn enter<'a>(
        &mut self,
        type_index: pdb::TypeIndex,
        name: &'a str,
        f: &mut IndentingWriter<'_, impl std::fmt::Write>,
    ) -> std::result::Result<(&'a str, bool), fmt::Error> {
        let (namespace, unqualified_name) =
            if self.enabled && !self.nested_type_indices.contains(&type_index) {
                match split_qualified_name(name) {
                    Some((namespace, unqualified_name)) if is_namespace_path(namespace) => {
                        (Some(namespace), unqualified_name)
                    }
                    _ => (None, name),
                }
            } else {
                (None, name)
            };
        let components: Vec<&str> = namespace.map_or_else(Vec::new, |n| n.split("::").collect());
        if self
            .open_blocks
            .iter()
            .flatten()
            .map(String::as_str)
            .eq(components.iter().copied())
        {
            return Ok((unqualified_name, false));
        }

        // Blocks of the enclosing namespaces stay open when unifying (each
        // block opens a single component then)
        let kept_block_count = if self.unify {
            self.open_blocks
                .iter()
                .zip(&components)
                .take_while(|(block, component)| block[0] == **component)
                .count()
        } else {
            0
        };
        while self.open_blocks.len() > kept_block_count {
            self.close_block(f)?;
        }
        let opened_components = &components[kept_block_count..];
        if opened_components.is_empty() {
            return Ok((unqualified_name, false));
        }
        writeln!(f)?;
        if self.unify {
            for component in opened_components {
                self.open_block(vec![(*component).to_owned()], f)?;
            }
        } else {
            self.open_block(
                opened_components.iter().map(|c| (*c).to_owned()).collect(),
                f,
            )?;
        }

        Ok((unqualified_name, true))
    }

    /// Closes every open namespace block.
    fn close(&mut self, f: &mut IndentingWriter<'_, impl std::fmt::Write>) -> fmt::Result {
        while !self.open_blocks.is_empty() {
            self.close_block(f)?;
        }
        Ok(())
    }

    fn open_block(
        &mut self,
        components: Vec<String>,
        f: &mut IndentingWriter<'_, impl std::fmt::Write>,
    ) -> fmt::Result {
        let brace_separator = if self.brackets_new_line { "\n" } else { " " };
        if components.iter().any(|c| c == ANONYMOUS_NAMESPACE) {
            // `namespace a::b` can't name an anonymous namespace
            let openings: Vec<String> = components
                .iter()
                .map(|component| {
                    if component == ANONYMOUS_NAMESPACE {
                        format!("namespace{brace_separator}{{")
                    } else {
                        format!("namespace {component}{brace_separator}{{")
                    }
                })
                .collect();
            writeln!(f, "{}", openings.join(brace_separator))?;
        } else {
            writeln!(f, "namespace {}{brace_separator}{{", components.join("::"))?;
        }
        self.open_blocks.push(components);
        self.update_indentation_level(f);
        Ok(())
    }

    /// Closes the innermost open block, commented with the namespace it ends.
    fn close_block(&mut self, f: &mut IndentingWriter<'_, impl std::fmt::Write>) -> fmt::Result {
        let Some(components) = self.open_blocks.pop() else {
            return Ok(());
        };
        self.update_indentation_level(f);
        let brace_count = if components.iter().any(|c| c == ANONYMOUS_NAMESPACE) {
            components.len()
        } else {
            1
        };
        let namespace = self
            .open_blocks
            .iter()
            .flatten()
            .chain(&components)
            .map(|component| {
                if component == ANONYMOUS_NAMESPACE {
                    "(anonymous)"
                } else {
                    component.as_str()
                }
            })
            .collect::<Vec<_>>()
            .join("::");
        writeln!(f, "{} // namespace {namespace}", "}".repeat(brace_count))
    }

    fn update_indentation_level(&self, f: &mut IndentingWriter<'_, impl std::fmt::Write>) {
        if self.indent {
            f.level = self.open_blocks.len();
        }
    }
}

/// Writes text with `level` indentations at the start of each non-empty line
struct IndentingWriter<'w, W: std::fmt::Write> {
    inner: &'w mut W,
    indentation: &'static str,
    level: usize,
    at_line_start: bool,
}

impl<'w, W: std::fmt::Write> IndentingWriter<'w, W> {
    fn new(inner: &'w mut W, indentation: &'static str) -> Self {
        Self {
            inner,
            indentation,
            level: 0,
            at_line_start: true,
        }
    }
}

impl<W: std::fmt::Write> std::fmt::Write for IndentingWriter<'_, W> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for line in s.split_inclusive('\n') {
            if self.at_line_start && line != "\n" {
                for _ in 0..self.level {
                    self.inner.write_str(self.indentation)?;
                }
            }
            self.inner.write_str(line)?;
            self.at_line_start = line.ends_with('\n');
        }
        Ok(())
    }
}

/// Name MSVC gives to anonymous namespaces in qualified type names (verified
/// with MSVC 14.51 PDBs)
const ANONYMOUS_NAMESPACE: &str = "`anonymous-namespace'";

/// Splits a qualified name at its last `::` that isn't part of template
/// arguments, returning the qualifier and the unqualified name.
pub(crate) fn split_qualified_name(name: &str) -> Option<(&str, &str)> {
    let mut depth: usize = 0;
    let mut split_position = None;
    let bytes = name.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'<' | b'(' | b'[' => depth += 1,
            b'>' | b')' | b']' => depth = depth.saturating_sub(1),
            b':' if depth == 0 && bytes.get(i + 1) == Some(&b':') => {
                split_position = Some(i);
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }

    split_position.map(|position| (&name[..position], &name[position + 2..]))
}

/// Indicates if every component of `qualifier` can be written as a C++
/// namespace name.
///
/// Types declared in a function are qualified with the function's name and a
/// lexical block component (e.g., `Function::__l2::Type`), which isn't a
/// namespace.
fn is_namespace_path(qualifier: &str) -> bool {
    qualifier.split("::").all(|component| {
        let is_lexical_block = component
            .strip_prefix("__l")
            .is_some_and(|block| !block.is_empty() && block.bytes().all(|c| c.is_ascii_digit()));
        let is_identifier = component
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && component
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_');
        component == ANONYMOUS_NAMESPACE || (is_identifier && !is_lexical_block)
    })
}

/// Returns the access specifier to write before a member with the given
/// `access`.
///
/// With `Automatic`, a specifier is only written when `access` differs from
/// the access in effect (`current_access`, initialized with the type's
/// default access), since a C++ access specifier applies to every member
/// that follows it. Members of unnamed unions/structs (`current_access` is
/// `None`) get the access written before the unnamed type. Members whose
/// access isn't recorded (`FieldAccess::None`, e.g., `LF_NESTTYPE`) don't
/// change the access in effect.
fn access_specifier(
    fmt_configuration: &DataFormatConfiguration,
    access: FieldAccess,
    current_access: Option<&Cell<FieldAccess>>,
) -> FieldAccess {
    match fmt_configuration.print_access_specifiers {
        AccessSpecifierReconstructionFlavor::Disabled => FieldAccess::None,
        AccessSpecifierReconstructionFlavor::Always => access,
        AccessSpecifierReconstructionFlavor::Automatic => match current_access {
            Some(current_access)
                if access != FieldAccess::None && current_access.get() != access =>
            {
                current_access.set(access);
                access
            }
            _ => FieldAccess::None,
        },
    }
}

/// Access specifier to write before an unnamed union/struct, whose members
/// get the access of its first member. Only needed with `Automatic`, the
/// other flavors write specifiers on the unnamed type's members.
fn unnamed_type_access_specifier(
    fmt_configuration: &DataFormatConfiguration,
    first_member_access: FieldAccess,
    current_access: Option<&Cell<FieldAccess>>,
) -> FieldAccess {
    match fmt_configuration.print_access_specifiers {
        AccessSpecifierReconstructionFlavor::Automatic => {
            access_specifier(fmt_configuration, first_member_access, current_access)
        }
        _ => FieldAccess::None,
    }
}

pub fn resolve_complete_type_index(
    forwarder_to_complete_type: &dashmap::DashMap<pdb::TypeIndex, pdb::TypeIndex>,
    type_index: pdb::TypeIndex,
) -> pdb::TypeIndex {
    match forwarder_to_complete_type.get(&type_index) {
        Some(d) => *d.value(),
        None => type_index,
    }
}

fn fmt_struct_fields_recursive<W: std::fmt::Write>(
    fmt_configuration: &DataFormatConfiguration,
    fields: &[Field],
    depth: usize,
    current_access: Option<&Cell<FieldAccess>>,
    f: &mut W,
) -> fmt::Result {
    fmt_struct_fields_interleaved(
        fmt_configuration,
        fields,
        depth,
        current_access,
        f,
        |_, _| Ok(()),
    )
}

/// Writes struct fields, calling `before_group` with the index of the first
/// field of each top-level group (a field or an unnamed union) before writing
/// that group. Used to interleave other members in PDB declaration order.
///
/// `current_access` is the access in effect for the type's members (see
/// `access_specifier`), `None` inside unnamed unions/structs.
fn fmt_struct_fields_interleaved<W: std::fmt::Write>(
    fmt_configuration: &DataFormatConfiguration,
    fields: &[Field],
    depth: usize,
    current_access: Option<&Cell<FieldAccess>>,
    f: &mut W,
    mut before_group: impl FnMut(usize, &mut W) -> fmt::Result,
) -> fmt::Result {
    if fields.is_empty() {
        return Ok(());
    }

    let unions_found = find_unnamed_unions_in_struct(fields);
    // Write fields into the `Formatter`
    let indentation = fmt_configuration.indent().repeat(depth);
    let mut last_field: Option<&Field> = None;
    for union_range in unions_found {
        before_group(union_range.start, f)?;
        // Fields out of unnamed unions are represented by "empty" unions
        if union_range.is_empty() {
            let field = &fields[union_range.start];

            // Check if we need to add padding, following a bit-field member
            if let Some((field_bit_offset, _)) = field.bitfield_info {
                if let Some(last_field) = last_field {
                    if let Some((last_bit_offset, last_bit_size)) = last_field.bitfield_info {
                        let potential_padding_bit_offset = last_bit_offset + last_bit_size;
                        if field.offset == last_field.offset {
                            // Padding within the same allocation unit
                            let bit_offset_delta = field_bit_offset - potential_padding_bit_offset;
                            // Add padding if needed
                            if bit_offset_delta > 0 {
                                write!(f, "{}", &indentation)?;
                                if fmt_configuration.print_offset_info {
                                    write!(f, "/* {:#06x} */ ", last_field.offset)?;
                                }
                                write!(f, "{} : {};", last_field.type_left, bit_offset_delta)?;
                                if fmt_configuration.print_offset_info {
                                    write!(f, " /* BitPos={potential_padding_bit_offset} */")?;
                                }
                                writeln!(f)?;
                            }
                        } else {
                            // Padding in the previous field
                            // FIXME(ergrelet): 0-bit padding is used systematically when we should only emit it when
                            // needed. It's not incorrect but might produce less elegant output.
                            write!(f, "{}", &indentation)?;
                            if fmt_configuration.print_offset_info {
                                write!(f, "/* {:#06x} */ ", last_field.offset)?;
                            }
                            write!(f, "{} : 0;", last_field.type_left)?;
                            if fmt_configuration.print_offset_info {
                                write!(f, " /* BitPos={potential_padding_bit_offset} */")?;
                            }
                            writeln!(f)?;
                        }
                    }
                }
            }

            write!(f, "{}", &indentation)?;
            if fmt_configuration.print_offset_info {
                write!(f, "/* {:#06x} */ ", field.offset)?;
            }
            write!(
                f,
                "{}",
                access_specifier(fmt_configuration, field.access, current_access)
            )?;
            write!(
                f,
                "{} {}{};",
                field.type_left,
                field.name.to_string(),
                field.type_right
            )?;
            if fmt_configuration.print_offset_info {
                if let Some((bit_position, _)) = field.bitfield_info {
                    write!(f, " /* BitPos={bit_position} */")?;
                }
            }
            writeln!(f)?;
            last_field = Some(field);
        } else {
            writeln!(
                f,
                "{}{}union{}{{",
                &indentation,
                unnamed_type_access_specifier(
                    fmt_configuration,
                    fields[union_range.start].access,
                    current_access
                ),
                if fmt_configuration.print_brackets_new_line {
                    format!("\n{}", &indentation)
                } else {
                    String::from(" ")
                }
            )?;
            fmt_union_fields_recursive(
                fmt_configuration,
                &fields[union_range],
                depth + 1,
                None,
                f,
            )?;
            writeln!(f, "{}}};", &indentation)?;
            last_field = None;
        }
    }

    Ok(())
}

fn find_unnamed_unions_in_struct(fields: &[Field]) -> Vec<Range<usize>> {
    let mut unions_found: Vec<Range<usize>> = vec![];
    // Temporary map of unions and fields that'll be used to compute the list
    // of unnamed unions which are in the struct.
    let mut unions_found_temp: BTreeMap<(u64, u8), (Range<usize>, u64)> = BTreeMap::new();

    // Discover unions
    let mut curr_union_offset_range: Range<u64> = Range::default();
    for (i, field) in fields.iter().enumerate() {
        // Check if the field is located inside of the union we're processing
        if curr_union_offset_range.contains(&field.offset) {
            // Third step of the "state machine", add new fields to the union.
            let union_info = unions_found_temp
                .get_mut(&(curr_union_offset_range.start, 0))
                .expect("key should exist in map");
            union_info.0.end = i + 1;
            // Update the union's size
            union_info.1 = std::cmp::max(
                union_info.1,
                field.offset - curr_union_offset_range.start + field.size as u64,
            );
            curr_union_offset_range.end = std::cmp::max(
                curr_union_offset_range.end,
                field.offset + field.size as u64,
            );
            // (Re)visit previous fields to compute the union's size
            // as well as the current union's end
            for previous_field in &fields[union_info.0.clone()] {
                // Update the union's size
                if previous_field.offset > field.offset {
                    union_info.1 = std::cmp::max(
                        union_info.1,
                        previous_field.offset - field.offset + previous_field.size as u64,
                    );
                } else {
                    union_info.1 = std::cmp::max(union_info.1, previous_field.size as u64);
                }
                curr_union_offset_range.end = std::cmp::max(
                    curr_union_offset_range.end,
                    previous_field.offset + previous_field.size as u64,
                );
            }
        } else {
            match unions_found_temp
                .get_mut(&(field.offset, field.bitfield_info.unwrap_or_default().0))
            {
                Some(union_info) => {
                    // Second step of the "state machine", two fields share the
                    // same offset (taking bitfields into account). This becomes
                    // a union (the current one).
                    union_info.0.end = i + 1;
                    curr_union_offset_range.start = field.offset;
                    // (Re)visit previous fields to compute the union's size
                    // as well as the current union's end
                    for previous_field in &fields[union_info.0.clone()] {
                        // Update the union's size
                        if previous_field.offset > field.offset {
                            union_info.1 = std::cmp::max(
                                union_info.1,
                                previous_field.offset - field.offset + previous_field.size as u64,
                            );
                        } else {
                            union_info.1 = std::cmp::max(union_info.1, previous_field.size as u64);
                        }
                        curr_union_offset_range.end = std::cmp::max(
                            curr_union_offset_range.end,
                            previous_field.offset + previous_field.size as u64,
                        );
                    }
                }
                None => {
                    // First step of the "state machine".
                    // Each field is a potential new union
                    unions_found_temp.insert(
                        (field.offset, field.bitfield_info.unwrap_or_default().0),
                        (Range { start: i, end: i }, field.size as u64),
                    );
                }
            }
        }
    }

    // Remove nested unions, they will be processed when we'll go deeper
    for (offset1, range1) in unions_found_temp.iter() {
        let mut is_top_level = true;
        for (offset2, range2) in unions_found_temp.iter() {
            if offset1 == offset2 {
                // Comparing a union with itself, ignore
                continue;
            }
            if range1.0.start >= range2.0.start && range1.0.end < range2.0.end {
                // Union #1 is contained by Union #2, no need to continue.
                // Union #1 isn't a "top-level" union.
                is_top_level = false;
                break;
            }
        }
        if is_top_level {
            // Only keep "top-level" union
            unions_found.push(range1.0.clone());
        }
    }

    unions_found
}

fn fmt_union_fields_recursive<W: std::fmt::Write>(
    fmt_configuration: &DataFormatConfiguration,
    fields: &[Field],
    depth: usize,
    current_access: Option<&Cell<FieldAccess>>,
    f: &mut W,
) -> fmt::Result {
    fmt_union_fields_interleaved(
        fmt_configuration,
        fields,
        depth,
        current_access,
        f,
        |_, _| Ok(()),
    )
}

/// Union counterpart of `fmt_struct_fields_interleaved`: `before_group` is
/// called with the index of the first field of each top-level group (a field
/// or an unnamed struct).
fn fmt_union_fields_interleaved<W: std::fmt::Write>(
    fmt_configuration: &DataFormatConfiguration,
    fields: &[Field],
    depth: usize,
    current_access: Option<&Cell<FieldAccess>>,
    f: &mut W,
    mut before_group: impl FnMut(usize, &mut W) -> fmt::Result,
) -> fmt::Result {
    if fields.is_empty() {
        return Ok(());
    }

    let structs_found = find_unnamed_structs_in_unions(fields);
    let indentation = fmt_configuration.indent().repeat(depth);
    for struct_range in structs_found {
        before_group(struct_range.start, f)?;
        // Fields out of unnamed structs are represented by "empty" structs
        if struct_range.is_empty() {
            let field = &fields[struct_range.start];
            write!(f, "{}", &indentation)?;
            if fmt_configuration.print_offset_info {
                write!(f, "/* {:#06x} */ ", field.offset)?;
            }
            write!(
                f,
                "{}",
                access_specifier(fmt_configuration, field.access, current_access)
            )?;
            write!(
                f,
                "{} {}{};",
                field.type_left,
                field.name.to_string(),
                field.type_right
            )?;
            if fmt_configuration.print_offset_info {
                if let Some((bit_position, _)) = field.bitfield_info {
                    write!(f, " /* BitPos={bit_position} */")?;
                }
            }
            writeln!(f)?;
        } else {
            writeln!(
                f,
                "{}{}struct{}{{",
                &indentation,
                unnamed_type_access_specifier(
                    fmt_configuration,
                    fields[struct_range.start].access,
                    current_access
                ),
                if fmt_configuration.print_brackets_new_line {
                    format!("\n{}", &indentation)
                } else {
                    String::from(" ")
                }
            )?;
            fmt_struct_fields_recursive(
                fmt_configuration,
                &fields[struct_range],
                depth + 1,
                None,
                f,
            )?;
            writeln!(f, "{}}};", &indentation)?;
        }
    }

    Ok(())
}

fn find_unnamed_structs_in_unions(fields: &[Field]) -> Vec<Range<usize>> {
    let mut structs_found: Vec<Range<usize>> = vec![];

    let field_count = fields.len();
    let union_offset = fields[0].offset;
    let mut previous_field_offset = fields[0].offset;
    let mut previous_field_bit_offset = fields[0].bitfield_info.unwrap_or_default().0;
    for (i, field) in fields.iter().enumerate() {
        // The field offset is lower than the offset of the previous field
        // -> "close" the struct
        if previous_field_offset > field.offset
            || (field.offset == previous_field_offset
                && previous_field_bit_offset > field.bitfield_info.unwrap_or_default().0)
        {
            if let Some(last_found_struct_range) = structs_found.pop() {
                // "Merge" previous field with the struct
                structs_found.push(Range {
                    start: last_found_struct_range.start,
                    end: i,
                });
            }
        }
        // Last element, check if we need to "close" a struct
        else if i == field_count - 1 {
            if let Some(last_found_struct_range) = structs_found.pop() {
                // Declare a new struct only if its length is greater than 1.
                if i > last_found_struct_range.start
                    && (field.offset != previous_field_offset
                        || field.bitfield_info.unwrap_or_default().0 != previous_field_bit_offset)
                {
                    // "Merge" previous field with the struct
                    structs_found.push(Range {
                        start: last_found_struct_range.start,
                        end: i + 1,
                    });
                } else {
                    structs_found.push(last_found_struct_range);
                }
            }
        }

        // Regular field, may be the beginning of a struct
        if field.offset == union_offset && field.bitfield_info.unwrap_or_default().0 == 0 {
            structs_found.push(Range { start: i, end: i });
        }

        previous_field_offset = field.offset;
        previous_field_bit_offset = field.bitfield_info.unwrap_or_default().0;
    }

    structs_found
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataFormatConfiguration {
    pub print_access_specifiers: AccessSpecifierReconstructionFlavor,
    pub size_print_flavor: SizePrintFlavor,
    pub integers_as_hexadecimal: bool,
    pub ignore_compiler_generated_methods: bool,
    /// Print fields and methods in their PDB declaration order instead of
    /// grouping fields before methods.
    pub preserve_member_order: bool,
    /// Wrap types in `namespace` blocks instead of declaring them with their
    /// qualified names.
    pub reconstruct_namespaces: bool,
    /// Keep namespace blocks open across definitions and nest the blocks of
    /// inner namespaces inside the blocks of enclosing ones.
    pub unify_namespaces: bool,
    /// Indent the contents of namespace blocks.
    pub indent_namespaces: bool,
    /// When types are written with their dependencies, write the types of a
    /// namespace one after the other when their dependencies allow it.
    pub group_namespaces: bool,
    /// Print `override` on virtual methods overriding a base class method.
    pub print_override_specifiers: bool,
    /// Don't print `virtual` on methods printed with `override`.
    pub omit_virtual_on_overrides: bool,
    /// Print the RVA of methods implemented in the PDB.
    pub print_method_rvas: bool,
    /// Print the calling convention of methods.
    pub print_calling_conventions: bool,
    pub vftable_comment_flavor: VftableCommentFlavor,
    /// Don't print the `/* offset: fields for Base */` comments.
    pub hide_base_class_offsets: bool,
    pub indentation: IndentationFlavor,
    /// Don't separate groups of members with blank lines.
    pub compact_output: bool,
    pub print_offset_info: bool,
    pub print_brackets_new_line: bool,
    /// Print where types are defined (source file and line) when known.
    pub print_source_locations: bool,
}

impl Default for DataFormatConfiguration {
    fn default() -> Self {
        Self {
            print_access_specifiers: AccessSpecifierReconstructionFlavor::Always,
            size_print_flavor: SizePrintFlavor::Comment,
            integers_as_hexadecimal: true,
            ignore_compiler_generated_methods: true,
            preserve_member_order: true,
            reconstruct_namespaces: true,
            unify_namespaces: false,
            indent_namespaces: false,
            group_namespaces: false,
            print_override_specifiers: true,
            omit_virtual_on_overrides: false,
            print_method_rvas: true,
            print_calling_conventions: false,
            vftable_comment_flavor: VftableCommentFlavor::Offset,
            hide_base_class_offsets: false,
            indentation: IndentationFlavor::TwoSpaces,
            compact_output: false,
            print_offset_info: true,
            print_brackets_new_line: false,
            print_source_locations: false,
        }
    }
}

impl DataFormatConfiguration {
    /// One level of indentation
    pub fn indent(&self) -> &'static str {
        self.indentation.as_str()
    }

    /// Writes the blank line separating groups of members, unless the output
    /// is compact. The line holds one level of indentation.
    pub fn fmt_separator(&self, f: &mut impl std::fmt::Write) -> fmt::Result {
        if self.compact_output {
            Ok(())
        } else {
            writeln!(f, "{}", self.indent())
        }
    }

    /// Writes the comment telling where a type is defined, when requested
    /// and known.
    pub fn fmt_source_location(
        &self,
        source_location: Option<&str>,
        f: &mut impl std::fmt::Write,
    ) -> fmt::Result {
        match source_location {
            Some(source_location) if self.print_source_locations => {
                writeln!(f, "// {source_location}")
            }
            _ => Ok(()),
        }
    }
}

/// Reference to a class or union member, used to record the order in which
/// members appear in the PDB field list. Each variant holds an index into the
/// matching member vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberRef {
    NestedType(usize),
    Field(usize),
    StaticField(usize),
    InstanceMethod(usize),
    StaticMethod(usize),
}

/// Kind of a run of members printed in PDB order. Runs of different kinds are
/// separated by a blank line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MemberKind {
    NestedType,
    Data,
    Method,
}

impl MemberRef {
    fn kind(self) -> MemberKind {
        match self {
            MemberRef::NestedType(_) => MemberKind::NestedType,
            MemberRef::Field(_) | MemberRef::StaticField(_) => MemberKind::Data,
            MemberRef::InstanceMethod(_) | MemberRef::StaticMethod(_) => MemberKind::Method,
        }
    }
}
