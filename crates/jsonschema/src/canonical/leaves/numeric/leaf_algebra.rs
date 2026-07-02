//! Shared algebra over numeric leaves (`integer`, `number`).

use std::cmp::Ordering;

use crate::{
    canonical::{
        intern::shared,
        intersect::{normalize_integer_not_multiple_of, normalize_number_not_multiple_of},
        ir::{
            BoundFraction, BoundInteger, Bounds, IntegerBounds, IntegerLeaf, NumberBounds,
            NumberLeaf, Schema, SharedSchema,
        },
    },
    JsonType,
};

/// Bounds of a numeric leaf, abstracted over the half-lines its negation needs and the containment its coverage check needs.
/// Endpoint comparison is type-specific (integers step exclusive markers; fractions tie-break on the flag), so `covers` stays per-type.
pub(crate) trait NumericBounds: Clone {
    /// Half-line strictly below the lower bound, or `None` when the leaf is unbounded below.
    fn below(&self) -> Option<Self>;
    /// Half-line strictly above the upper bound, or `None` when the leaf is unbounded above.
    fn above(&self) -> Option<Self>;
    /// Whether `self`'s interval contains every value `other`'s does (`self` is at least as wide on both ends).
    fn covers(&self, other: &Self) -> bool;
}

/// A numeric leaf whose negation follows one shape: the out-of-bounds half-lines, the `multipleOf` modulus flipped to its
/// `not_multiple_of` dual, and each excluded modulus flipped back to a `multipleOf` branch - all within the original bounds.
pub(crate) trait NumericLeaf: Sized {
    type Modulus: Clone;
    /// The bound carrier (`BoundInteger` or `BoundFraction`).
    type Scalar: Ord + Clone;
    const JSON_TYPE: JsonType;

    fn bounds(&self) -> &Bounds<Self::Scalar>;
    fn multiple_of(&self) -> Option<&Self::Modulus>;
    fn not_multiple_of(&self) -> &[Self::Modulus];
    /// Build the leaf's schema node from its three facets.
    fn into_schema(
        bounds: Bounds<Self::Scalar>,
        multiple_of: Option<Self::Modulus>,
        not_multiple_of: Vec<Self::Modulus>,
    ) -> Schema;
    /// Reassemble a leaf of this kind from the three facets an intersection produces.
    fn from_parts(
        bounds: Bounds<Self::Scalar>,
        multiple_of: Option<Self::Modulus>,
        not_multiple_of: Vec<Self::Modulus>,
    ) -> Self;
    /// Combine two `multipleOf` moduli (their LCM); `None` when unrepresentable, so the caller keeps both in an `AllOf`.
    fn combine_multiple_of(left: &Self::Modulus, right: &Self::Modulus) -> Option<Self::Modulus>;
    /// Sort/dedup the `not_multiple_of` list and drop redundant entries; `None` on a contradiction (leaf then empty).
    fn normalize(self) -> Option<Self>;
    /// Whether modulus `big` covers `small`: every `multipleOf: small` value is also a `big` multiple, i.e. `small` is a
    /// `big` multiple. Absent `big` covers anything; absent `small` is covered only by an absent `big`.
    fn modulus_covers(big: Option<&Self::Modulus>, small: Option<&Self::Modulus>) -> bool;
    /// Whether `value` is a multiple of `modulus` (`modulus` is known non-zero).
    fn value_is_multiple(value: &Self::Scalar, modulus: &Self::Modulus) -> bool;
    fn modulus_is_zero(modulus: &Self::Modulus) -> bool;
    fn scalar_is_zero(value: &Self::Scalar) -> bool;
}

/// Whether integer/number leaf `big` covers `small`: wider bounds, `big`'s `multipleOf` divides `small`'s, and `big`'s
/// excluded multiples subset `small`'s. Equal leaves return `false` so strict-containment callers keep one representative.
pub(crate) fn numeric_leaf_covers<L>(big: &L, small: &L) -> bool
where
    L: NumericLeaf + PartialEq,
    L::Modulus: PartialEq,
    Bounds<L::Scalar>: NumericBounds,
{
    if big == small {
        return false;
    }
    big.bounds().covers(small.bounds())
        && L::modulus_covers(big.multiple_of(), small.multiple_of())
        // Every value `big` excludes must also be excluded by `small`, else `big` is more restrictive and can't cover
        // `small`. A finer modulus in `small` excludes every multiple of a coarser one in `big`.
        && big.not_multiple_of().iter().all(|excluded| {
            small
                .not_multiple_of()
                .iter()
                .any(|finer| L::modulus_covers(Some(finer), Some(excluded)))
        })
}

/// In-kind branches of a numeric leaf's complement: the out-of-bounds half-lines plus the `multipleOf`/`not_multiple_of`
/// duals. Out-of-kind branches (every other JSON type) are added by the caller.
pub(crate) fn negate_in_kind<L: NumericLeaf>(leaf: &L) -> Vec<SharedSchema>
where
    Bounds<L::Scalar>: NumericBounds,
{
    let bounds = leaf.bounds();
    let mut in_kind: Vec<SharedSchema> = Vec::new();
    if let Some(below) = bounds.below() {
        in_kind.push(shared(L::into_schema(below, None, Vec::new())));
    }
    if let Some(above) = bounds.above() {
        in_kind.push(shared(L::into_schema(above, None, Vec::new())));
    }
    // The value is *not* a multiple of the original `multipleOf`, within the original bounds.
    if let Some(modulus) = leaf.multiple_of() {
        in_kind.push(shared(L::into_schema(
            bounds.clone(),
            None,
            vec![modulus.clone()],
        )));
    }
    // The value *is* a multiple of some excluded modulus, within the original bounds.
    for modulus in leaf.not_multiple_of() {
        in_kind.push(shared(L::into_schema(
            bounds.clone(),
            Some(modulus.clone()),
            Vec::new(),
        )));
    }
    in_kind
}

impl NumericBounds for IntegerBounds {
    fn below(&self) -> Option<Self> {
        let minimum = self.minimum.as_ref()?;
        let maximum = if self.exclusive_minimum {
            (minimum).owned()
        } else {
            // `minimum - 1`; `None` when pinned to `i64::MIN` (no representable half-line below; empty in the i64 model).
            minimum.checked_decrement()?
        };
        Some(IntegerBounds {
            minimum: None,
            maximum: Some(maximum),
            exclusive_minimum: false,
            exclusive_maximum: false,
        })
    }

    fn above(&self) -> Option<Self> {
        let maximum = self.maximum.as_ref()?;
        let minimum = if self.exclusive_maximum {
            (maximum).owned()
        } else {
            // `maximum + 1`; `None` when pinned to `i64::MAX` (no representable half-line above; empty in the i64 model).
            maximum.checked_increment()?
        };
        Some(IntegerBounds {
            minimum: Some(minimum),
            maximum: None,
            exclusive_minimum: false,
            exclusive_maximum: false,
        })
    }

    /// Exclusive markers shift each endpoint by one before comparing.
    fn covers(&self, other: &Self) -> bool {
        let lower_ok = match (&self.minimum, &other.minimum) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(big_minimum), Some(small_minimum)) => {
                let big_effective = if self.exclusive_minimum {
                    big_minimum + BoundInteger::one()
                } else {
                    (big_minimum).owned()
                };
                let small_effective = if other.exclusive_minimum {
                    small_minimum + BoundInteger::one()
                } else {
                    (small_minimum).owned()
                };
                big_effective <= small_effective
            }
        };
        if !lower_ok {
            return false;
        }
        match (&self.maximum, &other.maximum) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(big_maximum), Some(small_maximum)) => {
                let big_effective = if self.exclusive_maximum {
                    big_maximum - BoundInteger::one()
                } else {
                    (big_maximum).owned()
                };
                let small_effective = if other.exclusive_maximum {
                    small_maximum - BoundInteger::one()
                } else {
                    (small_maximum).owned()
                };
                big_effective >= small_effective
            }
        }
    }
}

impl NumericBounds for NumberBounds {
    fn below(&self) -> Option<Self> {
        let minimum = self.minimum.as_ref()?;
        Some(NumberBounds {
            minimum: None,
            maximum: Some((minimum).owned()),
            exclusive_minimum: false,
            exclusive_maximum: !self.exclusive_minimum,
        })
    }

    fn above(&self) -> Option<Self> {
        let maximum = self.maximum.as_ref()?;
        Some(NumberBounds {
            minimum: Some((maximum).owned()),
            maximum: None,
            exclusive_minimum: !self.exclusive_maximum,
            exclusive_maximum: false,
        })
    }

    /// Fractional bounds can't be shifted by a fixed delta; compare values plus exclusive flag.
    fn covers(&self, other: &Self) -> bool {
        let lower_ok = match (&self.minimum, &other.minimum) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(big_minimum), Some(small_minimum)) => {
                match big_minimum.cmp(small_minimum) {
                    Ordering::Less => true,
                    Ordering::Greater => false,
                    // Equal endpoints: big inclusive covers big exclusive.
                    Ordering::Equal => !self.exclusive_minimum || other.exclusive_minimum,
                }
            }
        };
        if !lower_ok {
            return false;
        }
        match (&self.maximum, &other.maximum) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(big_maximum), Some(small_maximum)) => match big_maximum.cmp(small_maximum) {
                Ordering::Greater => true,
                Ordering::Less => false,
                Ordering::Equal => !self.exclusive_maximum || other.exclusive_maximum,
            },
        }
    }
}

impl NumericLeaf for IntegerLeaf {
    type Modulus = BoundInteger;
    type Scalar = BoundInteger;
    const JSON_TYPE: JsonType = JsonType::Integer;

    fn bounds(&self) -> &IntegerBounds {
        &self.bounds
    }
    fn multiple_of(&self) -> Option<&BoundInteger> {
        self.multiple_of.as_ref()
    }
    fn not_multiple_of(&self) -> &[BoundInteger] {
        &self.not_multiple_of
    }
    fn into_schema(
        bounds: IntegerBounds,
        multiple_of: Option<BoundInteger>,
        not_multiple_of: Vec<BoundInteger>,
    ) -> Schema {
        Schema::Integer(IntegerLeaf {
            bounds,
            multiple_of,
            not_multiple_of,
        })
    }
    fn from_parts(
        bounds: IntegerBounds,
        multiple_of: Option<BoundInteger>,
        not_multiple_of: Vec<BoundInteger>,
    ) -> Self {
        IntegerLeaf {
            bounds,
            multiple_of,
            not_multiple_of,
        }
    }
    fn combine_multiple_of(left: &BoundInteger, right: &BoundInteger) -> Option<BoundInteger> {
        left.checked_lcm(right)
    }
    fn normalize(self) -> Option<Self> {
        normalize_integer_not_multiple_of(self)
    }

    /// `small % big == 0`, so every value `small` admits is also a `big` multiple. Zero modulus (spec-disallowed)
    /// declines.
    fn modulus_covers(big: Option<&BoundInteger>, small: Option<&BoundInteger>) -> bool {
        match (big, small) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(big_modulus), Some(small_modulus)) => {
                if big_modulus.is_zero() || small_modulus.is_zero() {
                    return false;
                }
                (small_modulus % big_modulus).is_zero()
            }
        }
    }

    fn value_is_multiple(value: &BoundInteger, modulus: &BoundInteger) -> bool {
        modulus.divides(value)
    }
    fn modulus_is_zero(modulus: &BoundInteger) -> bool {
        modulus.is_zero()
    }
    fn scalar_is_zero(value: &BoundInteger) -> bool {
        value.is_zero()
    }
}

impl NumericLeaf for NumberLeaf {
    type Modulus = BoundFraction;
    type Scalar = BoundFraction;
    const JSON_TYPE: JsonType = JsonType::Number;

    fn bounds(&self) -> &NumberBounds {
        &self.bounds
    }
    fn multiple_of(&self) -> Option<&BoundFraction> {
        self.multiple_of.as_ref()
    }
    fn not_multiple_of(&self) -> &[BoundFraction] {
        &self.not_multiple_of
    }
    fn into_schema(
        bounds: NumberBounds,
        multiple_of: Option<BoundFraction>,
        not_multiple_of: Vec<BoundFraction>,
    ) -> Schema {
        Schema::Number(NumberLeaf {
            bounds,
            multiple_of,
            not_multiple_of,
        })
    }
    fn from_parts(
        bounds: NumberBounds,
        multiple_of: Option<BoundFraction>,
        not_multiple_of: Vec<BoundFraction>,
    ) -> Self {
        NumberLeaf {
            bounds,
            multiple_of,
            not_multiple_of,
        }
    }
    fn combine_multiple_of(left: &BoundFraction, right: &BoundFraction) -> Option<BoundFraction> {
        left.checked_lcm(right)
    }
    fn normalize(self) -> Option<Self> {
        normalize_number_not_multiple_of(self)
    }

    /// `small / big` is a whole number, so every `small` multiple is also a `big` multiple. Zero modulus declines.
    fn modulus_covers(big: Option<&BoundFraction>, small: Option<&BoundFraction>) -> bool {
        match (big, small) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(big_modulus), Some(small_modulus)) => {
                if big_modulus.is_zero() || small_modulus.is_zero() {
                    return false;
                }
                (small_modulus / big_modulus).denominator_is_one()
            }
        }
    }

    fn value_is_multiple(value: &BoundFraction, modulus: &BoundFraction) -> bool {
        modulus.divides(value)
    }
    fn modulus_is_zero(modulus: &BoundFraction) -> bool {
        modulus.is_zero()
    }
    fn scalar_is_zero(value: &BoundFraction) -> bool {
        value.is_zero()
    }
}
