use std::fmt;

use super::{
    field::{FieldAccess, StaticField},
    fmt_union_fields_interleaved, fmt_union_fields_recursive,
    nested::NestedDeclaration,
    primitive_types::AccessSpecifierReconstructionFlavor,
    primitive_types::PrimitiveReconstructionFlavor,
    primitive_types::SizePrintFlavor,
    resolve_complete_type_index, type_bitfield_info, type_name, type_size, DataFormatConfiguration,
    Field, MemberKind, MemberRef, Method, NeededTypeSet, ReconstructibleTypeData, TypeForwarder,
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
        }
    }

    pub fn add_fields(
        &mut self,
        type_finder: &pdb::TypeFinder<'p>,
        type_forwarder: &TypeForwarder,
        type_index: pdb::TypeIndex,
        primitive_flavor: &PrimitiveReconstructionFlavor,
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
                        primitive_flavor,
                        needed_types,
                    )?;
                }

                if let Some(continuation) = data.continuation {
                    // recurse
                    self.add_fields(
                        type_finder,
                        type_forwarder,
                        continuation,
                        primitive_flavor,
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
        primitive_flavor: &PrimitiveReconstructionFlavor,
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
                    primitive_flavor,
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
                    primitive_flavor,
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
                    primitive_flavor,
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
                    primitive_flavor,
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
                                primitive_flavor,
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

        // Nested declarations (written in PDB order with the other members
        // when preserving member order)
        if !fmt_configuration.preserve_member_order && !self.nested_types.is_empty() {
            writeln!(f, "  ")?;
            for nested in &self.nested_types {
                nested.reconstruct(fmt_configuration, false, f)?;
            }
        }

        if fmt_configuration.preserve_member_order {
            self.fmt_members_in_order(fmt_configuration, f)?;
        } else {
            // Dump fields while detecting unnamed structs and unions
            fmt_union_fields_recursive(fmt_configuration, &self.fields, 1, f)?;

            // Static fields
            for field in &self.static_fields {
                Self::fmt_static_field(fmt_configuration, field, f)?;
            }

            if !self.instance_methods.is_empty() {
                writeln!(f, "  ")?;
                for method in &self.instance_methods {
                    Self::fmt_instance_method(fmt_configuration, method, f)?;
                }
            }

            if !self.static_methods.is_empty() {
                writeln!(f, "  ")?;
                for method in &self.static_methods {
                    Self::fmt_static_method(fmt_configuration, method, f)?;
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
        fmt_union_fields_interleaved(fmt_configuration, &self.fields, 1, f, |field_index, f| {
            self.fmt_non_field_members(
                fmt_configuration,
                field_positions[field_index],
                &mut next_position,
                &mut last_kind,
                f,
            )?;
            if last_kind != MemberKind::Data {
                writeln!(f, "  ")?;
            }
            last_kind = MemberKind::Data;
            Ok(())
        })?;
        self.fmt_non_field_members(
            fmt_configuration,
            self.members.len(),
            &mut next_position,
            &mut last_kind,
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
        f: &mut impl std::fmt::Write,
    ) -> fmt::Result {
        while *next_position < end_position {
            let member = self.members[*next_position];
            *next_position += 1;

            if let MemberRef::Field(_) = member {
                continue;
            }
            if *last_kind != member.kind() {
                writeln!(f, "  ")?;
            }
            *last_kind = member.kind();

            match member {
                MemberRef::Field(_) => unreachable!("instance fields are skipped above"),
                MemberRef::NestedType(index) => {
                    self.nested_types[index].reconstruct(fmt_configuration, false, f)?
                }
                MemberRef::StaticField(index) => {
                    Self::fmt_static_field(fmt_configuration, &self.static_fields[index], f)?
                }
                MemberRef::InstanceMethod(index) => {
                    Self::fmt_instance_method(fmt_configuration, &self.instance_methods[index], f)?
                }
                MemberRef::StaticMethod(index) => {
                    Self::fmt_static_method(fmt_configuration, &self.static_methods[index], f)?
                }
            }
        }

        Ok(())
    }

    fn fmt_static_field(
        fmt_configuration: &DataFormatConfiguration,
        field: &StaticField,
        f: &mut impl std::fmt::Write,
    ) -> fmt::Result {
        writeln!(
            f,
            "  {}static {} {}{};",
            match fmt_configuration.print_access_specifiers {
                AccessSpecifierReconstructionFlavor::Disabled => &FieldAccess::None,
                AccessSpecifierReconstructionFlavor::Always => &field.access,
                AccessSpecifierReconstructionFlavor::Automatic => {
                    if field.access != FieldAccess::Public {
                        &field.access
                    } else {
                        &FieldAccess::None
                    }
                }
            },
            field.type_left,
            &field.name,
            field.type_right,
        )
    }

    fn fmt_instance_method(
        fmt_configuration: &DataFormatConfiguration,
        method: &Method,
        f: &mut impl std::fmt::Write,
    ) -> fmt::Result {
        writeln!(
            f,
            "  {}{}{}{}{}({}){}{}{}{};",
            match fmt_configuration.print_access_specifiers {
                AccessSpecifierReconstructionFlavor::Disabled => &FieldAccess::None,
                AccessSpecifierReconstructionFlavor::Always => &method.access,
                AccessSpecifierReconstructionFlavor::Automatic => {
                    if method.access != FieldAccess::Public {
                        &method.access
                    } else {
                        &FieldAccess::None
                    }
                }
            },
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
            &method.name,
            method
                .arguments
                .iter()
                .map(|(type_left, type_right)| format!("{type_left}{type_right}"))
                .collect::<Vec<String>>()
                .join(", "),
            method.return_type_name.1,
            if method.is_const { " const" } else { "" },
            if method.is_volatile { " volatile" } else { "" },
            if method.is_pure_virtual { " = 0" } else { "" },
        )
    }

    fn fmt_static_method(
        fmt_configuration: &DataFormatConfiguration,
        method: &Method,
        f: &mut impl std::fmt::Write,
    ) -> fmt::Result {
        writeln!(
            f,
            "  {}{}static {}{}{}({}){}{}{};",
            match fmt_configuration.print_access_specifiers {
                AccessSpecifierReconstructionFlavor::Disabled => &FieldAccess::None,
                AccessSpecifierReconstructionFlavor::Always => &method.access,
                AccessSpecifierReconstructionFlavor::Automatic => {
                    if method.access != FieldAccess::Public {
                        &method.access
                    } else {
                        &FieldAccess::None
                    }
                }
            },
            if method.is_virtual { "virtual " } else { "" },
            method.return_type_name.0,
            if method.return_type_name.1.is_empty() {
                " "
            } else {
                ""
            },
            &method.name,
            method
                .arguments
                .iter()
                .map(|(type_left, type_right)| format!("{type_left}{type_right}"))
                .collect::<Vec<String>>()
                .join(", "),
            method.return_type_name.1,
            if method.is_const { " const" } else { "" },
            if method.is_volatile { " volatile" } else { "" },
        )
    }
}
