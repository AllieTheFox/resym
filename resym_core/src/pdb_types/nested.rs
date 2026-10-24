use std::{
    cell::Cell,
    fmt::{self, Write as _},
};

use super::{
    access_specifier,
    class::Class,
    enumeration::Enum,
    field::FieldAccess,
    forward_declaration::{ForwardDeclaration, ForwardDeclarationKind},
    is_unnamed_type, resolve_complete_type_index, type_name, type_source_location,
    union::Union,
    DataFormatConfiguration, MethodImplementations, NeededTypeSet, ReconstructibleTypeData,
    TypeForwarder, TypeNameFlavor, TypeSourceLocations,
};
use crate::error::Result;

/// Declaration found in a class or union's field list (`LF_NESTTYPE`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NestedDeclaration<'p> {
    pub access: FieldAccess,
    pub kind: NestedDeclarationKind<'p>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NestedDeclarationKind<'p> {
    /// Nested class/struct definition, reconstructed under its unqualified `name`
    Class { name: String, class: Class<'p> },
    /// Nested union definition, reconstructed under its unqualified `name`
    Union { name: String, union: Union<'p> },
    /// Nested enum definition, reconstructed under its unqualified `name`
    Enum { name: String, enumeration: Enum<'p> },
    /// Nested class/struct/union whose definition isn't in the PDB
    ForwardDeclaration {
        full_name: String,
        declaration: ForwardDeclaration,
    },
    /// Type alias (`typedef`/`using`) declared inside the parent type
    Typedef {
        name: String,
        type_left: String,
        type_right: String,
    },
}

impl<'p> NestedDeclaration<'p> {
    /// Parses an `LF_NESTTYPE` record of the type named `parent_name`.
    ///
    /// The record is a nested type definition when the referenced type is
    /// named `parent_name::name`. Otherwise, it is an alias (MSVC also emits
    /// `LF_NESTTYPE` for `typedef`/`using` declarations).
    ///
    /// Returns `None` for unnamed types (e.g., the type of `struct { ... } s;`),
    /// which are reconstructed as separate `_unnamed_` types.
    pub fn parse(
        parent_name: &str,
        nested: &pdb::NestedType<'p>,
        type_finder: &pdb::TypeFinder<'p>,
        type_forwarder: &TypeForwarder,
        type_name_flavor: &TypeNameFlavor,
        needed_types: &mut NeededTypeSet,
    ) -> Result<Option<Self>> {
        let name = nested.name.to_string().into_owned();
        if is_unnamed_type(&name) {
            return Ok(None);
        }

        let full_name = format!("{parent_name}::{name}");
        let type_index = resolve_complete_type_index(type_forwarder, nested.nested_type);
        let kind = match type_finder.find(type_index)?.parse()? {
            pdb::TypeData::Class(data) if data.name.to_string() == full_name => {
                if data.properties.forward_reference() {
                    NestedDeclarationKind::ForwardDeclaration {
                        declaration: ForwardDeclaration {
                            index: type_index,
                            kind: ForwardDeclarationKind::from_class_kind(data.kind),
                            name,
                        },
                        full_name,
                    }
                } else {
                    let mut class = Class::new(type_index, data.kind, full_name, data.size);
                    if let Some(fields) = data.fields {
                        class.add_fields(
                            type_finder,
                            type_forwarder,
                            fields,
                            type_name_flavor,
                            needed_types,
                        )?;
                    }
                    NestedDeclarationKind::Class { name, class }
                }
            }

            pdb::TypeData::Union(data) if data.name.to_string() == full_name => {
                if data.properties.forward_reference() {
                    NestedDeclarationKind::ForwardDeclaration {
                        declaration: ForwardDeclaration {
                            index: type_index,
                            kind: ForwardDeclarationKind::Union,
                            name,
                        },
                        full_name,
                    }
                } else {
                    let mut union = Union::new(type_index, full_name, data.size);
                    union.add_fields(
                        type_finder,
                        type_forwarder,
                        data.fields,
                        type_name_flavor,
                        needed_types,
                    )?;
                    NestedDeclarationKind::Union { name, union }
                }
            }

            pdb::TypeData::Enumeration(data) if data.name.to_string() == full_name => {
                let mut enumeration = Enum {
                    index: type_index,
                    name: full_name,
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
                enumeration.add_fields(type_finder, data.fields, needed_types)?;
                NestedDeclarationKind::Enum { name, enumeration }
            }

            _ => {
                let (type_left, type_right) = type_name(
                    type_finder,
                    type_forwarder,
                    type_index,
                    type_name_flavor,
                    needed_types,
                )?;
                NestedDeclarationKind::Typedef {
                    name,
                    type_left,
                    type_right,
                }
            }
        };

        Ok(Some(Self {
            access: FieldAccess::from_field_attribute(nested.attributes.access()),
            kind,
        }))
    }

    /// Sets what is known about the implementations of the methods of the
    /// nested type.
    pub fn assign_method_implementations(
        &mut self,
        implementations: &MethodImplementations,
        parameter_names: bool,
        rvas: bool,
    ) {
        match &mut self.kind {
            NestedDeclarationKind::Class { class, .. } => {
                class.assign_method_implementations(implementations, parameter_names, rvas)
            }
            NestedDeclarationKind::Union { union, .. } => {
                union.assign_method_implementations(implementations, parameter_names, rvas)
            }
            _ => {}
        }
    }

    /// Sets where the nested type (and the types declared inside it) is
    /// defined.
    pub fn assign_source_locations(
        &mut self,
        locations: &TypeSourceLocations,
        type_forwarder: &TypeForwarder,
    ) {
        match &mut self.kind {
            NestedDeclarationKind::Class { class, .. } => {
                class.assign_source_locations(locations, type_forwarder)
            }
            NestedDeclarationKind::Union { union, .. } => {
                union.assign_source_locations(locations, type_forwarder)
            }
            NestedDeclarationKind::Enum { enumeration, .. } => {
                enumeration.source_location =
                    type_source_location(locations, type_forwarder, enumeration.index)
            }
            _ => {}
        }
    }

    /// Calls `f` with the fully qualified name of this declaration and of
    /// every type declared inside it. Typedefs aren't types and are skipped.
    pub fn visit_type_names(&self, f: &mut impl FnMut(&str)) {
        match &self.kind {
            NestedDeclarationKind::Class { class, .. } => {
                f(&class.name);
                for nested in &class.nested_types {
                    nested.visit_type_names(f);
                }
            }
            NestedDeclarationKind::Union { union, .. } => {
                f(&union.name);
                for nested in &union.nested_types {
                    nested.visit_type_names(f);
                }
            }
            NestedDeclarationKind::Enum { enumeration, .. } => f(&enumeration.name),
            NestedDeclarationKind::ForwardDeclaration { full_name, .. } => f(full_name),
            NestedDeclarationKind::Typedef { .. } => {}
        }
    }

    /// Writes the declaration one indentation level deeper than its parent's
    /// members. `current_access` is the access in effect in the parent (see
    /// `access_specifier`).
    pub fn reconstruct(
        &self,
        fmt_configuration: &DataFormatConfiguration,
        current_access: &Cell<FieldAccess>,
        f: &mut impl std::fmt::Write,
    ) -> fmt::Result {
        let mut declaration = String::new();
        match &self.kind {
            NestedDeclarationKind::Class { name, class } => {
                class.reconstruct_as(name, fmt_configuration, &mut declaration)?
            }
            NestedDeclarationKind::Union { name, union } => {
                union.reconstruct_as(name, fmt_configuration, &mut declaration)?
            }
            NestedDeclarationKind::Enum { name, enumeration } => {
                enumeration.reconstruct_as(name, fmt_configuration, &mut declaration)?
            }
            NestedDeclarationKind::ForwardDeclaration {
                declaration: forward_declaration,
                ..
            } => forward_declaration.reconstruct(fmt_configuration, &mut declaration)?,
            NestedDeclarationKind::Typedef {
                name,
                type_left,
                type_right,
            } => writeln!(declaration, "typedef {type_left} {name}{type_right};")?,
        }

        let access = access_specifier(fmt_configuration, self.access, Some(current_access));
        let indent = fmt_configuration.indent();
        for (i, line) in declaration.lines().enumerate() {
            if i == 0 {
                writeln!(f, "{indent}{access}{line}")?;
            } else {
                writeln!(f, "{indent}{line}")?;
            }
        }

        Ok(())
    }
}
