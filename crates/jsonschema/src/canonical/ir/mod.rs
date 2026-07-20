use std::{
    cmp::Ordering,
    hash::{Hash, Hasher},
    sync::Arc,
};

use serde_json::{Number, Value};
use strum::{EnumDiscriminants, IntoStaticStr};

use crate::{JsonType, JsonTypeSet};

mod raw;

pub(crate) use raw::RawJson;

/// A `Const`/`Enum` member with one spelling per JSON value: numbers are normalized at
/// construction (`1.0` becomes `1`), so plain `Value` equality is value equality.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CanonicalJson(Arc<Value>);

impl CanonicalJson {
    #[must_use]
    pub(crate) fn from_value(value: &Value) -> Self {
        Self(Arc::new(normalized(value)))
    }

    #[must_use]
    pub(crate) fn as_value(&self) -> &Value {
        &self.0
    }

    #[must_use]
    pub(crate) fn to_value(&self) -> Value {
        (*self.0).clone()
    }

    #[must_use]
    pub(crate) fn json_type(&self) -> JsonType {
        match self.as_value() {
            Value::Null => JsonType::Null,
            Value::Bool(_) => JsonType::Boolean,
            Value::Number(number) => {
                if number.as_i64().is_some() || number.as_u64().is_some() {
                    JsonType::Integer
                } else {
                    JsonType::Number
                }
            }
            Value::String(_) => JsonType::String,
            Value::Array(_) => JsonType::Array,
            Value::Object(_) => JsonType::Object,
        }
    }
}

impl PartialOrd for CanonicalJson {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CanonicalJson {
    fn cmp(&self, other: &Self) -> Ordering {
        raw::compare_values(&self.0, &other.0)
    }
}

impl Hash for CanonicalJson {
    fn hash<H: Hasher>(&self, state: &mut H) {
        raw::hash_value(&self.0, state);
    }
}

/// One spelling per JSON value: integer-valued numbers become integers everywhere in the tree.
fn normalized(value: &Value) -> Value {
    match value {
        Value::Number(number) => Value::Number(normalized_number(number)),
        Value::Array(items) => Value::Array(items.iter().map(normalized).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| (key.clone(), normalized(item)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Rewrite an integer-valued float (`1.0`, `-0.0`) to its integer form so `Number` equality is value equality.
#[cfg(not(feature = "arbitrary-precision"))]
fn normalized_number(number: &Number) -> Number {
    use crate::canonical::json::{
        I64_LOWER_INCLUSIVE_F64, I64_UPPER_EXCLUSIVE_F64, U64_UPPER_EXCLUSIVE_F64,
    };
    let Some(float) = number
        .as_f64()
        .filter(|_| !number.is_i64() && !number.is_u64())
    else {
        return number.clone();
    };
    if float.fract() == 0.0 {
        if (0.0..U64_UPPER_EXCLUSIVE_F64).contains(&float) {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "guarded by the `0.0..U64_UPPER_EXCLUSIVE_F64` range and zero fractional part"
            )]
            return Number::from(float as u64);
        }
        if (I64_LOWER_INCLUSIVE_F64..I64_UPPER_EXCLUSIVE_F64).contains(&float) {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "guarded by the `I64_LOWER_INCLUSIVE_F64..I64_UPPER_EXCLUSIVE_F64` range and zero fractional part"
            )]
            return Number::from(float as i64);
        }
    }
    number.clone()
}

/// Rewrite an integer-valued float (`1.0`, `-0.0`) to its integer form so `Number` equality is value equality.
#[cfg(feature = "arbitrary-precision")]
fn normalized_number(number: &Number) -> Number {
    // The modeling gate admits only plain spellings, whose canonical texts are plain too.
    match crate::canonical::json::canonical_number(number.as_str()) {
        Some(text) => text.parse().expect("canonical number text parses"),
        None => number.clone(),
    }
}

/// Reference-counted canonical IR handle, passed throughout canonicalization.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct Schema(Arc<SchemaData>);

impl Schema {
    #[must_use]
    pub(crate) fn new(kind: SchemaKind) -> Self {
        let hash = structural_hash(&kind);
        Self(Arc::new(SchemaData { kind, hash }))
    }

    #[inline]
    #[must_use]
    pub(crate) fn kind(&self) -> &SchemaKind {
        &self.0.kind
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, EnumDiscriminants)]
#[strum_discriminants(
    name(CanonicalKind),
    vis(pub),
    derive(Hash, IntoStaticStr),
    strum(serialize_all = "snake_case"),
    doc = "Structural discriminant of a [`CanonicalSchema`](crate::CanonicalSchema), one variant per IR arm."
)]
pub(crate) enum SchemaKind {
    /// A value matches iff its JSON type is in the set (`Integer` drops when `Number` is present).
    MultiType(JsonTypeSet),
    /// A value matches iff its JSON type is `ty` *and* it satisfies `body` (Draft 4 `integer`, where `1.0` is not an integer).
    TypedGroup { ty: JsonType, body: Schema },
    /// Exactly one admitted value.
    Const(CanonicalJson),
    /// A sorted, deduplicated finite set of admitted values.
    Enum(Vec<CanonicalJson>),
    /// Matches any value.
    True,
    /// Matches no value.
    False,
    /// A schema the structural IR does not model, kept verbatim.
    Raw(RawJson),
}

impl SchemaKind {
    /// Drop redundant entries from a type set: `Integer` is removed when `Number` is present.
    #[must_use]
    pub(crate) fn canonical_type_set(set: JsonTypeSet) -> JsonTypeSet {
        if set.contains(JsonType::Number) {
            set.remove(JsonType::Integer)
        } else {
            set
        }
    }

    /// Expand a type set to its semantic cover: `Number` implies `Integer`.
    #[must_use]
    pub(crate) fn semantic_cover(set: JsonTypeSet) -> JsonTypeSet {
        if set.contains(JsonType::Number) {
            set.insert(JsonType::Integer)
        } else {
            set
        }
    }

    /// The type set `values` saturates - only `null` and `boolean` have finite universes.
    #[must_use]
    pub(crate) fn finite_values_saturated_domain(values: &[CanonicalJson]) -> Option<JsonTypeSet> {
        const NULL: u8 = 1 << 0;
        const FALSE: u8 = 1 << 1;
        const TRUE: u8 = 1 << 2;
        const BOTH_BOOLEANS: u8 = FALSE | TRUE;
        const ALL: u8 = NULL | FALSE | TRUE;
        let mut bits: u8 = 0;
        for value in values {
            bits |= match value.as_value() {
                Value::Null => NULL,
                Value::Bool(false) => FALSE,
                Value::Bool(true) => TRUE,
                _ => return None,
            };
        }
        // Distinctness check: a duplicated member means some inhabitant is missing.
        if bits.count_ones() as usize != values.len() {
            return None;
        }
        match bits {
            NULL => Some(JsonTypeSet::from(JsonType::Null)),
            BOTH_BOOLEANS => Some(JsonTypeSet::from(JsonType::Boolean)),
            ALL => Some(JsonTypeSet::from(JsonType::Null).insert(JsonType::Boolean)),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
struct SchemaData {
    kind: SchemaKind,
    /// Cached so equality rejects a mismatch without deep-comparing the subtree.
    hash: u64,
}

impl PartialEq for SchemaData {
    fn eq(&self, other: &Self) -> bool {
        // Cheap hash first, so a mismatch skips the deep `kind` compare.
        self.hash == other.hash && self.kind == other.kind
    }
}

impl Eq for SchemaData {}

impl PartialOrd for SchemaData {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SchemaData {
    fn cmp(&self, other: &Self) -> Ordering {
        if std::ptr::eq(self, other) {
            return Ordering::Equal;
        }
        self.kind.cmp(&other.kind)
    }
}

impl Hash for SchemaData {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.hash);
    }
}

// Folds in the variant plus each child's cached hash - O(direct children), not the whole subtree.
fn structural_hash(kind: &SchemaKind) -> u64 {
    let mut hasher = ahash::AHasher::default();
    kind.hash(&mut hasher);
    hasher.finish()
}
