use std::{cmp::Ordering, str::FromStr};

use serde_json::Number;

use crate::canonical::{
    context::CanonicalizationContext,
    ir::{BoundFraction, BoundInteger, NumberLeaf, Schema, SharedSchema},
    leaves::{Intersection, Leaf, TypedLeaf, Verdict},
    numeric::numeric_leaf_covers,
    prover::Prover,
};

use super::{bounds::is_empty_interval, intersect_numeric_leaf};

impl Leaf for NumberLeaf {
    /// `Residual` signals an unrepresentable combined `multipleOf`; the caller keeps both leaves in
    /// an `AllOf`.
    ///
    /// ```text
    /// BEFORE: {"type": "number", "minimum": 0.5}  and  {"type": "number", "maximum": 2.5}
    /// AFTER:  {"type": "number", "minimum": 0.5, "maximum": 2.5}
    ///
    /// BEFORE: {"type": "number", "multipleOf": 0.5}  and  {"type": "number", "multipleOf": 0.25}
    /// AFTER:  {"type": "number", "multipleOf": 0.5}
    /// ```
    fn intersect(&self, other: &Self, _ctx: &CanonicalizationContext) -> Intersection<Self> {
        intersect_numeric_leaf(self, other)
    }

    fn covers(&self, other: &Self, _prover: &Prover<'_>) -> Verdict {
        Verdict::proven_if(numeric_leaf_covers(self, other))
    }

    fn inhabited(&self, _formats_asserted: bool) -> Verdict {
        Verdict::Proven
    }

    fn is_open(&self) -> bool {
        self.bounds.minimum.is_none()
            && self.bounds.maximum.is_none()
            && self.multiple_of.is_none()
            && self.not_multiple_of.is_empty()
    }

    fn is_empty(&self, _ctx: &CanonicalizationContext) -> bool {
        is_empty_interval(
            self.bounds.minimum.as_ref(),
            self.bounds.exclusive_minimum,
            self.bounds.maximum.as_ref(),
            self.bounds.exclusive_maximum,
        ) || number_multiple_of_misses_interval(self)
    }
}

impl TypedLeaf for NumberLeaf {
    fn wrap(self) -> Schema {
        Schema::Number(self)
    }
    fn project(schema: &Schema) -> Option<&Self> {
        match schema {
            Schema::Number(leaf) => Some(leaf),
            _ => None,
        }
    }
}

fn number_multiple_of_misses_interval(leaf: &NumberLeaf) -> bool {
    let Some(modulus) = leaf.multiple_of.as_ref() else {
        return false;
    };
    if modulus.is_zero() {
        return false;
    }
    let (Some(minimum), Some(maximum)) =
        (leaf.bounds.minimum.as_ref(), leaf.bounds.maximum.as_ref())
    else {
        return false;
    };
    if minimum.cmp(maximum) == Ordering::Greater {
        return true;
    }
    let modulus = modulus.abs();
    let (Some(mut minimum_index), Some(mut maximum_index)) =
        (minimum.ceil_div(&modulus), maximum.floor_div(&modulus))
    else {
        return false;
    };
    if leaf.bounds.exclusive_minimum && hits_exactly(minimum, &modulus, minimum_index.owned()) {
        minimum_index += BoundInteger::one();
    }
    if leaf.bounds.exclusive_maximum && hits_exactly(maximum, &modulus, maximum_index.owned()) {
        maximum_index -= BoundInteger::one();
    }
    minimum_index > maximum_index
}

fn hits_exactly(value: &BoundFraction, modulus: &BoundFraction, index: BoundInteger) -> bool {
    let product = modulus * BoundFraction::from(index);
    product.cmp(value) == Ordering::Equal
}

/// True when intersecting the two number leaves would yield an unrepresentable combined `multipleOf` and bail back to
/// `AllOf`.
pub(crate) fn is_unsafe_number_pair(left: &SharedSchema, right: &SharedSchema) -> bool {
    let (Schema::Number(left), Schema::Number(right)) = (left.as_schema(), right.as_schema())
    else {
        return false;
    };
    match (left.multiple_of.as_ref(), right.multiple_of.as_ref()) {
        (Some(l), Some(r)) => l.checked_lcm(r).is_none(),
        _ => false,
    }
}

/// Parse a JSON number into a bound fraction; `None` when it can't be represented (e.g. exponent
/// notation past the digit cap), so the caller keeps a strict `AllOf` rather than guessing.
pub(super) fn scalar_from_json(number: &Number) -> Option<BoundFraction> {
    if let Some(value) = number.as_i64() {
        return Some(BoundFraction::from(value));
    }
    if let Some(value) = number.as_u64() {
        return Some(BoundFraction::from(value));
    }
    // `as_str` exposes the raw decimal without allocating.
    #[cfg(feature = "arbitrary-precision")]
    {
        BoundFraction::from_str(number.as_str()).ok()
    }
    // No `as_str` without arbitrary precision; `to_string` preserves the exact decimal that `as_f64` would round.
    #[cfg(not(feature = "arbitrary-precision"))]
    {
        BoundFraction::from_str(&number.to_string()).ok()
    }
}
