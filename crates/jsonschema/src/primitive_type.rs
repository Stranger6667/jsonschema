//! Primitive types for property type validators

use serde_json::Value;
use std::{convert::TryFrom, fmt, ops::BitOrAssign};

/// For faster error handling in "type" keyword validator we have this enum, to match
/// with it instead of a string.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
#[allow(missing_docs)]
pub enum PrimitiveType {
    Array,
    Boolean,
    Integer,
    Null,
    Number,
    Object,
    String,
}

impl fmt::Display for PrimitiveType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PrimitiveType::Array => f.write_str("array"),
            PrimitiveType::Boolean => f.write_str("boolean"),
            PrimitiveType::Integer => f.write_str("integer"),
            PrimitiveType::Null => f.write_str("null"),
            PrimitiveType::Number => f.write_str("number"),
            PrimitiveType::Object => f.write_str("object"),
            PrimitiveType::String => f.write_str("string"),
        }
    }
}

impl TryFrom<&str> for PrimitiveType {
    type Error = ();

    #[inline]
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "array" => Ok(PrimitiveType::Array),
            "boolean" => Ok(PrimitiveType::Boolean),
            "integer" => Ok(PrimitiveType::Integer),
            "null" => Ok(PrimitiveType::Null),
            "number" => Ok(PrimitiveType::Number),
            "object" => Ok(PrimitiveType::Object),
            "string" => Ok(PrimitiveType::String),
            _ => Err(()),
        }
    }
}

impl From<&Value> for PrimitiveType {
    fn from(instance: &Value) -> Self {
        match instance {
            Value::Null => PrimitiveType::Null,
            Value::Bool(_) => PrimitiveType::Boolean,
            Value::Number(_) => PrimitiveType::Number,
            Value::String(_) => PrimitiveType::String,
            Value::Array(_) => PrimitiveType::Array,
            Value::Object(_) => PrimitiveType::Object,
        }
    }
}

const fn primitive_type_to_bit_map_representation(primitive_type: PrimitiveType) -> u8 {
    match primitive_type {
        PrimitiveType::Array => 1,
        PrimitiveType::Boolean => 2,
        PrimitiveType::Integer => 4,
        PrimitiveType::Null => 8,
        PrimitiveType::Number => 16,
        PrimitiveType::Object => 32,
        PrimitiveType::String => 64,
    }
}

fn bit_map_representation_primitive_type(bit_representation: u8) -> PrimitiveType {
    match bit_representation {
        1 => PrimitiveType::Array,
        2 => PrimitiveType::Boolean,
        4 => PrimitiveType::Integer,
        8 => PrimitiveType::Null,
        16 => PrimitiveType::Number,
        32 => PrimitiveType::Object,
        64 => PrimitiveType::String,
        _ => unreachable!("This should never be possible"),
    }
}

/// Compact representation of multiple [`PrimitiveType`]
#[derive(Clone, Copy, Debug)]
pub struct PrimitiveTypesBitMap {
    inner: u8,
}
impl PrimitiveTypesBitMap {
    pub(crate) const fn new() -> Self {
        Self { inner: 0 }
    }

    #[inline]
    pub(crate) const fn add_type(mut self, primitive_type: PrimitiveType) -> Self {
        self.inner |= primitive_type_to_bit_map_representation(primitive_type);
        self
    }

    pub(crate) const fn contains_type(self, primitive_type: PrimitiveType) -> bool {
        primitive_type_to_bit_map_representation(primitive_type) & self.inner != 0
    }
}
impl BitOrAssign<PrimitiveType> for PrimitiveTypesBitMap {
    #[inline]
    fn bitor_assign(&mut self, rhs: PrimitiveType) {
        *self = self.add_type(rhs);
    }
}
impl IntoIterator for PrimitiveTypesBitMap {
    type Item = PrimitiveType;
    type IntoIter = PrimitiveTypesBitMapIterator;
    fn into_iter(self) -> Self::IntoIter {
        PrimitiveTypesBitMapIterator { bit_map: self }
    }
}
#[cfg(test)]
impl From<Vec<PrimitiveType>> for PrimitiveTypesBitMap {
    fn from(value: Vec<PrimitiveType>) -> Self {
        let mut result = Self::new();
        for primitive_type in value {
            result |= primitive_type;
        }
        result
    }
}

/// Iterator over all [`PrimitiveType`] present in a [`PrimitiveTypesBitMap`]
#[derive(Debug)]
pub struct PrimitiveTypesBitMapIterator {
    bit_map: PrimitiveTypesBitMap,
}

impl Iterator for PrimitiveTypesBitMapIterator {
    type Item = PrimitiveType;

    fn next(&mut self) -> Option<Self::Item> {
        if self.bit_map.inner == 0 {
            None
        } else {
            // Find the least significant bit that is set
            let least_significant_bit = self.bit_map.inner & -(self.bit_map.inner as i8) as u8;

            // Clear the least significant bit
            self.bit_map.inner &= self.bit_map.inner - 1;

            // Convert the bit to PrimitiveType and return
            Some(bit_map_representation_primitive_type(least_significant_bit))
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let count = self.bit_map.inner.count_ones() as usize;
        (count, Some(count))
    }
}

impl ExactSizeIterator for PrimitiveTypesBitMapIterator {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_multiple_types() {
        let mut types = PrimitiveTypesBitMap::new();
        types |= PrimitiveType::Null;
        types |= PrimitiveType::String;
        types |= PrimitiveType::Array;
        assert!(types.contains_type(PrimitiveType::Null));
        assert!(types.contains_type(PrimitiveType::String));
        assert!(types.contains_type(PrimitiveType::Array));
        assert_eq!(
            types.into_iter().collect::<Vec<PrimitiveType>>(),
            vec![
                PrimitiveType::Array,
                PrimitiveType::Null,
                PrimitiveType::String
            ]
        )
    }
}
