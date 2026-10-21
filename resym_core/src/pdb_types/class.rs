use std::fmt;

use super::{
    enumeration::Enum,
    field::{FieldAccess, StaticField},
    fmt_struct_fields_interleaved, fmt_struct_fields_recursive, is_unnamed_type,
    primitive_types::PrimitiveReconstructionFlavor,
    resolve_complete_type_index, type_bitfield_info, type_name, type_size,
    union::Union,
    AccessSpecifierReconstructionFlavor, DataFormatConfiguration, Field, MemberRef, Method,
    NeededTypeSet, ReconstructibleTypeData, Result, ResymCoreError, SizePrintFlavor, TypeForwarder,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassAccess {
    None,
    Private,
    Protected,
    Public,
}
impl ClassAccess {
    pub fn from_field_attribute(value: u8) -> Self {
        match value {
            0 => ClassAccess::None,
            1 => ClassAccess::Private,
            2 => ClassAccess::Protected,
            3 => ClassAccess::Public,
            _ => unreachable!("Major PDB format update?"),
        }
    }
}
impl fmt::Display for ClassAccess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                ClassAccess::None => "",
                ClassAccess::Private => "private",
                ClassAccess::Protected => "protected",
                ClassAccess::Public => "public",
            }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseClass {
    type_name: String,
    offset: u32,
    access: ClassAccess,
    is_virtual: bool,
    is_direct: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Class<'p> {
    pub index: pdb::TypeIndex,
    pub kind: pdb::ClassKind,
    pub name: String,
    pub size: u64,
    pub base_classes: Vec<BaseClass>,
    pub fields: Vec<Field<'p>>,
    pub static_fields: Vec<StaticField<'p>>,
    pub instance_methods: Vec<Method<'p>>,
    pub static_methods: Vec<Method<'p>>,
    /// Fields and methods in PDB declaration order
    pub members: Vec<MemberRef>,
    pub nested_classes: Vec<Class<'p>>,
    pub nested_unions: Vec<Union<'p>>,
    pub nested_enums: Vec<Enum<'p>>,
}

impl<'p> Class<'p> {
    #[allow(clippy::unnecessary_wraps)]
    pub fn add_derived_from(
        &mut self,
        _: &pdb::TypeFinder<'p>,
        _: pdb::TypeIndex,
        _: &mut NeededTypeSet,
    ) -> Result<()> {
        // TODO
        Ok(())
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

            // Nested types
            pdb::TypeData::Class(data) => {
                let name_str = data.name.to_string();
                // Rename unnamed anonymous tags to something unique
                let name = if is_unnamed_type(&name_str) {
                    format!("_unnamed_{type_index}")
                } else {
                    name_str.into_owned()
                };

                let mut class = Class {
                    index: type_index,
                    kind: data.kind,
                    name,
                    size: data.size,
                    fields: Vec::new(),
                    static_fields: Vec::new(),
                    base_classes: Vec::new(),
                    instance_methods: Vec::new(),
                    static_methods: Vec::new(),
                    members: Vec::new(),
                    nested_classes: Vec::new(),
                    nested_unions: Vec::new(),
                    nested_enums: Vec::new(),
                };

                if let Some(derived_from) = data.derived_from {
                    class.add_derived_from(type_finder, derived_from, needed_types)?;
                }

                if let Some(fields) = data.fields {
                    class.add_fields(
                        type_finder,
                        type_forwarder,
                        fields,
                        primitive_flavor,
                        needed_types,
                    )?;
                }

                self.nested_classes.insert(0, class);
            }

            pdb::TypeData::Union(data) => {
                let name_str = data.name.to_string();
                // Rename unnamed anonymous tags to something unique
                let name = if is_unnamed_type(&name_str) {
                    format!("_unnamed_{type_index}")
                } else {
                    name_str.into_owned()
                };

                let mut u = Union {
                    index: type_index,
                    name,
                    size: data.size,
                    fields: Vec::new(),
                    static_fields: Vec::new(),
                    instance_methods: Vec::new(),
                    static_methods: Vec::new(),
                    members: Vec::new(),
                    nested_classes: Vec::new(),
                    nested_unions: Vec::new(),
                    nested_enums: Vec::new(),
                };

                u.add_fields(
                    type_finder,
                    type_forwarder,
                    data.fields,
                    primitive_flavor,
                    needed_types,
                )?;

                self.nested_unions.insert(0, u);
            }

            pdb::TypeData::Enumeration(data) => {
                let name_str = data.name.to_string();
                // Rename unnamed anonymous tags to something unique
                let name = if is_unnamed_type(&name_str) {
                    format!("_unnamed_{type_index}")
                } else {
                    name_str.into_owned()
                };

                let mut e = Enum {
                    index: type_index,
                    name,
                    underlying_type_name: type_name(
                        type_finder,
                        type_forwarder,
                        data.underlying_type,
                        primitive_flavor,
                        needed_types,
                    )?
                    .0,
                    values: Vec::new(),
                };

                e.add_fields(type_finder, data.fields, needed_types)?;

                self.nested_enums.insert(0, e);
            }

            pdb::TypeData::Primitive(_)
            | pdb::TypeData::Pointer(_)
            | pdb::TypeData::Procedure(_)
            | pdb::TypeData::Modifier(_) => {
                // TODO: What does this represent? Suppress warnings for the
                // moment
            }

            other => {
                log::warn!(
                    "trying to Class::add_fields() got {} -> {:?}",
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
        match *field {
            pdb::TypeData::Member(ref data) => {
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

            pdb::TypeData::StaticMember(ref data) => {
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

            pdb::TypeData::Method(ref data) => {
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

            pdb::TypeData::OverloadedMethod(ref data) => {
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

            pdb::TypeData::BaseClass(ref data) => {
                // Resolve the complete type's index, if present in the PDB
                let complete_base_class_type_index =
                    resolve_complete_type_index(type_forwarder, data.base_class);
                self.base_classes.push(BaseClass {
                    type_name: type_name(
                        type_finder,
                        type_forwarder,
                        complete_base_class_type_index,
                        primitive_flavor,
                        needed_types,
                    )?
                    .0,
                    offset: data.offset,
                    access: ClassAccess::from_field_attribute(data.attributes.access()),
                    is_virtual: false,
                    is_direct: true,
                })
            }

            pdb::TypeData::VirtualBaseClass(ref data) => {
                // Resolve the complete type's index, if present in the PDB
                let complete_base_class_type_index =
                    resolve_complete_type_index(type_forwarder, data.base_class);
                self.base_classes.push(BaseClass {
                    type_name: type_name(
                        type_finder,
                        type_forwarder,
                        complete_base_class_type_index,
                        primitive_flavor,
                        needed_types,
                    )?
                    .0,
                    offset: data.base_pointer_offset,
                    access: ClassAccess::from_field_attribute(data.attributes.access()),
                    is_virtual: true,
                    is_direct: data.direct
                })
            }

            pdb::TypeData::VirtualFunctionTablePointer(ref _data) => {
                // TODO: Display a comment at the beginning of the declaration
                // to make it obvious a vtable is present?
            }

            // Nested type declaration
            pdb::TypeData::Nested(ref _data) => {
                // TODO: Properly handle nested types
                // let complete_type_index =
                //     resolve_complete_type_index(type_forwarder, data.nested_type);
                // self.add_fields(
                //     type_finder,
                //     type_forwarder,
                //     complete_type_index,
                //     needed_types,
                // )?;
            }

            ref other => {
                log::error!("trying to Class::add_field(): {:?}", other);
                return Err(ResymCoreError::InvalidParameterError(
                    "unexpected type in Class::add_field()".to_string(),
                ));
            }
        }

        Ok(())
    }
}

impl ReconstructibleTypeData for Class<'_> {
    fn reconstruct(
        &self,
        fmt_configuration: &DataFormatConfiguration,
        f: &mut impl std::fmt::Write,
    ) -> fmt::Result {
        write!(
            f,
            "{} {}",
            match self.kind {
                pdb::ClassKind::Class => "class",
                pdb::ClassKind::Struct => "struct",
                // Not used C and C++ but well ...
                pdb::ClassKind::Interface => "interface",
            },
            self.name
        )?;

        if !self.base_classes.is_empty() {
            for (i, base) in self.base_classes.iter().enumerate() {
                if base.is_direct {
                    let prefix = match i {
                        0 => " :",
                        _ => ",",
                    };
                    if base.is_virtual {
                        write!(f, "{} {} virtual {}", prefix, base.access, base.type_name)?;
                    } else {
                        write!(f, "{} {} {}", prefix, base.access, base.type_name)?;
                    }
                }
            }
        }

        writeln!(
            f,
            "{}{{{}",
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

        for base in &self.base_classes {
            writeln!(
                f,
                "  /* {:#06x}: fields for {} */",
                base.offset, base.type_name
            )?;
        }

        // Nested declarations
        if !self.nested_classes.is_empty() {
            writeln!(f, "  ")?;
            for class in &self.nested_classes {
                class.reconstruct(fmt_configuration, f)?;
            }
        }
        if !self.nested_unions.is_empty() {
            writeln!(f, "  ")?;
            for u in &self.nested_unions {
                u.reconstruct(fmt_configuration, f)?;
            }
        }
        if !self.nested_enums.is_empty() {
            writeln!(f, "  ")?;
            for e in &self.nested_enums {
                e.reconstruct(fmt_configuration, f)?;
            }
        }

        if fmt_configuration.preserve_member_order {
            self.fmt_members_in_order(fmt_configuration, f)?;
        } else {
            // Dump fields while detecting unnamed structs and unions
            fmt_struct_fields_recursive(fmt_configuration, &self.fields, 1, f)?;

            // Static fields
            for field in &self.static_fields {
                self.fmt_static_field(fmt_configuration, field, f)?;
            }

            if !self.instance_methods.is_empty() {
                writeln!(f, "  ")?;
                for method in &self.instance_methods {
                    if Self::is_method_shown(fmt_configuration, method) {
                        self.fmt_instance_method(fmt_configuration, method, f)?;
                    }
                }
            }

            if !self.static_methods.is_empty() {
                writeln!(f, "  ")?;
                for method in &self.static_methods {
                    self.fmt_static_method(fmt_configuration, method, f)?;
                }
            }
        }

        writeln!(f, "}};")?;
        if fmt_configuration.size_print_flavor == SizePrintFlavor::StaticAssert {
            if fmt_configuration.integers_as_hexadecimal {
                writeln!(
                    f,
                    "static_assert(sizeof({}) == {:#x}); // {}",
                    self.name, self.size, self.size
                )?;
            } else {
                writeln!(
                    f,
                    "static_assert(sizeof({}) == {}); // {:#x}",
                    self.name, self.size, self.size
                )?;
            }
        }

        Ok(())
    }
}

impl<'p> Class<'p> {
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

    fn is_method_shown(fmt_configuration: &DataFormatConfiguration, method: &Method) -> bool {
        !fmt_configuration.ignore_compiler_generated_methods || !method.is_compiler_generated
    }

    /// Writes members in PDB declaration order. Data members still go
    /// through `fmt_struct_fields_interleaved` so unnamed unions and bit-field
    /// padding are reconstructed; other members are written before the field
    /// group that follows them in the PDB. A blank line precedes every run of
    /// methods and every run of data members that follows methods, like the
    /// grouped layout does.
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
        let mut last_was_method = false;
        fmt_struct_fields_interleaved(fmt_configuration, &self.fields, 1, f, |field_index, f| {
            self.fmt_non_field_members(
                fmt_configuration,
                field_positions[field_index],
                &mut next_position,
                &mut last_was_method,
                f,
            )?;
            if last_was_method {
                writeln!(f, "  ")?;
            }
            last_was_method = false;
            Ok(())
        })?;
        self.fmt_non_field_members(
            fmt_configuration,
            self.members.len(),
            &mut next_position,
            &mut last_was_method,
            f,
        )
    }

    /// Writes the static fields and methods found in
    /// `self.members[*next_position..end_position]`, skipping instance fields
    /// which are written by `fmt_struct_fields_interleaved`.
    fn fmt_non_field_members(
        &self,
        fmt_configuration: &DataFormatConfiguration,
        end_position: usize,
        next_position: &mut usize,
        last_was_method: &mut bool,
        f: &mut impl std::fmt::Write,
    ) -> fmt::Result {
        while *next_position < end_position {
            let member = self.members[*next_position];
            *next_position += 1;

            let is_method = match member {
                MemberRef::Field(_) => continue,
                MemberRef::StaticField(_) => false,
                MemberRef::InstanceMethod(index) => {
                    if !Self::is_method_shown(fmt_configuration, &self.instance_methods[index]) {
                        continue;
                    }
                    true
                }
                MemberRef::StaticMethod(_) => true,
            };
            if *last_was_method != is_method {
                writeln!(f, "  ")?;
            }
            *last_was_method = is_method;

            match member {
                MemberRef::Field(_) => unreachable!("instance fields are skipped above"),
                MemberRef::StaticField(index) => {
                    self.fmt_static_field(fmt_configuration, &self.static_fields[index], f)?
                }
                MemberRef::InstanceMethod(index) => {
                    self.fmt_instance_method(fmt_configuration, &self.instance_methods[index], f)?
                }
                MemberRef::StaticMethod(index) => {
                    self.fmt_static_method(fmt_configuration, &self.static_methods[index], f)?
                }
            }
        }

        Ok(())
    }

    fn fmt_static_field(
        &self,
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
                    if self.kind == pdb::ClassKind::Class || field.access != FieldAccess::Public {
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
        &self,
        fmt_configuration: &DataFormatConfiguration,
        method: &Method,
        f: &mut impl std::fmt::Write,
    ) -> fmt::Result {
        let method_has_class_name = method.name == self.name.as_str().into();
        writeln!(
            f,
            "  {}{}{}{}{}({}){}{}{}{};",
            match fmt_configuration.print_access_specifiers {
                AccessSpecifierReconstructionFlavor::Disabled => &FieldAccess::None,
                AccessSpecifierReconstructionFlavor::Always => &method.access,
                AccessSpecifierReconstructionFlavor::Automatic => {
                    if self.kind == pdb::ClassKind::Class || method.access != FieldAccess::Public {
                        &method.access
                    } else {
                        &FieldAccess::None
                    }
                }
            },
            if method.is_virtual { "virtual " } else { "" },
            if method.is_ctor || method.is_dtor || method_has_class_name {
                ""
            } else {
                &method.return_type_name.0
            },
            if !method.is_ctor
                && !method.is_dtor
                && !method_has_class_name
                && method.return_type_name.1.is_empty()
            {
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
        &self,
        fmt_configuration: &DataFormatConfiguration,
        method: &Method,
        f: &mut impl std::fmt::Write,
    ) -> fmt::Result {
        writeln!(
            f,
            "  {}static {}{}{}({}){}{}{};",
            match fmt_configuration.print_access_specifiers {
                AccessSpecifierReconstructionFlavor::Disabled => &FieldAccess::None,
                AccessSpecifierReconstructionFlavor::Always => &method.access,
                AccessSpecifierReconstructionFlavor::Automatic => {
                    if self.kind == pdb::ClassKind::Class || method.access != FieldAccess::Public {
                        &method.access
                    } else {
                        &FieldAccess::None
                    }
                }
            },
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
