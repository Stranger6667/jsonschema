#[cfg(feature = "arbitrary-precision")]
use std::str::FromStr;

use serde_json::Number;

use crate::canonical::{
    context::CanonicalizationContext,
    ir::{BoundInteger, IntegerBounds, IntegerLeaf, Schema},
    leaves::{Intersection, Leaf, TypedLeaf, Verdict},
    numeric::numeric_leaf_covers,
    prover::Prover,
};

use super::intersect_numeric_leaf;

impl Leaf for IntegerLeaf {
    /// `Residual` signals an unrepresentable combined `multipleOf` (LCM overflow); the caller keeps
    /// both leaves in an `AllOf` so the validator enforces each constraint directly.
    ///
    /// ```text
    /// BEFORE: {"type": "integer", "minimum": 5}  and  {"type": "integer", "maximum": 10}
    /// AFTER:  {"type": "integer", "minimum": 5, "maximum": 10}
    ///
    /// BEFORE: {"type": "integer", "multipleOf": 4}  and  {"type": "integer", "multipleOf": 6}
    /// AFTER:  {"type": "integer", "multipleOf": 12}
    ///
    /// BEFORE: {"type": "integer", "multipleOf": 2}  and  {"not": {"type": "integer", "multipleOf": 2}}
    /// AFTER:  false
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
        integer_bounds_empty(&self.bounds) || integer_multiple_of_misses_interval(self)
    }
}

impl TypedLeaf for IntegerLeaf {
    fn wrap(self) -> Schema {
        Schema::Integer(self)
    }
    fn project(schema: &Schema) -> Option<&Self> {
        match schema {
            Schema::Integer(leaf) => Some(leaf),
            _ => None,
        }
    }
}

fn integer_bounds_empty(bounds: &IntegerBounds) -> bool {
    let (Some(minimum), Some(maximum)) = (bounds.effective_minimum(), bounds.effective_maximum())
    else {
        return false;
    };
    minimum > maximum
}

fn integer_multiple_of_misses_interval(leaf: &IntegerLeaf) -> bool {
    let Some(modulus) = leaf.multiple_of.as_ref() else {
        return false;
    };
    if modulus.is_zero() {
        return false;
    }
    let (Some(effective_minimum), Some(effective_maximum)) = (
        leaf.bounds.effective_minimum(),
        leaf.bounds.effective_maximum(),
    ) else {
        return false;
    };
    if effective_minimum > effective_maximum {
        return true;
    }
    // An unrepresentable next multiple lies past `i64::MAX`, hence past any representable maximum.
    match effective_minimum.checked_next_multiple_of(&modulus.abs()) {
        Some(first_multiple) => first_multiple > effective_maximum,
        None => true,
    }
}

/// Parse a JSON number into a bound integer.
///
/// `None` when outside the representable range (a `u64` above `i64::MAX` in the default build), so the caller defers
/// to the validator rather than clamping to a wrong verdict.
pub(super) fn scalar_from_json(number: &Number) -> Option<BoundInteger> {
    if let Some(value) = number.as_i64() {
        return Some(BoundInteger::from(value));
    }
    #[cfg(feature = "arbitrary-precision")]
    {
        if let Some(value) = number.as_u64() {
            return Some(BoundInteger::from(value));
        }
        // `as_str` exposes the raw digits without allocating.
        BoundInteger::from_str(number.as_str()).ok()
    }
    // `as_i64` handled the representable range. A remaining integer is a `u64` above `i64::MAX`, which the `i64` bound
    // type cannot hold; return `None` so callers treat it as unverifiable instead of clamping to a wrong value.
    #[cfg(not(feature = "arbitrary-precision"))]
    {
        None
    }
}
