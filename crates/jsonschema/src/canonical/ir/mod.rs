use std::{
    cmp::Ordering,
    hash::{Hash, Hasher},
    sync::Arc,
};

use serde_json::{Number, Value};
use strum::{EnumDiscriminants, IntoStaticStr, VariantArray};

use crate::{
    keywords::format::{builtin_format, BuiltinFormat},
    Draft, JsonType, JsonTypeSet,
};

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
mod property_map;
mod raw;
mod string_leaves;
mod verdict;

pub(crate) use array_leaves::ArrayLeaves;
pub(crate) use bound_cardinality::BoundCardinality;
pub(crate) use bound_integer::{BoundInteger, Round};
pub(crate) use bound_number::{BoundNumber, Side};
pub(crate) use bound_rational::BoundRational;
pub(crate) use constructors::{canonicalize_value_set, type_set_schema, typed_group};
pub(crate) use divisors::{Divisors, ExcludedDivisors};
pub(crate) use integer_leaves::IntegerLeaves;
pub(crate) use number_leaves::NumberLeaves;
pub(crate) use object_leaves::ObjectLeaves;
pub(crate) use property_map::PropertyMap;
pub(crate) use raw::RawJson;
pub(crate) use string_leaves::StringLeaves;
pub(crate) use verdict::{UncheckableFacet, Verdict};

/// A format name kept inline for the built-in set and shared only when the draft does not recognize
/// it. Canonicalization preserves the latter because it cannot decide its assertion semantics.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum StringFormat {
    Builtin(BuiltinFormat),
    Unknown(Arc<str>),
}

impl StringFormat {
    #[must_use]
    pub(crate) fn from_name(draft: Draft, name: &str) -> Self {
        builtin_format(draft, name).map_or_else(|| Self::Unknown(Arc::from(name)), Self::Builtin)
    }

    #[must_use]
    pub(crate) fn as_str(&self) -> &str {
        match self {
            Self::Builtin(format) => format.as_str(),
            Self::Unknown(name) => name,
        }
    }

    #[must_use]
    pub(crate) fn length_window(&self) -> Option<(u64, u64)> {
        match self {
            Self::Builtin(format) => format.length_window(),
            Self::Unknown(_) => None,
        }
    }

    /// A string this format accepts, or `None` when the format is not one of the built-ins.
    #[must_use]
    pub(crate) fn witness(&self) -> Option<&'static str> {
        match self {
            Self::Builtin(format) => Some(format.witness()),
            Self::Unknown(_) => None,
        }
    }

    #[must_use]
    pub(crate) fn is_valid(&self, text: &str) -> Option<bool> {
        match self {
            Self::Builtin(format) => Some(format.is_valid(text)),
            Self::Unknown(_) => None,
        }
    }
}

impl PartialOrd for StringFormat {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for StringFormat {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_str().cmp(other.as_str())
    }
}

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

thread_local! {
    static TRUE: Schema = Schema::nullary(SchemaKind::True);
    static FALSE: Schema = Schema::nullary(SchemaKind::False);
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

    fn nullary(kind: SchemaKind) -> Self {
        let hash = structural_hash(&kind);
        Self(Arc::new(SchemaData { kind, hash }))
    }

    /// Matches every value. Handed out from one node per thread: over half the tree is this, and no
    /// two mentions of it differ.
    #[must_use]
    pub(crate) fn truthy() -> Self {
        TRUE.with(Clone::clone)
    }

    /// Matches no value, shared for the same reason as [`Schema::truthy`].
    #[must_use]
    pub(crate) fn falsy() -> Self {
        FALSE.with(Clone::clone)
    }

    #[inline]
    #[must_use]
    pub(crate) fn kind(&self) -> &SchemaKind {
        &self.0.kind
    }

    #[inline]
    #[must_use]
    pub(crate) fn cached_hash(&self) -> u64 {
        self.0.hash
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
    derive(Hash, IntoStaticStr, VariantArray),
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
    /// The exact complement of an opaque schema, keeping the references it names symbolic.
    Not(Schema),
    /// A value matches iff every opaque branch matches.
    AllOf(AtLeastTwo<Schema>),
    /// A value matches iff at least one of the sorted, mutually unmergeable branches matches.
    AnyOf(AtLeastTwo<Schema>),
    /// A value matches iff exactly one branch matches; sorted branches retain duplicates because multiplicity is semantic.
    OneOf(Vec<Schema>),
    /// A static `$ref` kept symbolic. Its target, when known, lives in `CanonicalSchema::definitions`.
    Reference(Arc<str>),
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
    /// Divisors no admitted value is a multiple of.
    pub(crate) not_multiple_of: ExcludedDivisors,
    /// No admitted value is one of the draft's integers. Survives only under Draft 4, whose
    /// token integers no divisor can name; later drafts respell it as a barred divisor of one.
    pub(crate) excludes_integers: bool,
}

impl NumberLeaf {
    /// Whether no real value fits between the two ends.
    pub(crate) fn is_vacant(&self) -> bool {
        // A demanded divisor whose multiples are all barred leaves nothing.
        if self.not_multiple_of.conflicts(&self.multiple_of) {
            return true;
        }
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
    /// Divisors no admitted value is a multiple of.
    pub(crate) not_multiple_of: ExcludedDivisors,
}

impl MaybeEmpty for IntegerLeaf {
    fn is_empty(&self) -> bool {
        self.bounds.is_empty()
            || self.not_multiple_of.conflicts(&self.multiple_of)
            // Every integer is a multiple of one, so barring a divisor that covers the whole grid
            // leaves no integer.
            || self.not_multiple_of.empties_integers()
    }
}

/// What an array leaf requires of its elements: all distinct, some repeated, or neither.
///
/// Exhaustive on purpose: a new state must break every consumer that reads it, the bindings
/// included, rather than reaching a runtime fallback.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, IntoStaticStr, VariantArray,
)]
#[strum(serialize_all = "snake_case")]
pub enum Distinctness {
    /// Elements may repeat or not.
    #[default]
    Unconstrained,
    /// No two elements are the same value.
    AllDistinct,
    /// Two elements are the same value.
    SomeRepeated,
}

/// The constraints a [`SchemaKind::Array`] places on an array value. An array of at most one item
/// is distinct on its own, so distinctness is demanded only when the window admits a second item.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub(crate) struct ArrayLeaf {
    pub(crate) lengths: LengthBounds,
    pub(crate) distinctness: Distinctness,
    /// Per-index schemas: the element at position `i` must satisfy `prefix[i]`.
    pub(crate) prefix: Vec<Schema>,
    /// The schema every element from `prefix.len()` onward must satisfy.
    pub(crate) items: Option<Schema>,
    /// Existential demands: the number of elements matching each facet's schema sits in its window.
    pub(crate) contains: Vec<ContainsFacet>,
}

impl ArrayLeaf {
    /// Whether every array satisfies this leaf, which makes it the `array` type written longhand.
    /// Every facet has to be listed here, or a union folds the leaf into the type set and drops it.
    pub(crate) fn spans_domain(&self) -> bool {
        self.lengths.is_unbounded()
            && matches!(self.distinctness, Distinctness::Unconstrained)
            && self.prefix.is_empty()
            && self.items.is_none()
            && self.contains.is_empty()
    }
}

/// One `contains` demand: how many elements match `schema`. An absent minimum spells the draft
/// default of one; an explicit one is normalized to absent.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ContainsFacet {
    pub(crate) schema: Schema,
    pub(crate) minimum: Option<BoundCardinality>,
    pub(crate) maximum: Option<BoundCardinality>,
}

impl ContainsFacet {
    /// The smallest matching count the facet admits.
    pub(crate) fn effective_minimum(&self) -> BoundCardinality {
        self.minimum
            .clone()
            .unwrap_or_else(|| BoundCardinality::from(1))
    }
}

impl MaybeEmpty for ArrayLeaf {
    fn is_empty(&self) -> bool {
        self.lengths.is_empty()
    }
}

/// One demand produced by negation: the object must hold at least one entry that breaks the
/// stored rule.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum ObjectViolation {
    /// Some key's name fails the schema.
    NameFails(Schema),
    /// Some key outside `names` and matching none of `patterns` has a value failing `additional`.
    UndeclaredValueFails {
        names: Vec<Arc<str>>,
        patterns: Vec<Arc<str>>,
        additional: Schema,
    },
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
    pub(crate) properties: PropertyMap,
    /// The schema every key matching the pattern must satisfy when the object carries it.
    pub(crate) pattern_properties: PropertyMap,
    /// The schema every key `properties` does not name and no pattern in `pattern_properties`
    /// matches must satisfy; never `True` (normalized away).
    pub(crate) additional: Option<Schema>,
    /// Sorted, deduplicated. The object must break each of these rules.
    pub(crate) violations: Vec<ObjectViolation>,
}

impl ObjectLeaf {
    /// The number of keys that can actually be present, when the property-name schema admits a
    /// finite set: a key whose property schema admits no value can never appear, so it does not
    /// count.
    #[must_use]
    pub(crate) fn admitted_key_count(&self) -> Option<BoundCardinality> {
        let values = self.property_names.as_ref()?.kind().finite_values()?;
        // Only a key whose own schema admits nothing drops out of the count. Those keys are the ones
        // looked up in the name set, not the other way round: the property map hands them over in
        // ascending order, so the set answers all of them in one pass.
        let mut admitted = AscendingMembership::new(values);
        let barred = self
            .properties
            .iter()
            .filter(|(key, child)| matches!(child.kind(), SchemaKind::False) && admitted.holds(key))
            .count();
        Some(BoundCardinality::from((values.len() - barred) as u64))
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
            && self.additional.is_none()
            && self.violations.is_empty()
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
        // A demand needs a key present to break its rule, just as a required key needs one to be,
        // and a ceiling of zero leaves no slot for either.
        if ceiling.is_zero() && !self.violations.is_empty() {
            return true;
        }
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
    /// Sorted, deduplicated. A string must match none of these patterns. Only syntactic equality
    /// against `patterns` is decided, so a leaf can be spelled and still admit nothing.
    pub(crate) excluded_patterns: Vec<Arc<str>>,
    /// Sorted, deduplicated. A string must satisfy every format. Empty unless formats assert.
    pub(crate) formats: Vec<StringFormat>,
    /// Sorted, deduplicated. A string must satisfy none of these formats. Empty unless formats
    /// assert.
    pub(crate) excluded_formats: Vec<StringFormat>,
    /// Sorted, deduplicated. A string must satisfy every media type. Empty outside Draft 6/7, where
    /// `contentMediaType` is an annotation.
    pub(crate) content_media_types: Vec<Arc<str>>,
    /// Sorted, deduplicated. A string must satisfy every encoding. Empty outside Draft 6/7, where
    /// `contentEncoding` is an annotation.
    pub(crate) content_encodings: Vec<Arc<str>>,
    /// Sorted, deduplicated. A string must differ from every member. Members the rest of the leaf
    /// already rejects are dropped, so one value set has one form.
    pub(crate) excluded: Vec<Arc<str>>,
}

/// Sorted, deduplicated, and holding at least two elements; fewer collapses to a simpler node.
/// Two is what nearly every one of these holds, so that shape stays out of the allocator.
#[derive(Debug, Clone)]
pub(crate) enum AtLeastTwo<T> {
    Two([T; 2]),
    Many(Vec<T>),
}

/// String membership in a sorted value set, for strings handed over in ascending order: the cursor
/// never goes back, so a whole run of queries costs one pass over the set instead of one scan each.
pub(crate) struct AscendingMembership<'a> {
    values: &'a [CanonicalJson],
    cursor: usize,
}

impl<'a> AscendingMembership<'a> {
    pub(crate) fn new(values: &'a [CanonicalJson]) -> Self {
        Self { values, cursor: 0 }
    }

    pub(crate) fn holds(&mut self, text: &str) -> bool {
        while self.values.get(self.cursor).is_some_and(|value| {
            raw::compare_value_to_str(value.as_value(), text) == Ordering::Less
        }) {
            self.cursor += 1;
        }
        debug_assert!(
            self.cursor == 0
                || raw::compare_value_to_str(self.values[self.cursor - 1].as_value(), text)
                    == Ordering::Less,
            "a string arrived out of order, so the walk had already passed where it belongs"
        );
        self.values.get(self.cursor).is_some_and(|value| {
            raw::compare_value_to_str(value.as_value(), text) == Ordering::Equal
        })
    }
}

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
        Ok(Self::from_sorted(items))
    }
}

impl<T> AtLeastTwo<T> {
    fn from_sorted(mut items: Vec<T>) -> Self {
        if items.len() == 2 {
            let second = items.pop().expect("two elements");
            let first = items.pop().expect("two elements");
            return Self::Two([first, second]);
        }
        Self::Many(items)
    }
}

impl<T> AtLeastTwo<T> {
    pub(crate) fn as_slice(&self) -> &[T] {
        match self {
            Self::Two(items) => items,
            Self::Many(items) => items,
        }
    }

    pub(crate) fn into_vec(self) -> Vec<T> {
        match self {
            Self::Two([first, second]) => vec![first, second],
            Self::Many(items) => items,
        }
    }

    /// Split the last element off; the remainder still holds at least one.
    pub(crate) fn split_last(self) -> (Vec<T>, T) {
        match self {
            Self::Two([first, second]) => (vec![first], second),
            Self::Many(mut items) => {
                let last = items.pop().expect("at least two elements");
                (items, last)
            }
        }
    }
}

impl<T: PartialEq> PartialEq for AtLeastTwo<T> {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl<T: Eq> Eq for AtLeastTwo<T> {}

impl<T: PartialOrd> PartialOrd for AtLeastTwo<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.as_slice().partial_cmp(other.as_slice())
    }
}

impl<T: Ord> Ord for AtLeastTwo<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_slice().cmp(other.as_slice())
    }
}

impl<T: Hash> Hash for AtLeastTwo<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_slice().hash(state);
    }
}

impl<T> IntoIterator for AtLeastTwo<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.into_vec().into_iter()
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
    pub(crate) fn merge_all(windows: Vec<Self>) -> Vec<Self> {
        // The window bounds are all that decides membership here, so every value between two of them
        // is one the pair leaves out.
        Self::merge_all_across_vacant_gaps(windows, |_, _| false)
    }

    /// [`Bounds::merge_all`], folding a pair as well when `gap_is_vacant` reports that the values
    /// strictly between the two ends it is handed are all rejected anyway. Their hull then admits
    /// nothing the pair did not, so the windows are as good as touching.
    pub(crate) fn merge_all_across_vacant_gaps(
        mut windows: Vec<Self>,
        gap_is_vacant: impl Fn(&T, &T) -> bool,
    ) -> Vec<Self> {
        if windows.len() < 2 {
            return windows;
        }
        let count = windows.len();
        windows.sort_by(|left, right| left.minimum.cmp(&right.minimum));
        // `reaches` only holds left-to-right, so order the windows before folding.

        let mut merged: Vec<Self> = Vec::with_capacity(windows.len());
        for window in windows {
            match merged.last_mut() {
                Some(last) if last.reaches(&window, &gap_is_vacant) => {
                    *last = std::mem::take(last).hull(window);
                }
                _ => merged.push(window),
            }
        }
        debug_assert!(
            Self::is_canonical(&merged, &gap_is_vacant),
            "windows left unsorted or mergeable"
        );
        debug_assert!(merged.len() <= count, "merging invented a window");
        debug_assert!(!merged.is_empty(), "merging dropped every window");
        merged
    }

    /// Sorted by minimum, with no two neighbours left to merge.
    fn is_canonical(windows: &[Self], gap_is_vacant: &impl Fn(&T, &T) -> bool) -> bool {
        windows.windows(2).all(|pair| {
            pair[0].minimum <= pair[1].minimum && !pair[0].reaches(&pair[1], gap_is_vacant)
        })
    }

    /// Whether `self` and a window starting no lower than it leave no value between them. The domain
    /// is discrete, so windows that merely touch (`..=5` and `6..`) also have nothing between, and a
    /// gap `gap_is_vacant` empties counts as none either.
    fn reaches(&self, next: &Self, gap_is_vacant: &impl Fn(&T, &T) -> bool) -> bool {
        // Merging the pair takes their hull, which would invent values between two windows compared
        // the wrong way round.
        debug_assert!(
            self.minimum <= next.minimum,
            "windows compared out of order"
        );
        let (Some(end), Some(start)) = (self.maximum.as_ref(), next.minimum.as_ref()) else {
            return true;
        };
        if end
            .clone()
            .checked_increment()
            .is_none_or(|above| *start <= above)
        {
            return true;
        }
        // Only a genuine gap reaches here, so `gap_is_vacant` never sees an empty range.
        gap_is_vacant(end, start)
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
            | SchemaKind::Not(_)
            | SchemaKind::AllOf(_)
            | SchemaKind::AnyOf(_)
            | SchemaKind::OneOf(_)
            | SchemaKind::Reference(_)
            | SchemaKind::True
            | SchemaKind::False
            | SchemaKind::Raw(_) => None,
        }
    }

    /// The number of distinct values this node admits, when finite: `Const`/`Enum`, an integer
    /// window closed on both sides, or a type set drawn only from `null`/`boolean` - the only JSON
    /// types with a finite universe. A window whose count outgrows a `u64` counts as unbounded; a
    /// divisor is ignored, so the count is an upper bound rather than the exact size.
    #[must_use]
    pub(crate) fn finite_domain_size(&self) -> Option<u64> {
        if let Some(values) = self.finite_values() {
            return Some(values.len() as u64);
        }
        if let SchemaKind::Integer(leaf) = self {
            let bounds = &leaf.get().bounds;
            return bounds
                .minimum
                .as_ref()
                .zip(bounds.maximum.as_ref())
                .and_then(|(minimum, maximum)| minimum.span_to(maximum));
        }
        let SchemaKind::MultiType(set) = self else {
            return None;
        };
        let finite_types = JsonType::Null | JsonType::Boolean;
        if set.intersect(finite_types) != *set {
            return None;
        }
        let mut count = 0u64;
        if set.contains(JsonType::Null) {
            count += 1;
        }
        if set.contains(JsonType::Boolean) {
            count += 2;
        }
        Some(count)
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

    /// The type set `values` saturates - only `null` and `boolean` have finite universes. Callers
    /// pass at least two distinct values, so a lone `null` never arrives.
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
            BOTH_BOOLEANS => Some(JsonTypeSet::from(JsonType::Boolean)),
            ALL => Some(JsonType::Null | JsonType::Boolean),
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
