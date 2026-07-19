//! JSON Schema IR: interned, `Arc`-shared nodes.
//!
//! Every stage matches the [`Schema`] enum (`parse`, `emit`, and `view` each enumerate it in full).
//! Nodes carry a kind-mask and structural hash, so passes skip subtrees without relevant kinds and
//! interning makes `Arc::ptr_eq` the fixpoint test.

#![cfg_attr(
    not(feature = "arbitrary-precision"),
    allow(
        clippy::clone_on_copy,
        clippy::trivially_copy_pass_by_ref,
        clippy::wrong_self_convention
    )
)]
#![cfg_attr(feature = "arbitrary-precision", allow(clippy::cmp_owned))]

pub(crate) mod cardinality;
pub(crate) mod intern;

use std::{cmp::Ordering, fmt, sync::Arc};

use referencing::Uri;
use serde_json::Value;
use strum::{EnumCount, EnumDiscriminants, IntoStaticStr, VariantNames};

use crate::{
    canonical::{error::CanonicalizationError, intern::shared, leaves::Leaf},
    JsonType, JsonTypeSet,
};

mod bound_cardinality;
mod bound_fraction;
mod bound_integer;
mod raw;

pub(crate) use bound_cardinality::BoundCardinality;
pub(crate) use bound_fraction::BoundFraction;
pub(crate) use bound_integer::BoundInteger;
pub(crate) use raw::RawJson;

/// A closed/open interval shared by integer and number leaves; `T` is the bound carrier
/// (`BoundInteger` or `BoundFraction`). `None` on a side means unbounded there.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct Bounds<T> {
    pub(crate) minimum: Option<T>,
    pub(crate) maximum: Option<T>,
    pub(crate) exclusive_minimum: bool,
    pub(crate) exclusive_maximum: bool,
}

// Hand-written so the impl does not pick up a spurious `T: Default` bound from the derive.
impl<T> Default for Bounds<T> {
    fn default() -> Self {
        Self {
            minimum: None,
            maximum: None,
            exclusive_minimum: false,
            exclusive_maximum: false,
        }
    }
}

pub(crate) type IntegerBounds = Bounds<BoundInteger>;
pub(crate) type NumberBounds = Bounds<BoundFraction>;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum BooleanBounds {
    Any,
    JustTrue,
    JustFalse,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub(crate) struct IntegerLeaf {
    pub(crate) bounds: IntegerBounds,
    pub(crate) multiple_of: Option<BoundInteger>,
    /// Sorted, deduplicated. Value must be a multiple of none of these.
    pub(crate) not_multiple_of: Vec<BoundInteger>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub(crate) struct NumberLeaf {
    pub(crate) bounds: NumberBounds,
    pub(crate) multiple_of: Option<BoundFraction>,
    /// Sorted, deduplicated. Value must be a multiple of none of these.
    pub(crate) not_multiple_of: Vec<BoundFraction>,
}

/// Canonical-JSON text for `Const`/`Enum`. The inner string is always [`crate::canonical::json::to_string`] output;
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct CanonicalJson(Arc<str>);

impl CanonicalJson {
    /// # Panics
    ///
    /// Panics if the canonical writer fails - in practice only when the value nests deeper than `255` levels, which
    /// [`crate::canonical::parse`] already rejects.
    #[must_use]
    pub(crate) fn from_value(value: &Value) -> Self {
        Self::try_from_value(value).expect("canonical JSON serialisation within recursion limit")
    }

    pub(crate) fn try_from_value(value: &Value) -> Result<Self, CanonicalizationError> {
        crate::canonical::json::to_string(value)
            .map(|text| Self(text.into()))
            .map_err(|error| CanonicalizationError::InvalidJsonValue(error.to_string()))
    }

    /// Caller MUST have canonicalized `text`; arbitrary JSON breaks equality.
    #[must_use]
    pub(crate) fn from_canonical_text(text: Arc<str>) -> Self {
        debug_assert!(!text.is_empty(), "canonical text must be non-empty JSON");
        Self(text)
    }

    #[must_use]
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub(crate) fn as_arc(&self) -> &Arc<str> {
        &self.0
    }

    #[must_use]
    pub(crate) fn to_value(&self) -> Value {
        serde_json::from_str(self.as_str()).expect("CanonicalJson holds well-formed canonical JSON")
    }

    /// JSON type inferred from the canonical text. Normalisation converts integer-valued floats
    /// (`1.0` -> `"1"`); past-cap values keep the scientific normal form, classified by exponent.
    #[must_use]
    pub(crate) fn json_type(&self) -> JsonType {
        let text = self.as_str();
        match text
            .as_bytes()
            .first()
            .copied()
            .expect("Valid JSON is not empty")
        {
            b'n' if text == "null" => JsonType::Null,
            b't' if text == "true" => JsonType::Boolean,
            b'f' if text == "false" => JsonType::Boolean,
            b'"' => JsonType::String,
            b'[' => JsonType::Array,
            b'{' => JsonType::Object,
            b'-' | b'0'..=b'9' => {
                if number_text_is_integer(text) {
                    JsonType::Integer
                } else {
                    JsonType::Number
                }
            }
            _ => unreachable!("Always valid JSON"),
        }
    }
}

/// Whether canonical number text denotes an integer. Plain form: integer iff no `.`. Scientific
/// normal form `d[.f]e<exp>` (only past-cap values): the mantissa carries no trailing zeros, so
/// the value is an integer iff the exponent covers every fraction digit.
fn number_text_is_integer(text: &str) -> bool {
    let Some(exponent_at) = text.bytes().position(|b| matches!(b, b'e' | b'E')) else {
        return !text.as_bytes().contains(&b'.');
    };
    let mantissa = &text[..exponent_at];
    let exponent = &text[exponent_at + 1..];
    if exponent.starts_with('-') {
        return false;
    }
    let fraction_len = mantissa
        .bytes()
        .position(|b| b == b'.')
        .map_or(0, |dot| mantissa.len() - dot - 1);
    // An exponent past `u64` covers any fraction the text could physically hold.
    let Ok(exponent) = exponent.parse::<u64>() else {
        return true;
    };
    exponent >= fraction_len as u64
}

#[expect(
    clippy::struct_field_names,
    reason = "field names mirror the JSON Schema keywords (`contentEncoding`, etc.)"
)]
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub(crate) struct ContentFacet {
    pub(crate) content_encoding: Option<Arc<str>>,
    pub(crate) content_media_type: Option<Arc<str>>,
    pub(crate) content_schema: Option<CanonicalJson>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub(crate) struct StringLeaf {
    pub(crate) min_length: Option<BoundCardinality>,
    pub(crate) max_length: Option<BoundCardinality>,
    pub(crate) patterns: Vec<Arc<str>>,
    /// Sorted, deduplicated. String must match none of these patterns.
    pub(crate) not_patterns: Vec<Arc<str>>,
    pub(crate) format: Option<Arc<str>>,
    pub(crate) content: Vec<ContentFacet>,
}

impl StringLeaf {
    /// Whether any positive or negative pattern needs the extended (fancy) regex engine under the
    /// context's pattern options. Derived on demand, not stored, so it never drifts across the
    /// pattern edits intersection/negation perform.
    #[must_use]
    pub(crate) fn extended_regex(
        &self,
        ctx: &crate::canonical::context::CanonicalizationContext,
    ) -> bool {
        self.patterns
            .iter()
            .chain(&self.not_patterns)
            .any(|pattern| ctx.pattern_is_extended(pattern))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub(crate) struct LengthBounds {
    pub(crate) minimum: BoundCardinality,
    pub(crate) maximum: Option<BoundCardinality>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ContainsClause {
    pub(crate) schema: SharedSchema,
    pub(crate) min_contains: BoundCardinality,
    pub(crate) max_contains: Option<BoundCardinality>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ArrayLeaf {
    pub(crate) prefix: Vec<SharedSchema>,
    pub(crate) tail: SharedSchema,
    pub(crate) length: LengthBounds,
    pub(crate) unique_items: bool,
    /// Array has at least one duplicated pair (the negation of `unique_items`).
    pub(crate) repeated_items: bool,
    pub(crate) contains: Vec<ContainsClause>,
}

impl Default for ArrayLeaf {
    /// The open array: any items, any length.
    fn default() -> Self {
        ArrayLeaf {
            prefix: Vec::new(),
            tail: shared(Schema::True),
            length: LengthBounds::default(),
            unique_items: false,
            repeated_items: false,
            contains: Vec::new(),
        }
    }
}

impl ArrayLeaf {
    /// Normalize invariants implied by `repeated_items`; `None` means the duplicate requirement is unsatisfiable.
    pub(crate) fn normalize_repeated_items(mut self) -> Option<Self> {
        if !self.repeated_items {
            return Some(self);
        }
        let two = BoundCardinality::from(2_u8);
        if self.unique_items
            || self
                .length
                .maximum
                .as_ref()
                .is_some_and(|maximum| maximum < &two)
        {
            return None;
        }
        if self.length.minimum < two {
            self.length.minimum = two;
        }
        Some(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum ObjectRequirement {
    RequiredProperty(Arc<str>),
    /// At least one property whose name matches `matcher` has a value satisfying `schema`. Existential dual of
    /// [`ObjectConstraint`]; `matcher` may be [`PropertyNameMatcher::AdditionalProperties`].
    PatternPropertyRequirement {
        matcher: PropertyNameMatcher,
        schema: SharedSchema,
    },
    DependentPropertiesRequirement {
        property: Arc<str>,
        required_properties: Vec<Arc<str>>,
    },
    DependentSchemaRequirement {
        property: Arc<str>,
        schema: SharedSchema,
    },
    MinProperties(BoundCardinality),
    MaxProperties(BoundCardinality),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ObjectConstraint {
    pub(crate) matcher: PropertyNameMatcher,
    pub(crate) schema: SharedSchema,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum PropertyNameMatcher {
    NamedProperty(Arc<str>),
    PatternProperty(Arc<str>),
    AdditionalProperties,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub(crate) struct ObjectLeaf {
    pub(crate) requirements: Vec<ObjectRequirement>,
    pub(crate) constraints: Vec<ObjectConstraint>,
    pub(crate) property_names: Option<SharedSchema>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct OneOf(pub Vec<SharedSchema>);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct IfThenElse {
    pub(crate) condition: SharedSchema,
    pub(crate) then_branch: Option<SharedSchema>,
    pub(crate) else_branch: Option<SharedSchema>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, EnumDiscriminants)]
#[strum_discriminants(
    name(CanonicalKind),
    vis(pub),
    derive(Hash, EnumCount, VariantNames, IntoStaticStr),
    strum(serialize_all = "snake_case"),
    doc = "Cheap structural discriminant of a [`CanonicalSchema`](crate::CanonicalSchema). One arm per IR variant. Use [`CanonicalSchema::view`](crate::CanonicalSchema::view) to read the node's data."
)]
pub(crate) enum Schema {
    Null,
    Boolean(BooleanBounds),
    Integer(IntegerLeaf),
    Number(NumberLeaf),
    String(StringLeaf),
    Array(ArrayLeaf),
    Object(ObjectLeaf),
    /// A value matches iff its JSON type is in the set. Canonical form of `{"type": [...]}` with no per-branch
    /// constraint; `Integer` drops when `Number` is present (every integer is a number).
    MultiType(JsonTypeSet),
    AllOf(Vec<SharedSchema>),
    AnyOf(Vec<SharedSchema>),
    OneOf(OneOf),
    Not(SharedSchema),
    IfThenElse(IfThenElse),
    /// Typed conjunction: a value matches iff its JSON type is `ty` *and* it satisfies `body`; other types do not
    /// match. One branch per entry in `{"type": [...]}`. Pins a kind (see `Schema::pinned_kind`).
    TypedGroup {
        ty: JsonType,
        body: SharedSchema,
    },
    /// Typed implication (`¬ty ∨ body`): type-`ty` values are constrained by `body`, all other types match
    /// unconditionally. Produced by kind-restricted keywords without an explicit `type`. Does *not* pin a kind.
    TypeGuard {
        ty: JsonType,
        body: SharedSchema,
    },
    Const(CanonicalJson),
    Enum(Vec<CanonicalJson>),
    True,
    False,
    /// External `$ref` URI; in-document refs are inlined at parse time.
    Reference(Uri<String>),
    /// Cycle binding name.
    Recursive(Arc<str>),
    /// Resolved at strategy time.
    DynamicRef(Arc<str>),
    /// A schema object whose validation semantics are preserved verbatim because they depend on draft features the
    /// structural IR does not model.
    Raw(RawJson),
}

/// Reference-counted handle to an interned [`SchemaNode`].
pub(crate) type SharedSchema = Arc<SchemaNode>;

/// A [`Schema`] paired with the [`SchemaKindSet`] of variants present anywhere in its subtree. The mask is computed at
/// `node` time so passes can skip whole subtrees with a single bitmask test.
#[derive(Debug, Clone)]
pub(crate) struct SchemaNode {
    pub(crate) schema: Schema,
    pub(crate) mask: SchemaKindSet,
    /// Structural hash of `schema`, cached at construction. Equality compares it first so it
    /// short-circuits instead of walking the whole subtree; a deep compare only runs on a hash tie.
    /// Ordering cannot use it (hash order is not structural order) and always compares deeply.
    pub(crate) hash: u64,
    /// Saturating tree size (shared subtrees counted per occurrence) - the cost of tree-shaped work.
    pub(crate) size: u32,
}

impl PartialEq for SchemaNode {
    fn eq(&self, other: &Self) -> bool {
        // Pointer equality is sufficient (same allocation = same content).
        std::ptr::eq(self, other) || (self.hash == other.hash && self.schema == other.schema)
    }
}

impl Eq for SchemaNode {}

impl PartialOrd for SchemaNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SchemaNode {
    fn cmp(&self, other: &Self) -> Ordering {
        if std::ptr::eq(self, other) {
            return Ordering::Equal;
        }
        self.schema.cmp(&other.schema)
    }
}

impl std::hash::Hash for SchemaNode {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write_u64(self.hash);
    }
}

impl SchemaNode {
    #[inline]
    #[must_use]
    pub(crate) fn as_schema(&self) -> &Schema {
        &self.schema
    }

    /// Extract the type tag and body of any single-type canonical node.
    ///
    /// Returns `None` for multi-type nodes (`AnyOf`, `AllOf`, …) and schema-logic nodes (`True`, `False`, `Not`, …).
    pub(crate) fn as_typed_view(self: &Arc<Self>) -> Option<TypedView> {
        if let Schema::TypedGroup { ty, body } = self.as_schema() {
            return Some(TypedView {
                ty: *ty,
                schema: Arc::clone(body),
            });
        }
        // `pinned_kind` also returns `Some` for `TypedGroup`, but that arm is handled above.
        Some(TypedView {
            ty: self.as_schema().pinned_kind()?,
            schema: Arc::clone(self),
        })
    }
}

/// A typed view over a canonical schema node: its JSON type tag and the body node (unwrapped out of `TypedGroup`
/// if present).
pub(crate) struct TypedView {
    pub(crate) ty: JsonType,
    pub(crate) schema: SharedSchema,
}

impl std::ops::Deref for SchemaNode {
    type Target = Schema;
    fn deref(&self) -> &Schema {
        &self.schema
    }
}

/// Bit set over [`Schema`] variants present anywhere in a subtree. Each bit is `1 << (CanonicalKind as u32)`, so the
/// set stays exhaustive over the IR by construction.
#[derive(Copy, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct SchemaKindSet(u32);

const _: () = assert!(CanonicalKind::COUNT <= u32::BITS as usize);

impl fmt::Debug for SchemaKindSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SchemaKindSet{{")?;
        let mut bits = self.0;
        let mut first = true;
        while bits != 0 {
            let index = bits.trailing_zeros() as usize;
            bits &= bits - 1;
            if !first {
                write!(f, ", ")?;
            }
            first = false;
            f.write_str(CanonicalKind::VARIANTS[index])?;
        }
        write!(f, "}}")
    }
}

impl SchemaKindSet {
    #[must_use]
    pub(crate) const fn empty() -> Self {
        Self(0)
    }

    /// The set holding exactly `kind`.
    #[must_use]
    pub(crate) const fn of(kind: CanonicalKind) -> Self {
        Self(1 << (kind as u32))
    }

    /// The set holding every kind in `kinds`. `const` so module-level masks can be built without a runtime initializer.
    #[must_use]
    pub(crate) const fn from_kinds(kinds: &[CanonicalKind]) -> Self {
        let mut bits = 0;
        let mut index = 0;
        while index < kinds.len() {
            bits |= 1 << (kinds[index] as u32);
            index += 1;
        }
        Self(bits)
    }

    /// True when no kind is shared. Accepts a [`CanonicalKind`] or another set.
    #[must_use]
    pub(crate) fn is_disjoint(self, other: impl Into<Self>) -> bool {
        (self.0 & other.into().0) == 0
    }

    #[must_use]
    pub(crate) const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl From<CanonicalKind> for SchemaKindSet {
    fn from(kind: CanonicalKind) -> Self {
        Self::of(kind)
    }
}

impl Schema {
    /// Bit for this variant alone - no recursion into children.
    #[must_use]
    pub(crate) fn variant_bit(&self) -> SchemaKindSet {
        SchemaKindSet::of(self.into())
    }

    /// Drop redundant entries from a type set: `Integer` is a subset of `Number`, so when both are present the integer
    /// entry is removed.
    #[must_use]
    pub(crate) fn canonical_type_set(set: JsonTypeSet) -> JsonTypeSet {
        if set.contains(JsonType::Number) {
            set.remove(JsonType::Integer)
        } else {
            set
        }
    }

    /// Type-set complement honoring `Integer ⊆ Number`. `None` when the complement isn't a type set - the original
    /// has `Integer` but not `Number`, since "non-integer numbers" cannot be a type tag.
    #[must_use]
    pub(crate) fn type_set_complement(set: JsonTypeSet) -> Option<JsonTypeSet> {
        let canon = Self::canonical_type_set(set);
        if canon.contains(JsonType::Integer) {
            return None;
        }
        Some(Self::canonical_type_set(
            Self::semantic_cover(canon).complement(),
        ))
    }

    /// Type-set intersection honoring `Integer ⊆ Number`. Each input is first expanded with `Integer` when `Number` is
    /// present, then intersected.
    #[must_use]
    pub(crate) fn type_set_intersect(left: JsonTypeSet, right: JsonTypeSet) -> JsonTypeSet {
        Self::canonical_type_set(Self::semantic_cover(left).intersect(Self::semantic_cover(right)))
    }

    /// Type-set difference honoring `Integer ⊆ Number`. Returns `None` when the result requires expressing "non-integer
    /// numbers" - i.e. when the desired semantic set contains `Number` but not `Integer`.
    #[must_use]
    pub(crate) fn type_set_subtract(left: JsonTypeSet, right: JsonTypeSet) -> Option<JsonTypeSet> {
        let desired =
            Self::semantic_cover(left).intersect(Self::semantic_cover(right).complement());
        if desired.contains(JsonType::Number) && !desired.contains(JsonType::Integer) {
            return None;
        }
        Some(Self::canonical_type_set(desired))
    }

    /// Expand a type set to its semantic cover: `Number` implies `Integer`. Used before set-theoretic ops that need to
    /// respect the subtype.
    #[must_use]
    pub(crate) fn semantic_cover(set: JsonTypeSet) -> JsonTypeSet {
        if set.contains(JsonType::Number) {
            set.insert(JsonType::Integer)
        } else {
            set
        }
    }

    #[must_use]
    pub(crate) fn type_set_contains_kind(set: JsonTypeSet, kind: JsonType) -> bool {
        Self::semantic_cover(set).contains(kind)
    }

    #[must_use]
    pub(crate) fn type_set_covers(big: JsonTypeSet, small: JsonTypeSet) -> bool {
        let big_expanded = Self::semantic_cover(big);
        small.iter().all(|kind| big_expanded.contains(kind))
    }

    #[must_use]
    pub(crate) fn finite_values(&self) -> Option<&[CanonicalJson]> {
        match self {
            Self::Const(value) => Some(std::slice::from_ref(value)),
            Self::Enum(values) => Some(values.as_slice()),
            _ => None,
        }
    }

    #[must_use]
    pub(crate) fn finite_value_type_domain(values: &[CanonicalJson]) -> JsonTypeSet {
        values.iter().fold(JsonTypeSet::empty(), |set, value| {
            set.insert(value.json_type())
        })
    }

    /// The type set `values` saturates - every inhabitant of each returned type is present. Only
    /// `null` and `boolean` have finite universes, so the only saturable sets are `{null}`,
    /// `{false, true}`, and `{null, false, true}`.
    #[must_use]
    pub(crate) fn finite_values_saturated_domain(values: &[CanonicalJson]) -> Option<JsonTypeSet> {
        let mut bits: u8 = 0;
        for value in values {
            bits |= match value.as_str() {
                "null" => 1,
                "false" => 2,
                "true" => 4,
                _ => return None,
            };
        }
        // Distinctness check: a duplicated member means some inhabitant is missing.
        if bits.count_ones() as usize != values.len() {
            return None;
        }
        match bits {
            1 => Some(JsonTypeSet::from(JsonType::Null)),
            6 => Some(JsonTypeSet::from(JsonType::Boolean)),
            7 => Some(JsonTypeSet::from(JsonType::Null).insert(JsonType::Boolean)),
            _ => None,
        }
    }

    #[must_use]
    pub(crate) fn finite_values_saturate_type(values: &[CanonicalJson]) -> Option<JsonType> {
        match Self::finite_values_saturated_domain(values)? {
            set if set == JsonTypeSet::from(JsonType::Null) => Some(JsonType::Null),
            set if set == JsonTypeSet::from(JsonType::Boolean) => Some(JsonType::Boolean),
            _ => None,
        }
    }

    /// If this schema is equivalent to `{"type": [...]}` with no additional constraints, return the set of types it
    /// admits. Drives the AnyOf-fold normalization and emit-time type-array recognition.
    #[must_use]
    pub(crate) fn as_type_set(&self) -> Option<JsonTypeSet> {
        if let Some(kind) = self.open_kind() {
            return Some(JsonTypeSet::from(kind));
        }
        match self {
            Self::Const(value) => Self::finite_values_saturated_domain(std::slice::from_ref(value)),
            Self::Enum(values) => Self::finite_values_saturated_domain(values),
            Self::MultiType(set) => Some(*set),
            _ => None,
        }
    }

    #[must_use]
    pub(crate) fn type_domain_upper_bound(&self) -> Option<JsonTypeSet> {
        self.as_type_set()
            .or_else(|| self.finite_values().map(Self::finite_value_type_domain))
            .or_else(|| self.pinned_kind().map(JsonTypeSet::from))
    }

    #[must_use]
    pub(crate) fn direct_type_domain(&self) -> Option<JsonTypeSet> {
        self.as_type_set()
            .or_else(|| self.pinned_kind().map(JsonTypeSet::from))
    }

    /// Upper bound on the JSON types this schema admits, when determinable. Recurses through `AnyOf`
    /// (union of members) and `AllOf` (intersection); `None` means unconstrained (the universe).
    #[must_use]
    pub(crate) fn type_domain(&self) -> Option<JsonTypeSet> {
        if let Some(set) = self.direct_type_domain() {
            return Some(set);
        }
        match self {
            Self::AnyOf(branches) => {
                let mut union = JsonTypeSet::empty();
                for branch in branches {
                    union = union.union(branch.as_schema().type_domain()?);
                }
                Some(union)
            }
            Self::AllOf(branches) => {
                let mut domain: Option<JsonTypeSet> = None;
                for branch in branches {
                    if let Some(set) = branch.as_schema().type_domain() {
                        domain = Some(match domain {
                            Some(prev) => Self::type_set_intersect(prev, set),
                            None => set,
                        });
                    }
                }
                domain
            }
            _ => None,
        }
    }

    #[must_use]
    pub(crate) fn single_type_domain(&self) -> Option<JsonType> {
        self.pinned_kind().or_else(|| match self.finite_values()? {
            [only] => Some(only.json_type()),
            _ => None,
        })
    }

    /// The JSON type when this schema is the unconstrained form of a single type - `null`, any boolean, or an open
    /// typed leaf - i.e. it admits every value of exactly that type and nothing else. `None` otherwise.
    #[must_use]
    fn open_kind(&self) -> Option<JsonType> {
        match self {
            Self::Null => Some(JsonType::Null),
            Self::Boolean(BooleanBounds::Any) => Some(JsonType::Boolean),
            Self::Integer(leaf) if leaf.is_open() => Some(JsonType::Integer),
            Self::Number(leaf) if leaf.is_open() => Some(JsonType::Number),
            Self::String(leaf) if leaf.is_open() => Some(JsonType::String),
            Self::Array(leaf) if leaf.is_open() => Some(JsonType::Array),
            Self::Object(leaf) if leaf.is_open() => Some(JsonType::Object),
            _ => None,
        }
    }

    #[must_use]
    pub(crate) fn single_unconstrained_type(&self) -> Option<JsonType> {
        match self {
            Self::TypedGroup { ty, body } => match body.as_schema() {
                Self::True => Some(*ty),
                other => other.accepts_all_values_of_same_type(*ty).then_some(*ty),
            },
            Self::Const(value) => Self::finite_values_saturate_type(std::slice::from_ref(value)),
            Self::Enum(values) => Self::finite_values_saturate_type(values),
            _ => self.open_kind(),
        }
    }

    fn accepts_all_values_of_same_type(&self, kind: JsonType) -> bool {
        if self.open_kind() == Some(kind) {
            return true;
        }
        let saturated = match self {
            Self::Const(value) => Self::finite_values_saturate_type(std::slice::from_ref(value)),
            Self::Enum(values) => Self::finite_values_saturate_type(values),
            _ => return false,
        };
        saturated == Some(kind)
    }

    /// JSON type tag of a direct-form variant. Returns `None` for `Const`, `Enum`, and schema-logic variants (`AllOf`,
    /// `AnyOf`, ...). Callers that need to classify those handle them separately.
    #[must_use]
    pub(crate) fn pinned_kind(&self) -> Option<JsonType> {
        match self {
            Self::Null => Some(JsonType::Null),
            Self::Boolean(_) => Some(JsonType::Boolean),
            Self::Integer(_) => Some(JsonType::Integer),
            Self::Number(_) => Some(JsonType::Number),
            Self::String(_) => Some(JsonType::String),
            Self::Array(_) => Some(JsonType::Array),
            Self::Object(_) => Some(JsonType::Object),
            // `TypeGuard { ty, body }` accepts *all* non-`ty` values too, so it does not pin a kind.
            Self::TypedGroup { ty, .. } => Some(*ty),
            _ => None,
        }
    }

    /// Whether this schema is a direct typed *leaf* of `kind`: a [`pinned_kind`](Self::pinned_kind) match that,
    /// unlike `TypedGroup`, carries no extra acceptance. The source of truth for leaf-variant <-> [`JsonType`].
    #[must_use]
    pub(crate) fn is_typed_leaf_of(&self, kind: JsonType) -> bool {
        !matches!(self, Self::TypedGroup { .. }) && self.pinned_kind() == Some(kind)
    }

    /// Visit schema-shaped direct children only - never bound names or leaf payloads. Does not allocate.
    pub(crate) fn for_each_child(&self, mut visit: impl FnMut(&SharedSchema)) {
        match self {
            Self::Null
            | Self::Boolean(_)
            | Self::Integer(_)
            | Self::Number(_)
            | Self::String(_)
            | Self::Const(_)
            | Self::Enum(_)
            | Self::True
            | Self::False
            | Self::Reference(_)
            | Self::Recursive(_)
            | Self::DynamicRef(_)
            | Self::Raw(_)
            | Self::MultiType(_) => {}
            Self::Array(leaf) => {
                for item in &leaf.prefix {
                    visit(item);
                }
                visit(&leaf.tail);
                for clause in &leaf.contains {
                    visit(&clause.schema);
                }
            }
            Self::Object(leaf) => {
                for requirement in &leaf.requirements {
                    match requirement {
                        ObjectRequirement::DependentSchemaRequirement { schema, .. }
                        | ObjectRequirement::PatternPropertyRequirement { schema, .. } => {
                            visit(schema);
                        }
                        _ => {}
                    }
                }
                for constraint in &leaf.constraints {
                    visit(&constraint.schema);
                }
                if let Some(value) = &leaf.property_names {
                    visit(value);
                }
            }
            Self::AllOf(branches) | Self::AnyOf(branches) | Self::OneOf(OneOf(branches)) => {
                for branch in branches {
                    visit(branch);
                }
            }
            Self::Not(inner) => visit(inner),
            Self::IfThenElse(IfThenElse {
                condition,
                then_branch,
                else_branch,
            }) => {
                visit(condition);
                if let Some(value) = then_branch {
                    visit(value);
                }
                if let Some(value) = else_branch {
                    visit(value);
                }
            }
            Self::TypedGroup { body, .. } | Self::TypeGuard { body, .. } => visit(body),
        }
    }

    /// Schema-shaped direct children only - never bound names or leaf payloads. Allocates on every call.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn children(&self) -> Vec<SharedSchema> {
        let mut out = Vec::new();
        self.for_each_child(|child| out.push(Arc::clone(child)));
        out
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use test_case::test_case;

    use super::CanonicalJson;
    use crate::JsonType;

    // Scientific normal form (`d[.f]e<exp>`, mantissa without trailing zeros) denotes an integer
    // iff the exponent covers every fraction digit.
    #[test_case("1e1048577" => JsonType::Integer ; "plain mantissa positive exponent")]
    #[test_case("1.5e300" => JsonType::Integer ; "fraction covered by exponent")]
    #[test_case("1.5e1" => JsonType::Integer ; "fraction exactly covered")]
    #[test_case("1.51e1" => JsonType::Number ; "fraction past exponent")]
    #[test_case("1e-1048577" => JsonType::Number ; "negative exponent")]
    #[test_case("-2.5e12" => JsonType::Integer ; "negative mantissa")]
    #[test_case("42" => JsonType::Integer ; "plain integer")]
    #[test_case("4.2" => JsonType::Number ; "plain decimal")]
    fn json_type_classifies_number_text(text: &str) -> JsonType {
        CanonicalJson::from_canonical_text(std::sync::Arc::from(text)).json_type()
    }

    // Empty text is not JSON; a `CanonicalJson` holding it breaks `json_type` and `parse_canonical`.
    #[test]
    #[should_panic(expected = "canonical text must be non-empty JSON")]
    fn from_canonical_text_rejects_empty_in_debug() {
        let _ = CanonicalJson::from_canonical_text(std::sync::Arc::from(""));
    }

    // The `SchemaKindSet` Debug impl is reached only through the derived `CanonicalSchema`/`SchemaNode` Debug.
    // A multi-kind root exercises both the first-entry path and the comma separator.
    #[test]
    fn kind_set_debug_renders_subtree_mask() {
        let schema = crate::canonicalize(&json!({
            "type": "object",
            "properties": {"a": {"type": "integer"}}
        }))
        .expect("canonicalize");
        let rendered = format!("{schema:?}");
        // Assert the exact multi-kind mask rendering, so the `, ` is verified as the mask's own separator,
        // not one of the incidental `, ` the derived Debug emits for other fields.
        assert!(rendered.contains("SchemaKindSet{integer, object}"));
    }
}
