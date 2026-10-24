use std::{cell::Cell, fmt};

use super::{
    access_specifier,
    field::{FieldAccess, StaticField},
    fmt_union_fields_interleaved, fmt_union_fields_recursive,
    nested::NestedDeclaration,
    primitive_types::SizePrintFlavor,
    resolve_complete_type_index, type_bitfield_info, type_name, type_size, type_source_location,
    DataFormatConfiguration, Field, MemberKind, MemberRef, Method, MethodImplementations,
    NeededTypeSet, ReconstructibleTypeData, TypeForwarder, TypeNameFlavor, TypeSourceLocations,
};
use crate::error::{Result, ResymCoreError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Union<'p> {
    pub index: pdb::TypeIndex,
    pub name: String,
    pub size: u64,
    pub fields: Vec<Field<'p>>,
    pub static_fields: Vec<StaticField<'p>>,
    pub instance_methods: Vec<Method<'p>>,
    pub static_methods: Vec<Method<'p>>,
    /// Fields and methods in PDB declaration order
    pub members: Vec<MemberRef>,
    /// Types and typedefs declared inside the union, in PDB order
    pub nested_types: Vec<NestedDeclaration<'p>>,
    /// Where the union is defined (`file(line)`), if known
    pub source_location: Option<String>,
}

impl<'p> Union<'p> {
    pub fn new(index: pdb::TypeIndex, name: String, size: u64) -> Self {
        Self {
            index,
            name,
            size,
            fields: Vec::new(),
            static_fields: Vec::new(),
            instance_methods: Vec::new(),
            static_methods: Vec::new(),
            members: Vec::new(),
            nested_types: Vec::new(),
            source_location: None,
        }
    }

    pub fn add_fields(
        &mut self,
        type_finder: &pdb::TypeFinder<'p>,
        type_forwarder: &TypeForwarder,
        type_index: pdb::TypeIndex,
        type_name_flavor: &TypeNameFlavor,
        needed_types: &mut NeededTypeSet,
    ) -> Result<()> {
        // Resolve the complete type's index, if present in the PDB
        let complete_type_index = resolve_complete_type_index(type_forwarder, type_index);
        match type_finder.find(complete_type_index)?.parse()? {
            pdb::TypeData::FieldList(data) => {
                for field in &data.fields {
                    self.add_field(
                        type_finder,
                        type_forwarder,
                        field,
                        type_name_flavor,
                        needed_types,
                    )?;
                }

                if let Some(continuation) = data.continuation {
                    // recurse
                    self.add_fields(
                        type_finder,
                        type_forwarder,
                        continuation,
                        type_name_flavor,
                        needed_types,
                    )?;
                }
            }

            pdb::TypeData::Primitive(_) | pdb::TypeData::Pointer(_) => {
                // TODO: What does this represent?
            }

            other => {
                log::warn!(
                    "trying to Union::add_fields() got {} -> {:?}",
                    type_index,
                    other
                );
            }
        }

        Ok(())
    }

    fn add_field(
        &mut self,
        type_finder: &pdb::TypeFinder<'p>,
        type_forwarder: &TypeForwarder,
        field: &pdb::TypeData<'p>,
        type_name_flavor: &TypeNameFlavor,
        needed_types: &mut NeededTypeSet,
    ) -> Result<()> {
        match field {
            pdb::TypeData::Member(data) => {
                // Resolve the complete type's index, if present in the PDB
                let complete_type_index =
                    resolve_complete_type_index(type_forwarder, data.field_type);
                let (type_left, type_right) = type_name(
                    type_finder,
                    type_forwarder,
                    complete_type_index,
                    type_name_flavor,
                    needed_types,
                )?;
                let type_bitfield_info = type_bitfield_info(type_finder, complete_type_index)?;
                let type_size = type_size(type_finder, complete_type_index)?;
                let access = FieldAccess::from_field_attribute(data.attributes.access());

                // TODO: attributes (static, virtual, etc.)
                self.members.push(MemberRef::Field(self.fields.len()));
                self.fields.push(Field {
                    type_left,
                    type_right,
                    name: data.name,
                    offset: data.offset,
                    size: type_size,
                    bitfield_info: type_bitfield_info,
                    access,
                });
            }

            pdb::TypeData::StaticMember(data) => {
                let complete_type_index =
                    resolve_complete_type_index(type_forwarder, data.field_type);
                let (type_left, type_right) = type_name(
                    type_finder,
                    type_forwarder,
                    complete_type_index,
                    type_name_flavor,
                    needed_types,
                )?;
                let access = FieldAccess::from_field_attribute(data.attributes.access());

                self.members
                    .push(MemberRef::StaticField(self.static_fields.len()));
                self.static_fields.push(StaticField {
                    type_left,
                    type_right,
                    name: data.name,
                    access,
                });
            }

            pdb::TypeData::Method(data) => {
                let method = Method::find(
                    data.name,
                    data.attributes,
                    type_finder,
                    type_forwarder,
                    data.method_type,
                    type_name_flavor,
                    needed_types,
                )?;
                self.push_method(method, data.attributes.is_static());
            }

            // Nested type declaration
            pdb::TypeData::Nested(data) => {
                if let Some(nested) = NestedDeclaration::parse(
                    &self.name,
                    data,
                    type_finder,
                    type_forwarder,
                    type_name_flavor,
                    needed_types,
                )? {
                    self.members
                        .push(MemberRef::NestedType(self.nested_types.len()));
                    self.nested_types.push(nested);
                }
            }

            pdb::TypeData::OverloadedMethod(data) => {
                // this just means we have more than one method with the same name
                // find the method list
                match type_finder.find(data.method_list)?.parse()? {
                    pdb::TypeData::MethodList(method_list) => {
                        for pdb::MethodListEntry {
                            attributes,
                            method_type,
                            ..
                        } in method_list.methods
                        {
                            // hooray
                            let method = Method::find(
                                data.name,
                                attributes,
                                type_finder,
                                type_forwarder,
                                method_type,
                                type_name_flavor,
                                needed_types,
                            )?;

                            self.push_method(method, attributes.is_static());
                        }
                    }
                    other => {
                        log::error!(
                            "processing OverloadedMethod, expected MethodList, got {} -> {:?}",
                            data.method_list,
                            other
                        );
                        return Err(ResymCoreError::InvalidParameterError(
                            "unexpected type in Class::add_field()".to_string(),
                        ));
                    }
                }
            }

            other => {
                log::error!("trying to Union::add_field(): {:?}", other);
                return Err(ResymCoreError::InvalidParameterError(
                    "unexpected type in Union::add_field()".to_string(),
                ));
            }
        }

        Ok(())
    }
}

impl ReconstructibleTypeData for Union<'_> {
    fn reconstruct(
        &self,
        fmt_configuration: &DataFormatConfiguration,
        f: &mut impl std::fmt::Write,
    ) -> fmt::Result {
        self.reconstruct_as(&self.name, fmt_configuration, f)
    }
}

impl Union<'_> {
    /// Reconstructs the union under the given name (e.g., the unqualified
    /// name when declared inside its parent type).
    pub(crate) fn reconstruct_as(
        &self,
        name: &str,
        fmt_configuration: &DataFormatConfiguration,
        f: &mut impl std::fmt::Write,
    ) -> fmt::Result {
        fmt_configuration.fmt_source_location(self.source_location.as_deref(), f)?;
        writeln!(
            f,
            "union {}{}{{{}",
            name,
            if fmt_configuration.print_brackets_new_line {
                "\n"
            } else {
                " "
            },
            if fmt_configuration.size_print_flavor == SizePrintFlavor::Comment {
                format!(" /* Size={:#x} */", self.size)
            } else {
                String::default()
            }
        )?;

        // Access in effect for the next member (see `access_specifier`)
        let current_access = Cell::new(FieldAccess::Public);

        // Nested declarations (written in PDB order with the other members
        // when preserving member order)
        if !fmt_configuration.preserve_member_order && !self.nested_types.is_empty() {
            fmt_configuration.fmt_separator(f)?;
            for nested in &self.nested_types {
                nested.reconstruct(fmt_configuration, &current_access, f)?;
            }
        }

        if fmt_configuration.preserve_member_order {
            self.fmt_members_in_order(fmt_configuration, &current_access, f)?;
        } else {
            // Dump fields while detecting unnamed structs and unions
            fmt_union_fields_recursive(
                fmt_configuration,
                &self.fields,
                1,
                Some(&current_access),
                f,
            )?;

            // Static fields
            for field in &self.static_fields {
                Self::fmt_static_field(fmt_configuration, field, &current_access, f)?;
            }

            if !self.instance_methods.is_empty() {
                fmt_configuration.fmt_separator(f)?;
                for method in &self.instance_methods {
                    Self::fmt_instance_method(fmt_configuration, method, &current_access, f)?;
                }
            }

            if !self.static_methods.is_empty() {
                fmt_configuration.fmt_separator(f)?;
                for method in &self.static_methods {
                    Self::fmt_static_method(fmt_configuration, method, &current_access, f)?;
                }
            }
        }

        writeln!(f, "}};")?;
        if fmt_configuration.size_print_flavor == SizePrintFlavor::StaticAssert {
            if fmt_configuration.integers_as_hexadecimal {
                writeln!(
                    f,
                    "static_assert(sizeof({}) == {:#x}); // {}",
                    name, self.size, self.size
                )?;
            } else {
                writeln!(
                    f,
                    "static_assert(sizeof({}) == {}); // {:#x}",
                    name, self.size, self.size
                )?;
            }
        }

        Ok(())
    }
}

impl<'p> Union<'p> {
    /// Sets what is known about the implementations of this union's methods
    /// and of the methods of its nested types: parameter names and/or RVAs.
    pub(crate) fn assign_method_implementations(
        &mut self,
        implementations: &MethodImplementations,
        parameter_names: bool,
        rvas: bool,
    ) {
        let methods = self
            .instance_methods
            .iter_mut()
            .chain(self.static_methods.iter_mut());
        for method in methods {
            let key = (format!("{}::{}", self.name, method.name), method.type_index);
            if let Some(implementation) = implementations.get(&key) {
                if parameter_names {
                    if let Some(names) = &implementation.parameter_names {
                        method.parameter_names = names.clone();
                    }
                }
                if rvas {
                    method.rva = implementation.rva;
                }
            }
        }
        for nested in &mut self.nested_types {
            nested.assign_method_implementations(implementations, parameter_names, rvas);
        }
    }

    /// Sets where the union and its nested types are defined.
    pub(crate) fn assign_source_locations(
        &mut self,
        locations: &TypeSourceLocations,
        type_forwarder: &TypeForwarder,
    ) {
        self.source_location = type_source_location(locations, type_forwarder, self.index);
        for nested in &mut self.nested_types {
            nested.assign_source_locations(locations, type_forwarder);
        }
    }

    fn push_method(&mut self, method: Method<'p>, is_static: bool) {
        if is_static {
            self.members
                .push(MemberRef::StaticMethod(self.static_methods.len()));
            self.static_methods.push(method);
        } else {
            self.members
                .push(MemberRef::InstanceMethod(self.instance_methods.len()));
            self.instance_methods.push(method);
        }
    }

    /// Writes members in PDB declaration order. See
    /// `Class::fmt_members_in_order`.
    fn fmt_members_in_order(
        &self,
        fmt_configuration: &DataFormatConfiguration,
        current_access: &Cell<FieldAccess>,
        f: &mut impl std::fmt::Write,
    ) -> fmt::Result {
        let mut field_positions = vec![0; self.fields.len()];
        for (position, member) in self.members.iter().enumerate() {
            if let MemberRef::Field(field_index) = *member {
                field_positions[field_index] = position;
            }
        }

        let mut next_position = 0;
        let mut last_kind = MemberKind::Data;
        fmt_union_fields_interleaved(
            fmt_configuration,
            &self.fields,
            1,
            Some(current_access),
            f,
            |field_index, f| {
                self.fmt_non_field_members(
                    fmt_configuration,
                    field_positions[field_index],
                    &mut next_position,
                    &mut last_kind,
                    current_access,
                    f,
                )?;
                if last_kind != MemberKind::Data {
                    fmt_configuration.fmt_separator(f)?;
                }
                last_kind = MemberKind::Data;
                Ok(())
            },
        )?;
        self.fmt_non_field_members(
            fmt_configuration,
            self.members.len(),
            &mut next_position,
            &mut last_kind,
            current_access,
            f,
        )
    }

    /// Writes the nested types, static fields and methods found in
    /// `self.members[*next_position..end_position]`, skipping instance fields
    /// which are written by `fmt_union_fields_interleaved`.
    fn fmt_non_field_members(
        &self,
        fmt_configuration: &DataFormatConfiguration,
        end_position: usize,
        next_position: &mut usize,
        last_kind: &mut MemberKind,
        current_access: &Cell<FieldAccess>,
        f: &mut impl std::fmt::Write,
    ) -> fmt::Result {
        while *next_position < end_position {
            let member = self.members[*next_position];
            *next_position += 1;

            if let MemberRef::Field(_) = member {
                continue;
            }
            if *last_kind != member.kind() {
                fmt_configuration.fmt_separator(f)?;
            }
            *last_kind = member.kind();

            match member {
                MemberRef::Field(_) => unreachable!("instance fields are skipped above"),
                MemberRef::NestedType(index) => {
                    self.nested_types[index].reconstruct(fmt_configuration, current_access, f)?
                }
                MemberRef::StaticField(index) => Self::fmt_static_field(
                    fmt_configuration,
                    &self.static_fields[index],
                    current_access,
                    f,
                )?,
                MemberRef::InstanceMethod(index) => Self::fmt_instance_method(
                    fmt_configuration,
                    &self.instance_methods[index],
                    current_access,
                    f,
                )?,
                MemberRef::StaticMethod(index) => Self::fmt_static_method(
                    fmt_configuration,
                    &self.static_methods[index],
                    current_access,
                    f,
                )?,
            }
        }

        Ok(())
    }

    fn fmt_static_field(
        fmt_configuration: &DataFormatConfiguration,
        field: &StaticField,
        current_access: &Cell<FieldAccess>,
        f: &mut impl std::fmt::Write,
    ) -> fmt::Result {
        writeln!(
            f,
            "{}{}static {} {}{};",
            fmt_configuration.indent(),
            access_specifier(fmt_configuration, field.access, Some(current_access)),
            field.type_left,
            &field.name,
            field.type_right,
        )
    }

    fn fmt_instance_method(
        fmt_configuration: &DataFormatConfiguration,
        method: &Method,
        current_access: &Cell<FieldAccess>,
        f: &mut impl std::fmt::Write,
    ) -> fmt::Result {
        writeln!(
            f,
            "{}{}{}{}{}{}{}({}){}{}{}{};{}",
            fmt_configuration.indent(),
            access_specifier(fmt_configuration, method.access, Some(current_access)),
            if method.is_virtual { "virtual " } else { "" },
            if method.is_ctor || method.is_dtor {
                ""
            } else {
                &method.return_type_name.0
            },
            if !method.is_ctor && !method.is_dtor && method.return_type_name.1.is_empty() {
                " "
            } else {
                ""
            },
            method
                .calling_convention_keyword(fmt_configuration)
                .map(|keyword| format!("{keyword} "))
                .unwrap_or_default(),
            &method.name,
            method.fmt_arguments(),
            method.return_type_name.1,
            if method.is_const { " const" } else { "" },
            if method.is_volatile { " volatile" } else { "" },
            if method.is_pure_virtual { " = 0" } else { "" },
            method.fmt_trailing_comments(fmt_configuration),
        )
    }

    fn fmt_static_method(
        fmt_configuration: &DataFormatConfiguration,
        method: &Method,
        current_access: &Cell<FieldAccess>,
        f: &mut impl std::fmt::Write,
    ) -> fmt::Result {
        writeln!(
            f,
            "{}{}{}static {}{}{}{}({}){}{}{};{}",
            fmt_configuration.indent(),
            access_specifier(fmt_configuration, method.access, Some(current_access)),
            if method.is_virtual { "virtual " } else { "" },
            method.return_type_name.0,
            if method.return_type_name.1.is_empty() {
                " "
            } else {
                ""
            },
            method
                .calling_convention_keyword(fmt_configuration)
                .map(|keyword| format!("{keyword} "))
                .unwrap_or_default(),
            &method.name,
            method.fmt_arguments(),
            method.return_type_name.1,
            if method.is_const { " const" } else { "" },
            if method.is_volatile { " volatile" } else { "" },
            method.fmt_trailing_comments(fmt_configuration),
        )
    }
}
