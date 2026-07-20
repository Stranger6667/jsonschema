//! JSON type representations for schema validation.
//!
//! Provides [`JsonType`] for individual types and [`JsonTypeSet`] for efficient
//! bitset-based type checking in validation hot paths.

use core::fmt;
use std::str::FromStr;

use serde_json::Value;

/// Represents a JSON value type.
///
/// Discriminant order is `Null < Boolean < Integer < Number < String < Array < Object`
/// (primitives before compounds, integers before numbers); [`JsonTypeSet`] iterates in this order.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[repr(u8)]
pub enum JsonType {
    Null = 1 << 0,
    Boolean = 1 << 1,
    Integer = 1 << 2,
    Number = 1 << 3,
    String = 1 << 4,
    Array = 1 << 5,
    Object = 1 << 6,
}

impl fmt::Display for JsonType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl JsonType {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            JsonType::Array => "array",
            JsonType::Boolean => "boolean",
            JsonType::Integer => "integer",
            JsonType::Null => "null",
            JsonType::Number => "number",
            JsonType::Object => "object",
            JsonType::String => "string",
        }
    }

    pub(crate) fn from_repr(repr: u8) -> Self {
        match repr {
            1 => JsonType::Null,
            2 => JsonType::Boolean,
            4 => JsonType::Integer,
            8 => JsonType::Number,
            16 => JsonType::String,
            32 => JsonType::Array,
            64 => JsonType::Object,
            _ => panic!("Invalid JsonType representation: {repr}"),
        }
    }
}

impl From<&Value> for JsonType {
    fn from(instance: &Value) -> Self {
        match instance {
            Value::Null => JsonType::Null,
            Value::Bool(_) => JsonType::Boolean,
            Value::Number(_) => JsonType::Number,
            Value::String(_) => JsonType::String,
            Value::Array(_) => JsonType::Array,
            Value::Object(_) => JsonType::Object,
        }
    }
}

impl FromStr for JsonType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "array" => Ok(JsonType::Array),
            "boolean" => Ok(JsonType::Boolean),
            "integer" => Ok(JsonType::Integer),
            "null" => Ok(JsonType::Null),
            "number" => Ok(JsonType::Number),
            "object" => Ok(JsonType::Object),
            "string" => Ok(JsonType::String),
            _ => Err(()),
        }
    }
}

/// A set of JSON types.
#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct JsonTypeSet(u8);

impl Default for JsonTypeSet {
    fn default() -> Self {
        Self::empty()
    }
}

impl From<JsonType> for JsonTypeSet {
    #[inline]
    fn from(ty: JsonType) -> Self {
        Self(ty as u8)
    }
}

impl std::ops::BitOr for JsonType {
    type Output = JsonTypeSet;
    #[inline]
    fn bitor(self, rhs: JsonType) -> JsonTypeSet {
        JsonTypeSet::from(self).insert(rhs)
    }
}

impl std::ops::BitOr<JsonType> for JsonTypeSet {
    type Output = JsonTypeSet;
    #[inline]
    fn bitor(self, rhs: JsonType) -> JsonTypeSet {
        self.insert(rhs)
    }
}

impl JsonTypeSet {
    /// Create an empty set of types.
    #[inline]
    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }
    /// Create a set with all possible JSON types.
    #[inline]
    #[must_use]
    pub const fn all() -> Self {
        JsonTypeSet::empty()
            .insert(JsonType::Null)
            .insert(JsonType::Boolean)
            .insert(JsonType::Integer)
            .insert(JsonType::Number)
            .insert(JsonType::String)
            .insert(JsonType::Array)
            .insert(JsonType::Object)
    }
    /// Add a type to this set and return the modified set.
    #[inline]
    #[must_use]
    pub const fn insert(mut self, ty: JsonType) -> Self {
        self.0 |= ty as u8;
        self
    }
    /// Remove a type from this set and return the modified set.
    #[inline]
    #[must_use]
    pub const fn remove(mut self, ty: JsonType) -> Self {
        self.0 &= !(ty as u8);
        self
    }
    /// Types in both sets.
    #[inline]
    #[must_use]
    pub(crate) const fn intersect(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }
    /// Types in either set.
    #[inline]
    #[must_use]
    pub(crate) const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
    /// Return the number of types in this set.
    #[inline]
    #[must_use]
    pub const fn len(self) -> usize {
        self.0.count_ones() as usize
    }
    /// Return `true` if the set contains no types.
    #[inline]
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
    /// Check if this set includes the specified type.
    #[inline]
    #[must_use]
    pub fn contains(self, ty: JsonType) -> bool {
        self.0 & ty as u8 != 0
    }
    /// Check if a JSON value's type is allowed by this set.
    #[must_use]
    pub fn contains_value_type(self, value: &Value) -> bool {
        match value {
            Value::Array(_) => self.contains(JsonType::Array),
            Value::Bool(_) => self.contains(JsonType::Boolean),
            Value::Null => self.contains(JsonType::Null),
            Value::Number(n) => {
                if number_is_integer(n) {
                    // Integer numbers match either Integer or Number types
                    self.contains(JsonType::Integer) || self.contains(JsonType::Number)
                } else {
                    // Floating-point numbers only match Number type
                    self.contains(JsonType::Number)
                }
            }
            Value::Object(_) => self.contains(JsonType::Object),
            Value::String(_) => self.contains(JsonType::String),
        }
    }
    /// Get an iterator over the types in this set.
    #[inline]
    #[must_use]
    pub fn iter(&self) -> JsonTypeSetIterator {
        JsonTypeSetIterator { set: *self }
    }
}

/// Whether `n` holds an integer value per drafts 6+ (floats with a zero fractional part count as integers).
pub(crate) fn number_is_integer(n: &serde_json::Number) -> bool {
    #[cfg(feature = "arbitrary-precision")]
    {
        use crate::ext::numeric::bignum;
        use num_traits::One;

        // Important: check BigFraction BEFORE as_f64() to avoid precision loss.
        n.is_i64()
            || n.is_u64()
            || if bignum::try_parse_bigint(n).is_some() {
                true
            } else if let Some(bigfrac) = bignum::try_parse_bigfraction(n) {
                bigfrac.denom().is_none_or(One::is_one)
            } else if let Some(f) = n.as_f64() {
                f.fract() == 0.
            } else {
                // Numbers that overflow to infinity (as_f64() returns None).
                false
            }
    }
    #[cfg(not(feature = "arbitrary-precision"))]
    {
        if n.is_i64() || n.is_u64() {
            true
        } else if let Some(f) = n.as_f64() {
            f.fract() == 0.
        } else {
            unreachable!("Numbers always fit in u64/i64/f64 without arbitrary-precision")
        }
    }
}

impl IntoIterator for &JsonTypeSet {
    type Item = JsonType;
    type IntoIter = JsonTypeSetIterator;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl IntoIterator for JsonTypeSet {
    type Item = JsonType;
    type IntoIter = JsonTypeSetIterator;

    fn into_iter(self) -> Self::IntoIter {
        JsonTypeSetIterator { set: self }
    }
}

impl fmt::Debug for JsonTypeSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(")?;

        let mut iter = self.iter();

        if let Some(ty) = iter.next() {
            write!(f, "{ty}")?;
        }

        for ty in iter {
            write!(f, ", {ty}")?;
        }

        write!(f, ")")
    }
}

/// Iterator for traversing the types in a `JsonTypeSet`.
#[derive(Debug)]
pub struct JsonTypeSetIterator {
    set: JsonTypeSet,
}

impl Iterator for JsonTypeSetIterator {
    type Item = JsonType;

    fn next(&mut self) -> Option<Self::Item> {
        if self.set.0 == 0 {
            None
        } else {
            // Find the least significant bit that is set
            let lsb = self.set.0 & self.set.0.wrapping_neg();

            // Clear the least significant bit
            self.set.0 &= self.set.0 - 1;

            Some(JsonType::from_repr(lsb))
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let count = self.set.0.count_ones() as usize;
        (count, Some(count))
    }
}

impl ExactSizeIterator for JsonTypeSetIterator {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use test_case::test_case;

    #[test]
    fn type_bitor_builds_set() {
        let set = JsonType::String | JsonType::Integer;
        assert!(set.contains(JsonType::String));
        assert!(set.contains(JsonType::Integer));
        assert!(!set.contains(JsonType::Null));
        let extended = set | JsonType::Null;
        assert_eq!(
            extended,
            JsonType::String | JsonType::Integer | JsonType::Null
        );
        assert!(extended.contains(JsonType::Null));
    }

    #[test_case("array" => Ok(JsonType::Array) ; "parse array")]
    #[test_case("boolean" => Ok(JsonType::Boolean) ; "parse boolean")]
    #[test_case("integer" => Ok(JsonType::Integer) ; "parse integer")]
    #[test_case("null" => Ok(JsonType::Null) ; "parse null")]
    #[test_case("number" => Ok(JsonType::Number) ; "parse number")]
    #[test_case("object" => Ok(JsonType::Object) ; "parse object")]
    #[test_case("string" => Ok(JsonType::String) ; "parse string")]
    #[test_case("invalid" => Err(()) ; "parse invalid")]
    fn test_from_str(input: &str) -> Result<JsonType, ()> {
        JsonType::from_str(input)
    }

    #[test_case(JsonType::Array => "array" ; "display array")]
    #[test_case(JsonType::Boolean => "boolean" ; "display boolean")]
    #[test_case(JsonType::Integer => "integer" ; "display integer")]
    #[test_case(JsonType::Null => "null" ; "display null")]
    #[test_case(JsonType::Number => "number" ; "display number")]
    #[test_case(JsonType::Object => "object" ; "display object")]
    #[test_case(JsonType::String => "string" ; "display string")]
    fn test_display(json_type: JsonType) -> String {
        json_type.to_string()
    }

    #[test_case(&json!(null) => JsonType::Null ; "value null")]
    #[test_case(&json!(true) => JsonType::Boolean ; "value boolean")]
    #[test_case(&json!(42) => JsonType::Number ; "value number int")]
    #[test_case(&json!(1.12) => JsonType::Number ; "value number float")]
    #[test_case(&json!("hello") => JsonType::String ; "value string")]
    #[test_case(&json!([1, 2, 3]) => JsonType::Array ; "value array")]
    #[test_case(&json!({"key": "value"}) => JsonType::Object ; "value object")]
    fn test_from_value(value: &Value) -> JsonType {
        JsonType::from(value)
    }

    #[test]
    fn test_insert_types() {
        let mut set = JsonTypeSet::empty();
        set = set.insert(JsonType::String);
        assert!(set.contains(JsonType::String));
        assert!(!set.contains(JsonType::Number));

        set = set.insert(JsonType::Number);
        assert!(set.contains(JsonType::String));
        assert!(set.contains(JsonType::Number));
        assert!(!set.contains(JsonType::Array));
    }

    #[test]
    fn test_from_json_type() {
        let set = JsonTypeSet::from(JsonType::String);
        assert!(set.contains(JsonType::String));
        assert_eq!(set.len(), 1);
    }

    #[test_case(&json!(null), JsonTypeSet::from(JsonType::Null) => true ; "null type")]
    #[test_case(&json!(true), JsonTypeSet::from(JsonType::Boolean) => true ; "boolean type")]
    #[test_case(&json!("test"), JsonTypeSet::from(JsonType::String) => true ; "string type")]
    #[test_case(&json!([1,2]), JsonTypeSet::from(JsonType::Array) => true ; "array type")]
    #[test_case(&json!({"a": 1}), JsonTypeSet::from(JsonType::Object) => true ; "object type")]
    #[test_case(&json!(42), JsonTypeSet::from(JsonType::Number) => true ; "number matches number")]
    #[test_case(&json!(42), JsonTypeSet::from(JsonType::Integer) => true ; "int matches integer")]
    #[test_case(&json!(1.23), JsonTypeSet::from(JsonType::Number) => true ; "float matches number")]
    #[test_case(&json!(1.23), JsonTypeSet::from(JsonType::Integer) => false ; "float doesn't match integer")]
    fn test_contains_value_type(value: &Value, set: JsonTypeSet) -> bool {
        set.contains_value_type(value)
    }

    #[test]
    fn test_remove_types() {
        let set = JsonTypeSet::all().remove(JsonType::Number);
        assert!(!set.contains(JsonType::Number));
        assert!(set.contains(JsonType::Integer));
        assert_eq!(set.len(), 6);

        let empty = JsonTypeSet::empty();
        assert_eq!(empty.remove(JsonType::Boolean), empty);
    }

    #[test]
    fn test_len() {
        let empty = JsonTypeSet::empty();
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);

        let with_string = empty.insert(JsonType::String);
        assert!(!with_string.is_empty());
        assert_eq!(with_string.len(), 1);
        assert_eq!(JsonTypeSet::all().len(), 7);
    }

    #[test]
    fn test_debug_format() {
        assert_eq!(format!("{:?}", JsonTypeSet::default()), "()");
        assert_eq!(
            format!("{:?}", JsonTypeSet::from(JsonType::String)),
            "(string)"
        );
        assert_eq!(
            format!(
                "{:?}",
                JsonTypeSet::from(JsonType::String).insert(JsonType::Number)
            ),
            "(number, string)"
        );
    }

    #[test]
    fn test_empty_iterator() {
        let set = JsonTypeSet::empty();
        let mut iter = set.iter();
        assert_eq!(iter.next(), None);
        assert_eq!(iter.size_hint(), (0, Some(0)));
    }

    #[test]
    fn test_single_type_iterator() {
        let set = JsonTypeSet::from(JsonType::String);
        let mut iter = set.iter();
        assert_eq!(iter.size_hint(), (1, Some(1)));
        assert_eq!(iter.next(), Some(JsonType::String));
        assert_eq!(iter.size_hint(), (0, Some(0)));
        assert_eq!(iter.next(), None);
        assert_eq!(iter.size_hint(), (0, Some(0)));
    }

    #[test]
    fn test_multiple_types_iterator() {
        let set = JsonTypeSet::from(JsonType::String)
            .insert(JsonType::Number)
            .insert(JsonType::Boolean);

        let types: Vec<JsonType> = set.iter().collect();
        assert_eq!(types.len(), 3);
        assert!(types.contains(&JsonType::String));
        assert!(types.contains(&JsonType::Number));
        assert!(types.contains(&JsonType::Boolean));

        assert_eq!(set.iter().size_hint(), (3, Some(3)));
    }

    #[test]
    fn test_all_types_iterator() {
        let set = JsonTypeSet::all();

        let types: Vec<JsonType> = set.iter().collect();
        assert_eq!(types.len(), 7);

        let mut iter = set.iter();
        assert_eq!(iter.len(), 7);
        iter.next();
        assert_eq!(iter.len(), 6);
    }
}
