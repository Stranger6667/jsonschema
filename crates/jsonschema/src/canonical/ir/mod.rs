use std::{
    cmp::Ordering,
    hash::{Hash, Hasher},
    sync::Arc,
};

use serde_json::{Number, Value};
use strum::{EnumDiscriminants, IntoStaticStr};

use crate::{JsonType, JsonTypeSet};

mod bound_cardinality;
mod bound_integer;
mod raw;

pub(crate) use bound_cardinality::BoundCardinality;
pub(crate) use bound_integer::BoundInteger;
pub(crate) use raw::RawJson;

/// A `Const`/`Enum` member normalized at construction (`1.0` becomes `1`) so `Value` equality is value equality.
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
                if jsonschema_value::types::number_is_integer(number) {
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
    use crate::canonical::json::{integer_valued_i64, integer_valued_u64};
    let Some(float) = number
        .as_f64()
        .filter(|_| !number.is_i64() && !number.is_u64())
    else {
        return number.clone();
    };
    integer_valued_u64(float)
        .map(Number::from)
        .or_else(|| integer_valued_i64(float).map(Number::from))
        .unwrap_or_else(|| number.clone())
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

    /// Take the kind out, cloning only when the node is shared.
    #[must_use]
    pub(crate) fn into_kind(self) -> SchemaKind {
        match Arc::try_unwrap(self.0) {
            Ok(data) => data.kind,
            Err(shared) => shared.kind.clone(),
        }
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
    /// A string value within a length window; non-string values are matched by a surrounding union.
    String(StringLeaf),
    /// An integer value within a range; non-integer values are matched by a surrounding union.
    Integer(IntegerBounds),
    /// Exactly one admitted value.
    Const(CanonicalJson),
    /// A sorted, deduplicated finite set of admitted values.
    Enum(Vec<CanonicalJson>),
    /// A value matches iff at least one of the sorted, mutually unmergeable branches matches.
    AnyOf(Vec<Schema>),
    /// Matches any value.
    True,
    /// Matches no value.
    False,
    /// A schema the structural IR does not model, kept verbatim.
    Raw(RawJson),
}

/// The constraints a [`SchemaKind::String`] places on a string value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub(crate) struct StringLeaf {
    pub(crate) lengths: LengthBounds,
    /// Sorted, deduplicated. A string must match every pattern.
    pub(crate) patterns: Vec<Arc<str>>,
}

/// A closed interval; an absent side is unbounded.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub(crate) struct Bounds<T> {
    pub(crate) minimum: Option<T>,
    pub(crate) maximum: Option<T>,
}

impl<T: Ord> Bounds<T> {
    /// The window both accept: the higher minimum, the lower maximum.
    pub(crate) fn intersect(self, other: Self) -> Self {
        Self {
            minimum: tighter(self.minimum, other.minimum, Ord::max),
            maximum: tighter(self.maximum, other.maximum, Ord::min),
        }
    }

    pub(crate) fn contains(&self, value: &T) -> bool {
        self.minimum.as_ref().is_none_or(|min| value >= min)
            && self.maximum.as_ref().is_none_or(|max| value <= max)
    }

    /// Whether no value fits, i.e. `minimum > maximum`.
    pub(crate) fn is_empty(&self) -> bool {
        matches!((&self.minimum, &self.maximum), (Some(min), Some(max)) if min > max)
    }
}

/// The bound present on both sides picked by `keep`; otherwise whichever side has one.
fn tighter<T>(first: Option<T>, second: Option<T>, keep: impl FnOnce(T, T) -> T) -> Option<T> {
    match (first, second) {
        (Some(a), Some(b)) => Some(keep(a, b)),
        (bound, None) | (None, bound) => bound,
    }
}

pub(crate) type LengthBounds = Bounds<BoundCardinality>;
pub(crate) type IntegerBounds = Bounds<BoundInteger>;

impl SchemaKind {
    /// The admitted values when this node is a finite value set (`Const`/`Enum`), else `None`.
    #[must_use]
    pub(crate) fn finite_values(&self) -> Option<&[CanonicalJson]> {
        match self {
            SchemaKind::Const(value) => Some(std::slice::from_ref(value)),
            SchemaKind::Enum(values) => Some(values),
            _ => None,
        }
    }

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
        match bits {
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
