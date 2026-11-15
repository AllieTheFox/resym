use std::collections::HashMap;

use crate::pdb_types::split_qualified_name;

type MemberConstants = HashMap<String, Option<(pdb::TypeIndex, pdb::Variant)>>;

/// Value-bearing symbols, grouped by their raw declaring type and member name.
#[derive(Default)]
pub(crate) struct ConstantValues {
    members: HashMap<String, MemberConstants>,
}

impl ConstantValues {
    pub(crate) fn insert(&mut self, name: &str, type_index: pdb::TypeIndex, value: pdb::Variant) {
        let Some((owner, member)) = split_qualified_name(name) else {
            return;
        };
        let constant = (type_index, value);
        self.members
            .entry(owner.to_owned())
            .or_default()
            .entry(member.to_owned())
            .and_modify(|previous| {
                // Separate compilands can disagree. Don't invent a single initializer.
                if *previous != Some(constant) {
                    *previous = None;
                }
            })
            .or_insert(Some(constant));
    }

    pub(crate) fn get(
        &self,
        owner: &str,
        member: &str,
        type_index: pdb::TypeIndex,
    ) -> Option<pdb::Variant> {
        let (constant_type, value) = self.members.get(owner)?.get(member)?.as_ref()?;
        (*constant_type == type_index).then_some(*value)
    }
}

pub(crate) fn unmodified_type_index(
    type_finder: &pdb::TypeFinder<'_>,
    mut type_index: pdb::TypeIndex,
) -> pdb::TypeIndex {
    while let Ok(pdb::TypeData::Modifier(modifier)) =
        type_finder.find(type_index).and_then(|item| item.parse())
    {
        type_index = modifier.underlying_type;
    }
    type_index
}

/// Numeric leaves may encode an unsigned constant as a smaller signed value.
pub(crate) fn normalize_constant(
    type_finder: &pdb::TypeFinder<'_>,
    mut type_index: pdb::TypeIndex,
    value: pdb::Variant,
) -> pdb::Variant {
    let signed = match value {
        pdb::Variant::U8(value) => i64::from(value as i8),
        pdb::Variant::I8(value) => i64::from(value),
        pdb::Variant::I16(value) => i64::from(value),
        pdb::Variant::I32(value) => i64::from(value),
        pdb::Variant::I64(value) => value,
        _ => return value,
    };
    if signed >= 0 {
        return value;
    }
    loop {
        type_index = unmodified_type_index(type_finder, type_index);
        match type_finder.find(type_index).and_then(|item| item.parse()) {
            Ok(pdb::TypeData::Enumeration(enumeration)) => {
                type_index = enumeration.underlying_type;
            }
            Ok(pdb::TypeData::Primitive(primitive)) if primitive.indirection.is_none() => {
                return match primitive.kind {
                    pdb::PrimitiveKind::UChar
                    | pdb::PrimitiveKind::U8
                    | pdb::PrimitiveKind::Char8 => pdb::Variant::U16(u16::from(signed as u8)),
                    pdb::PrimitiveKind::UShort
                    | pdb::PrimitiveKind::U16
                    | pdb::PrimitiveKind::WChar
                    | pdb::PrimitiveKind::RChar16 => pdb::Variant::U16(signed as u16),
                    pdb::PrimitiveKind::ULong
                    | pdb::PrimitiveKind::U32
                    | pdb::PrimitiveKind::RChar32 => pdb::Variant::U32(signed as u32),
                    pdb::PrimitiveKind::UQuad | pdb::PrimitiveKind::U64 => {
                        pdb::Variant::U64(signed as u64)
                    }
                    _ => value,
                };
            }
            _ => return value,
        }
    }
}

pub(crate) fn is_enum_type(
    type_finder: &pdb::TypeFinder<'_>,
    mut type_index: pdb::TypeIndex,
) -> bool {
    loop {
        match type_finder.find(type_index).and_then(|item| item.parse()) {
            Ok(pdb::TypeData::Modifier(modifier)) => type_index = modifier.underlying_type,
            Ok(pdb::TypeData::Enumeration(_)) => return true,
            _ => return false,
        }
    }
}

pub(crate) fn is_integral_type(
    type_finder: &pdb::TypeFinder<'_>,
    type_index: pdb::TypeIndex,
) -> bool {
    let type_index = unmodified_type_index(type_finder, type_index);
    match type_finder.find(type_index).and_then(|item| item.parse()) {
        Ok(pdb::TypeData::Enumeration(_)) => true,
        Ok(pdb::TypeData::Primitive(primitive)) if primitive.indirection.is_none() => {
            matches!(
                primitive.kind,
                pdb::PrimitiveKind::Char
                    | pdb::PrimitiveKind::RChar
                    | pdb::PrimitiveKind::UChar
                    | pdb::PrimitiveKind::WChar
                    | pdb::PrimitiveKind::RChar16
                    | pdb::PrimitiveKind::RChar32
                    | pdb::PrimitiveKind::Char8
                    | pdb::PrimitiveKind::I8
                    | pdb::PrimitiveKind::U8
                    | pdb::PrimitiveKind::Short
                    | pdb::PrimitiveKind::UShort
                    | pdb::PrimitiveKind::I16
                    | pdb::PrimitiveKind::U16
                    | pdb::PrimitiveKind::Long
                    | pdb::PrimitiveKind::ULong
                    | pdb::PrimitiveKind::I32
                    | pdb::PrimitiveKind::U32
                    | pdb::PrimitiveKind::Quad
                    | pdb::PrimitiveKind::UQuad
                    | pdb::PrimitiveKind::I64
                    | pdb::PrimitiveKind::U64
                    | pdb::PrimitiveKind::Bool8
                    | pdb::PrimitiveKind::Bool16
                    | pdb::PrimitiveKind::Bool32
                    | pdb::PrimitiveKind::Bool64
                    | pdb::PrimitiveKind::HRESULT
            )
        }
        _ => false,
    }
}

pub(crate) fn constant_initializer(
    value: pdb::Variant,
    hexadecimal: bool,
    is_enum: bool,
    type_name: &str,
) -> String {
    let literal = match value {
        pdb::Variant::U8(value) if hexadecimal => format!("0x{value:02x}"),
        // LF_CHAR is signed, although the pdb crate represents it as U8.
        pdb::Variant::U8(value) => (value as i8).to_string(),
        pdb::Variant::U16(value) if hexadecimal => format!("0x{value:04x}"),
        pdb::Variant::U32(value) if hexadecimal => format!("0x{value:08x}"),
        pdb::Variant::U64(value) if hexadecimal => format!("0x{value:016x}"),
        pdb::Variant::U64(value) if value > i64::MAX as u64 => format!("{value}ULL"),
        pdb::Variant::I64(i64::MIN) => "(-9223372036854775807LL - 1)".to_owned(),
        value => value.to_string(),
    };
    if is_enum {
        format!("static_cast<{type_name}>({literal})")
    } else {
        literal
    }
}

#[cfg(test)]
mod tests {
    use super::{constant_initializer, is_integral_type, normalize_constant, ConstantValues};
    use pdb::FallibleIterator;

    #[test]
    fn constants_require_matching_owner_type_and_unambiguous_value() {
        let mut values = ConstantValues::default();
        let type_index = pdb::TypeIndex(0x1002);
        values.insert("First::limit", type_index, pdb::Variant::U16(9));
        values.insert("Second::limit", type_index, pdb::Variant::U16(12));
        values.insert("First::limit", type_index, pdb::Variant::U16(9));
        assert_eq!(
            values.get("First", "limit", type_index),
            Some(pdb::Variant::U16(9))
        );
        assert_eq!(
            values.get("Second", "limit", type_index),
            Some(pdb::Variant::U16(12))
        );
        assert_eq!(values.get("First", "limit", pdb::TypeIndex(0x1003)), None);
        values.insert("First::limit", type_index, pdb::Variant::U16(10));
        values.insert("First::limit", type_index, pdb::Variant::U16(9));
        assert_eq!(values.get("First", "limit", type_index), None);
    }

    #[test]
    fn compact_negative_leaves_use_the_declared_unsigned_width() {
        let file = std::fs::File::open("tests/data/test.pdb").expect("open test PDB");
        let mut pdb = pdb::PDB::open(file).expect("read test PDB");
        let types = pdb.type_information().expect("read types");
        let finder = types.finder();
        let negative_one = pdb::Variant::U8(255);
        // CodeView's unsigned __int64, unsigned long, and unsigned char primitives.
        assert_eq!(
            normalize_constant(&finder, pdb::TypeIndex(0x23), negative_one),
            pdb::Variant::U64(u64::MAX)
        );
        assert_eq!(
            normalize_constant(&finder, pdb::TypeIndex(0x22), negative_one),
            pdb::Variant::U32(u32::MAX)
        );
        assert_eq!(
            normalize_constant(&finder, pdb::TypeIndex(0x20), negative_one),
            pdb::Variant::U16(255)
        );
        assert_eq!(
            normalize_constant(&finder, pdb::TypeIndex(0x13), negative_one),
            negative_one
        );
    }

    #[test]
    fn aggregate_and_pointer_constants_are_not_integer_initializers() {
        let file = std::fs::File::open("tests/data/test.pdb").expect("open test PDB");
        let mut pdb = pdb::PDB::open(file).expect("read test PDB");
        let types = pdb.type_information().expect("read types");
        let mut finder = types.finder();
        let mut iter = types.iter();
        while iter.next().expect("read type").is_some() {
            finder.update(&iter);
        }
        assert!(is_integral_type(&finder, pdb::TypeIndex(0x1398)));
        assert!(is_integral_type(&finder, pdb::TypeIndex(0x1397)));
        assert!(!is_integral_type(&finder, pdb::TypeIndex(0x195e)));
        assert!(!is_integral_type(&finder, pdb::TypeIndex(0x0403)));
    }

    #[test]
    fn integer_initializers_preserve_signedness_and_literal_boundaries() {
        assert_eq!(
            constant_initializer(pdb::Variant::U8(255), false, false, "signed char"),
            "-1"
        );
        assert_eq!(
            constant_initializer(pdb::Variant::U16(255), false, false, "unsigned char"),
            "255"
        );
        assert_eq!(
            constant_initializer(
                pdb::Variant::U64(u64::MAX),
                false,
                false,
                "unsigned __int64"
            ),
            "18446744073709551615ULL"
        );
        assert_eq!(
            constant_initializer(pdb::Variant::I64(i64::MIN), false, false, "__int64"),
            "(-9223372036854775807LL - 1)"
        );
        assert_eq!(
            constant_initializer(pdb::Variant::U64(9), true, false, "unsigned __int64"),
            "0x0000000000000009"
        );
        assert_eq!(
            constant_initializer(pdb::Variant::U16(9), false, true, "const Flags"),
            "static_cast<const Flags>(9)"
        );
    }
}
