use std::{
    cmp::Ordering,
    collections::BTreeMap,
    hash::{Hash, Hasher},
    sync::Arc,
};

use serde_json::{Number, Value};
use strum::{EnumDiscriminants, IntoStaticStr};

use crate::{JsonType, JsonTypeSet};

mod array_leaves;
mod bound_cardinality;
mod bound_integer;
mod bound_number;
mod bound_rational;
mod constructors;
mod divisors;
mod integer_leaves;
mod number_leaves;
mod object_leaves;
mod raw;
mod string_leaves;
mod verdict;

pub(crate) use array_leaves::ArrayLeaves;
pub(crate) use bound_cardinality::BoundCardinality;
pub(crate) use bound_integer::{BoundInteger, Round};
pub(crate) use bound_number::{BoundNumber, Side};
pub(crate) use bound_rational::BoundRational;
pub(crate) use constructors::{canonicalize_value_set, type_set_schema, typed_group};
pub(crate) use divisors::Divisors;
pub(crate) use integer_leaves::IntegerLeaves;
pub(crate) use number_leaves::NumberLeaves;
pub(crate) use object_leaves::ObjectLeaves;
pub(crate) use raw::RawJson;
pub(crate) use string_leaves::StringLeaves;
pub(crate) use verdict::Verdict;

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
        other @ (Value::Null | Value::Bool(_) | Value::String(_)) => other.clone(),
    }
}

/// Rewrite an integer-valued float (`1.0`, `-0.0`) to its integer form so `Number` equality is value equality.
#[cfg(not(feature = "arbitrary-precision"))]
pub(crate) fn normalized_number(number: &Number) -> Number {
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
pub(crate) fn normalized_number(number: &Number) -> Number {
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
    String(NonEmpty<StringLeaf>),
    /// An integer value within a range; non-integer values are matched by a surrounding union.
    Integer(NonEmpty<IntegerLeaf>),
    /// A number value within a real interval; other types are matched by a surrounding union.
    Number(NonEmpty<NumberLeaf>),
    /// An array value within the leaf's constraints; other types are matched by a surrounding
    /// union.
    Array(NonEmpty<ArrayLeaf>),
    /// An object value whose property count is within a window and which carries every required key;
    /// other types are matched by a surrounding union.
    Object(NonEmpty<ObjectLeaf>),
    /// Exactly one admitted value.
    Const(CanonicalJson),
    /// A sorted, deduplicated finite set of admitted values.
    Enum(AtLeastTwo<CanonicalJson>),
    /// A value matches iff at least one of the sorted, mutually unmergeable branches matches.
    AnyOf(AtLeastTwo<Schema>),
    /// Matches any value.
    True,
    /// Matches no value.
    False,
    /// A schema the structural IR does not model, kept verbatim.
    Raw(RawJson),
}

/// The constraints a [`SchemaKind::Number`] places on a number value. The interval is over the
/// reals, so each end carries its own inclusivity and no successor exists to fold it away.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub(crate) struct NumberLeaf {
    pub(crate) minimum: Option<BoundNumber>,
    pub(crate) maximum: Option<BoundNumber>,
    /// Divisors every admitted value is a multiple of.
    pub(crate) multiple_of: Divisors,
}

impl NumberLeaf {
    /// Whether no real value fits between the two ends.
    pub(crate) fn is_vacant(&self) -> bool {
        // An interval holding no multiple of the divisor admits nothing either.
        if !self
            .multiple_of
            .admit_between(self.minimum.as_ref(), self.maximum.as_ref())
        {
            return true;
        }
        let (Some(min), Some(max)) = (&self.minimum, &self.maximum) else {
            return false;
        };
        // The ends cross, or they meet on a limit at least one side excludes.
        !min.admits(&max.to_number(), Side::Lower) || !max.admits(&min.to_number(), Side::Upper)
    }
}

impl MaybeEmpty for NumberLeaf {
    fn is_empty(&self) -> bool {
        self.is_vacant()
    }
}

/// The constraints a [`SchemaKind::Integer`] places on an integer value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub(crate) struct IntegerLeaf {
    pub(crate) bounds: IntegerBounds,
    /// Divisors every admitted value is a multiple of.
    pub(crate) multiple_of: Divisors,
}

impl MaybeEmpty for IntegerLeaf {
    fn is_empty(&self) -> bool {
        self.bounds.is_empty()
    }
}

/// The constraints a [`SchemaKind::Array`] places on an array value. An array of at most one item
/// is distinct on its own, so `unique` is set only when the window admits a second item.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub(crate) struct ArrayLeaf {
    pub(crate) lengths: LengthBounds,
    pub(crate) unique: bool,
    /// Per-index schemas: the element at position `i` must satisfy `prefix[i]`.
    pub(crate) prefix: Vec<Schema>,
    /// The schema every element from `prefix.len()` onward must satisfy.
    pub(crate) items: Option<Schema>,
}

impl ArrayLeaf {
    /// Whether every array satisfies this leaf, which makes it the `array` type written longhand.
    /// Every facet has to be listed here, or a union folds the leaf into the type set and drops it.
    pub(crate) fn spans_domain(&self) -> bool {
        self.lengths.is_unbounded()
            && !self.unique
            && self.prefix.is_empty()
            && self.items.is_none()
    }
}

impl MaybeEmpty for ArrayLeaf {
    fn is_empty(&self) -> bool {
        self.lengths.is_empty()
    }
}

/// The constraints a [`SchemaKind::Object`] places on an object value. A required key implies a
/// property, so `sizes.minimum` is kept above `required.len()` or absent - never a repeat of it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub(crate) struct ObjectLeaf {
    pub(crate) sizes: LengthBounds,
    /// Sorted, deduplicated. An object must carry every one of these keys.
    pub(crate) required: Vec<Arc<str>>,
    /// Every key must satisfy this schema, which is narrowed to the string domain.
    pub(crate) property_names: Option<Schema>,
    /// The schema each named key must satisfy when the object carries it.
    pub(crate) properties: BTreeMap<Arc<str>, Schema>,
    /// The schema every key matching the pattern must satisfy when the object carries it.
    pub(crate) pattern_properties: BTreeMap<Arc<str>, Schema>,
}

impl ObjectLeaf {
    /// The number of keys that can actually be present, when the property-name schema admits a
    /// finite set: a key whose property schema admits no value can never appear, so it does not
    /// count.
    #[must_use]
    pub(crate) fn admitted_key_count(&self) -> Option<BoundCardinality> {
        let values = self.property_names.as_ref()?.kind().finite_values()?;
        let present = values
            .iter()
            .filter(|value| {
                !matches!(value.as_value(), serde_json::Value::String(key)
                    if self
                        .properties
                        .get(key.as_str())
                        .is_some_and(|child| matches!(child.kind(), SchemaKind::False)))
            })
            .count();
        Some(BoundCardinality::from(present as u64))
    }

    /// The size window with a finite set of admitted keys folded in as a ceiling: those keys are
    /// distinct, so they cap the property count just as `maxProperties` does.
    #[must_use]
    pub(crate) fn effective_sizes(&self) -> LengthBounds {
        LengthBounds {
            minimum: self.sizes.minimum.clone(),
            maximum: tighter(
                self.sizes.maximum.clone(),
                self.admitted_key_count(),
                Ord::min,
            ),
        }
    }

    /// Whether every object satisfies this leaf, which makes it the `object` type written longhand.
    /// Every facet has to be listed here, or a union folds the leaf into the type set and drops it.
    pub(crate) fn spans_domain(&self) -> bool {
        self.sizes.is_unbounded()
            && self.required.is_empty()
            && self.property_names.is_none()
            && self.properties.is_empty()
            && self.pattern_properties.is_empty()
    }

    /// The keys an object must carry, as a count bound.
    #[must_use]
    pub(crate) fn required_count(&self) -> BoundCardinality {
        BoundCardinality::from(self.required.len() as u64)
    }
}

impl MaybeEmpty for ObjectLeaf {
    fn is_empty(&self) -> bool {
        if self.sizes.is_empty() {
            return true;
        }
        let Some(ceiling) = self.effective_sizes().maximum else {
            return false;
        };
        ceiling < self.required_count()
            || self
                .sizes
                .minimum
                .as_ref()
                .is_some_and(|min| ceiling < *min)
    }
}

/// The constraints a [`SchemaKind::String`] places on a string value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub(crate) struct StringLeaf {
    pub(crate) lengths: LengthBounds,
    /// Sorted, deduplicated. A string must match every pattern.
    pub(crate) patterns: Vec<Arc<str>>,
    /// Sorted, deduplicated. A string must satisfy every format. Empty unless formats assert.
    pub(crate) formats: Vec<Arc<str>>,
}

/// Sorted, deduplicated, and holding at least two elements; fewer collapses to a simpler node.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct AtLeastTwo<T>(Vec<T>);

impl<T: Ord> AtLeastTwo<T> {
    /// Sorts and deduplicates; the survivors come back in `Err` when fewer than two remain.
    pub(crate) fn new(mut items: Vec<T>) -> Result<Self, Vec<T>> {
        items.sort();
        items.dedup();
        if items.len() < 2 {
            return Err(items);
        }
        debug_assert!(
            items.windows(2).all(|pair| pair[0] < pair[1]),
            "items left unsorted or duplicated"
        );
        Ok(Self(items))
    }
}

impl<T> AtLeastTwo<T> {
    pub(crate) fn as_slice(&self) -> &[T] {
        &self.0
    }

    pub(crate) fn into_vec(self) -> Vec<T> {
        self.0
    }

    /// Split the last element off; the remainder still holds at least one.
    pub(crate) fn split_last(self) -> (Vec<T>, T) {
        let mut items = self.0;
        let last = items.pop().expect("at least two elements");
        (items, last)
    }
}

impl<T> IntoIterator for AtLeastTwo<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

/// A facet set admitting at least one value; the only way to build one is [`NonEmpty::new`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct NonEmpty<T>(T);

pub(crate) trait MaybeEmpty {
    fn is_empty(&self) -> bool;
}

impl<T: MaybeEmpty> NonEmpty<T> {
    pub(crate) fn new(inner: T) -> Option<Self> {
        (!inner.is_empty()).then_some(Self(inner))
    }

    pub(crate) fn get(&self) -> &T {
        &self.0
    }

    pub(crate) fn into_inner(self) -> T {
        self.0
    }
}

impl<T: Ord> MaybeEmpty for Bounds<T> {
    fn is_empty(&self) -> bool {
        Bounds::is_empty(self)
    }
}

impl MaybeEmpty for StringLeaf {
    fn is_empty(&self) -> bool {
        self.lengths.is_empty()
    }
}

/// A closed interval; an absent side is unbounded.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct Bounds<T> {
    pub(crate) minimum: Option<T>,
    pub(crate) maximum: Option<T>,
}

// Hand-written to avoid a spurious `T: Default` bound; an unbounded window needs none.
impl<T> Default for Bounds<T> {
    fn default() -> Self {
        Self {
            minimum: None,
            maximum: None,
        }
    }
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

    /// Whether every value `other` admits also fits here.
    pub(crate) fn covers(&self, other: &Self) -> bool {
        self.minimum
            .as_ref()
            .is_none_or(|min| other.minimum.as_ref().is_some_and(|start| start >= min))
            && self
                .maximum
                .as_ref()
                .is_none_or(|max| other.maximum.as_ref().is_some_and(|end| end <= max))
    }

    /// Whether every value in the domain fits.
    pub(crate) fn is_unbounded(&self) -> bool {
        self.minimum.is_none() && self.maximum.is_none()
    }

    /// The narrowest window holding both: the lower minimum, the higher maximum. An absent bound is
    /// unbounded on that side, so it swallows the present one.
    fn hull(self, other: Self) -> Self {
        Self {
            minimum: self.minimum.zip(other.minimum).map(|(a, b)| a.min(b)),
            maximum: self.maximum.zip(other.maximum).map(|(a, b)| a.max(b)),
        }
    }
}

impl<T: Discrete> Bounds<T> {
    /// Fold windows that overlap or touch into one; windows with a value between them stay apart.
    pub(crate) fn merge_all(mut windows: Vec<Self>) -> Vec<Self> {
        if windows.len() < 2 {
            return windows;
        }
        let count = windows.len();
        windows.sort_by(|left, right| left.minimum.cmp(&right.minimum));
        // `reaches` only holds left-to-right, so order the windows before folding.

        let mut merged: Vec<Self> = Vec::with_capacity(windows.len());
        for window in windows {
            match merged.last_mut() {
                Some(last) if last.reaches(&window) => {
                    *last = std::mem::take(last).hull(window);
                }
                _ => merged.push(window),
            }
        }
        debug_assert!(
            Self::is_canonical(&merged),
            "windows left unsorted or mergeable"
        );
        debug_assert!(merged.len() <= count, "merging invented a window");
        debug_assert!(!merged.is_empty(), "merging dropped every window");
        merged
    }

    /// Sorted by minimum, with no two neighbours left to merge.
    fn is_canonical(windows: &[Self]) -> bool {
        windows
            .windows(2)
            .all(|pair| pair[0].minimum <= pair[1].minimum && !pair[0].reaches(&pair[1]))
    }

    /// Whether `self` and a window starting no lower than it leave no value between them. The domain
    /// is discrete, so windows that merely touch (`..=5` and `6..`) also have nothing between.
    fn reaches(&self, next: &Self) -> bool {
        // Merging the pair takes their hull, which would invent values between two windows compared
        // the wrong way round.
        debug_assert!(
            self.minimum <= next.minimum,
            "windows compared out of order"
        );
        let (Some(end), Some(start)) = (self.maximum.as_ref(), next.minimum.as_ref()) else {
            return true;
        };
        end.clone()
            .checked_increment()
            .is_none_or(|above| *start <= above)
    }
}

/// A domain where each value has an immediate successor, so adjacent windows are contiguous.
pub(crate) trait Discrete: Ord + Clone {
    /// The next value up, or `None` at the top of the representable range.
    fn checked_increment(self) -> Option<Self>;
}

/// The bound present on both sides picked by `keep`; otherwise whichever side has one.
pub(crate) fn tighter<T>(
    first: Option<T>,
    second: Option<T>,
    keep: impl FnOnce(T, T) -> T,
) -> Option<T> {
    match (first, second) {
        (Some(a), Some(b)) => Some(keep(a, b)),
        (bound, None) | (None, bound) => bound,
    }
}

/// Drop every leaf whose values another already admits, where `subsumes(outer, inner)` says the
/// values of `inner` all lie in `outer`. A leaf already dropped neither drops nor saves another.
pub(crate) fn drop_subsumed<T>(leaves: &mut Vec<T>, subsumes: impl Fn(&T, &T) -> bool) {
    if leaves.len() < 2 {
        return;
    }
    let mut keep = vec![true; leaves.len()];
    for (index, leaf) in leaves.iter().enumerate() {
        for (other_index, other) in leaves.iter().enumerate() {
            if index == other_index || !keep[other_index] || !keep[index] {
                continue;
            }
            if subsumes(other, leaf) {
                keep[index] = false;
            }
        }
    }
    let mut index = 0;
    leaves.retain(|_| {
        let keeps = keep[index];
        index += 1;
        keeps
    });
}

pub(crate) type LengthBounds = Bounds<BoundCardinality>;
pub(crate) type IntegerBounds = Bounds<BoundInteger>;

impl SchemaKind {
    /// The admitted values when this node is a finite value set (`Const`/`Enum`), else `None`.
    #[must_use]
    pub(crate) fn finite_values(&self) -> Option<&[CanonicalJson]> {
        match self {
            SchemaKind::Const(value) => Some(std::slice::from_ref(value)),
            SchemaKind::Enum(values) => Some(values.as_slice()),
            SchemaKind::MultiType(_)
            | SchemaKind::TypedGroup { .. }
            | SchemaKind::String(_)
            | SchemaKind::Integer(_)
            | SchemaKind::Number(_)
            | SchemaKind::Array(_)
            | SchemaKind::Object(_)
            | SchemaKind::AnyOf(_)
            | SchemaKind::True
            | SchemaKind::False
            | SchemaKind::Raw(_) => None,
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
                Value::Number(_) | Value::String(_) | Value::Array(_) | Value::Object(_) => {
                    return None
                }
            };
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
