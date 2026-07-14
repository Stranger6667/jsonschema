use std::cmp::Ordering;

use serde_json::Number;

use crate::canonical::{
    context::CanonicalizationContext,
    ir::{BoundFraction, BoundInteger, NumberLeaf, Schema},
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
    // Overflow means the hit cannot be proven; not shrinking the window keeps the verdict conservative.
    modulus
        .checked_mul(&BoundFraction::from(index))
        .is_some_and(|product| product.cmp(value) == Ordering::Equal)
}

/// Parse a JSON number into a bound fraction; `None` when it can't be represented, so the caller
/// keeps a strict `AllOf` rather than guessing. Delegates to the parse-side conversion: bounds and
/// values must map to the same rational or boundary membership flips.
pub(super) fn scalar_from_json(number: &Number) -> Option<BoundFraction> {
    crate::canonical::parse::bigfraction_from_number(number)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::canonical::tests_util::canonicalize;

    // u64-magnitude bounds with a fractional modulus must not overflow the divisibility math.
    #[test]
    fn extreme_bound_with_fractional_modulus_does_not_overflow() {
        let schema = json!({
            "type": "number",
            "minimum": 9_223_372_036_854_775_807_i64,
            "maximum": 9_223_372_036_854_775_807_i64,
            "multipleOf": 0.1
        });
        assert!(canonicalize(&schema).is_satisfiable());
    }

    // Bounds and const/enum values must parse to the same rational, or boundary membership flips.
    #[test]
    fn boundary_const_survives_bound_intersection() {
        let schema = json!({"maximum": 2_251_799_813_685_248.5, "const": 2_251_799_813_685_248.5});
        let canonical = canonicalize(&schema);
        assert!(canonical.is_satisfiable());
        let validator =
            crate::validator_for(&canonical.to_json_schema()).expect("canonical compiles");
        assert!(validator.is_valid(&json!(2_251_799_813_685_248.5)));
    }
}
