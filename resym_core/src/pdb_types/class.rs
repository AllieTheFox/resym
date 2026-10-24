use std::{cell::Cell, fmt};

use super::{
    access_specifier, class_kind_keyword,
    field::{FieldAccess, StaticField},
    fmt_struct_fields_interleaved, fmt_struct_fields_recursive,
    nested::NestedDeclaration,
    resolve_complete_type_index, type_bitfield_info, type_name, type_size, type_source_location,
    DataFormatConfiguration, Field, MemberKind, MemberRef, Method, MethodImplementations,
    NeededTypeSet, ReconstructibleTypeData, Result, ResymCoreError, SizePrintFlavor, TypeForwarder,
    TypeNameFlavor, TypeSourceLocations,
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
    /// Pointer size of the class' own vfptr, if the class introduces one
    pub vftable_pointer_size: Option<usize>,
    /// Types and typedefs declared inside the class, in PDB order
    pub nested_types: Vec<NestedDeclaration<'p>>,
    /// Where the class is defined (`file(line)`), if known
    pub source_location: Option<String>,
}

impl<'p> Class<'p> {
    pub fn new(index: pdb::TypeIndex, kind: pdb::ClassKind, name: String, size: u64) -> Self {
        Self {
            index,
            kind,
            name,
            size,
            base_classes: Vec::new(),
            fields: Vec::new(),
            static_fields: Vec::new(),
            instance_methods: Vec::new(),
            static_methods: Vec::new(),
            members: Vec::new(),
            vftable_pointer_size: None,
            nested_types: Vec::new(),
            source_location: None,
        }
    }

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
                let mut method = Method::find(
                    data.name,
                    data.attributes,
                    type_finder,
                    type_forwarder,
                    data.method_type,
                    type_name_flavor,
                    needed_types,
                )?;
                method.vtable_offset = data.vtable_offset;
                self.push_method(method, data.attributes.is_static());
            }

            pdb::TypeData::OverloadedMethod(data) => {
                // this just means we have more than one method with the same name
                // find the method list
                match type_finder.find(data.method_list)?.parse()? {
                    pdb::TypeData::MethodList(method_list) => {
                        for pdb::MethodListEntry {
                            attributes,
                            method_type,
                            vtable_offset,
                        } in method_list.methods
                        {
                            // hooray
                            let mut method = Method::find(
                                data.name,
                                attributes,
                                type_finder,
                                type_forwarder,
                                method_type,
                                type_name_flavor,
                                needed_types,
                            )?;
                            method.vtable_offset = vtable_offset;

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

            pdb::TypeData::BaseClass(data) => {
                // Resolve the complete type's index, if present in the PDB
                let complete_base_class_type_index =
                    resolve_complete_type_index(type_forwarder, data.base_class);
                self.base_classes.push(BaseClass {
                    type_name: type_name(
                        type_finder,
                        type_forwarder,
                        complete_base_class_type_index,
                        // Base specifiers can't be elaborated
                        &type_name_flavor.without_elaborated_type_specifiers(),
                        needed_types,
                    )?
                    .0,
                    offset: data.offset,
                    access: ClassAccess::from_field_attribute(data.attributes.access()),
                    is_virtual: false,
                    is_direct: true,
                })
            }

            pdb::TypeData::VirtualBaseClass(data) => {
                // Resolve the complete type's index, if present in the PDB
                let complete_base_class_type_index =
                    resolve_complete_type_index(type_forwarder, data.base_class);
                self.base_classes.push(BaseClass {
                    type_name: type_name(
                        type_finder,
                        type_forwarder,
                        complete_base_class_type_index,
                        &type_name_flavor.without_elaborated_type_specifiers(),
                        needed_types,
                    )?
                    .0,
                    offset: data.base_pointer_offset,
                    access: ClassAccess::from_field_attribute(data.attributes.access()),
                    is_virtual: true,
                    is_direct: data.direct
                })
            }

            pdb::TypeData::VirtualFunctionTablePointer(data) => {
                // `LF_VFUNCTAB` is only emitted for classes that introduce
                // their own vfptr (not when reusing a base class' vfptr).
                self.vftable_pointer_size = Some(type_size(type_finder, data.table)?);
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

            other => {
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
        self.reconstruct_as(&self.name, fmt_configuration, f)
    }
}

impl Class<'_> {
    /// Reconstructs the class under the given name (e.g., the unqualified
    /// name when declared inside its parent type).
    pub(crate) fn reconstruct_as(
        &self,
        name: &str,
        fmt_configuration: &DataFormatConfiguration,
        f: &mut impl std::fmt::Write,
    ) -> fmt::Result {
        fmt_configuration.fmt_source_location(self.source_location.as_deref(), f)?;
        write!(f, "{} {}", class_kind_keyword(self.kind), name)?;

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

        if !fmt_configuration.hide_base_class_offsets {
            for base in &self.base_classes {
                writeln!(
                    f,
                    "{}/* {:#06x}: fields for {} */",
                    fmt_configuration.indent(),
                    base.offset,
                    base.type_name
                )?;
            }
        }

        // Access in effect for the next member (see `access_specifier`)
        let current_access = Cell::new(if self.kind == pdb::ClassKind::Class {
            FieldAccess::Private
        } else {
            FieldAccess::Public
        });

        // Nested declarations (written in PDB order with the other members
        // when preserving member order)
        if !fmt_configuration.preserve_member_order && !self.nested_types.is_empty() {
            fmt_configuration.fmt_separator(f)?;
            for nested in &self.nested_types {
                nested.reconstruct(fmt_configuration, &current_access, f)?;
            }
        }

        self.fmt_vftable_pointer(fmt_configuration, f)?;

        if fmt_configuration.preserve_member_order {
            self.fmt_members_in_order(fmt_configuration, &current_access, f)?;
        } else {
            // Dump fields while detecting unnamed structs and unions
            fmt_struct_fields_recursive(
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
                    if Self::is_method_shown(fmt_configuration, method) {
                        self.fmt_instance_method(fmt_configuration, method, &current_access, f)?;
                    }
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

    /// Sets what is known about the implementations of this class' methods
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

    /// Sets where the class and its nested types are defined.
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

    fn is_method_shown(fmt_configuration: &DataFormatConfiguration, method: &Method) -> bool {
        !fmt_configuration.ignore_compiler_generated_methods || !method.is_compiler_generated
    }

    /// Writes the class' own vfptr. MSVC places a class' own vfptr at offset
    /// 0 (non-polymorphic bases are moved after it), but `LF_VFUNCTAB` does
    /// not record the offset, so it is only printed when no field or
    /// non-virtual base overlaps the first pointer-sized bytes.
    ///
    /// It is written as a comment: the class declares virtual methods, so the
    /// compiler adds the vfptr itself, and a `void*` member would add a second
    /// one.
    fn fmt_vftable_pointer(
        &self,
        fmt_configuration: &DataFormatConfiguration,
        f: &mut impl std::fmt::Write,
    ) -> fmt::Result {
        let Some(pointer_size) = self.vftable_pointer_size else {
            return Ok(());
        };

        write!(f, "{}", fmt_configuration.indent())?;
        if fmt_configuration.print_offset_info {
            let pointer_size = pointer_size as u64;
            let is_at_offset_zero = self.fields.iter().all(|field| field.offset >= pointer_size)
                && self
                    .base_classes
                    .iter()
                    .filter(|base| !base.is_virtual)
                    .all(|base| u64::from(base.offset) >= pointer_size);
            if is_at_offset_zero {
                write!(f, "/* {:#06x} */ ", 0)?;
            }
        }
        writeln!(f, "/* void* __vftable; */")
    }

    /// Writes members in PDB declaration order. Data members still go
    /// through `fmt_struct_fields_interleaved` so unnamed unions and bit-field
    /// padding are reconstructed; other members are written before the field
    /// group that follows them in the PDB. A blank line precedes every run of
    /// nested types or methods and every run of data members that follows
    /// them, like the grouped layout does.
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
        fmt_struct_fields_interleaved(
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
    /// which are written by `fmt_struct_fields_interleaved`.
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

            match member {
                MemberRef::Field(_) => continue,
                MemberRef::InstanceMethod(index)
                    if !Self::is_method_shown(fmt_configuration, &self.instance_methods[index]) =>
                {
                    continue
                }
                _ => {}
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
                MemberRef::InstanceMethod(index) => self.fmt_instance_method(
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
        &self,
        fmt_configuration: &DataFormatConfiguration,
        method: &Method,
        current_access: &Cell<FieldAccess>,
        f: &mut impl std::fmt::Write,
    ) -> fmt::Result {
        let method_has_class_name = method.name == self.name.as_str().into();
        let has_return_type = !method.is_ctor && !method.is_dtor && !method_has_class_name;
        writeln!(
            f,
            "{}{}{}{}{}{}{}({}){}{}{}{}{};{}",
            fmt_configuration.indent(),
            access_specifier(fmt_configuration, method.access, Some(current_access)),
            if method.is_virtual_printed(fmt_configuration) {
                "virtual "
            } else {
                ""
            },
            if has_return_type {
                &method.return_type_name.0
            } else {
                ""
            },
            if has_return_type && method.return_type_name.1.is_empty() {
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
            if method.is_override_printed(fmt_configuration) {
                " override"
            } else {
                ""
            },
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
            "{}{}static {}{}{}{}({}){}{}{};{}",
            fmt_configuration.indent(),
            access_specifier(fmt_configuration, method.access, Some(current_access)),
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
