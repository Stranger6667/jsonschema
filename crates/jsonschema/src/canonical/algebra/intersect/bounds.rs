use std::{cmp::Ordering, sync::Arc};

use crate::canonical::{
    context::CanonicalizationContext,
    ir::{Bounds, SharedSchema},
    leaves::Intersection,
    numeric::NumericLeaf,
};

use super::intersect_internal;

/// Intersect two numeric leaves of the same kind: tighten bounds, LCM moduli, union `not_multiple_of`,
/// normalize. `Residual` if the combined modulus is unrepresentable; `Empty` if disjoint/contradictory.
pub(super) fn intersect_numeric_leaf<L: NumericLeaf>(left: &L, right: &L) -> Intersection<L> {
    let (lower, upper) = (left.bounds(), right.bounds());
    let (minimum, exclusive_minimum) = combine_bound::<L::Scalar>(
        Tighter::Larger,
        lower.minimum.as_ref(),
        lower.exclusive_minimum,
        upper.minimum.as_ref(),
        upper.exclusive_minimum,
    );
    let (maximum, exclusive_maximum) = combine_bound::<L::Scalar>(
        Tighter::Smaller,
        lower.maximum.as_ref(),
        lower.exclusive_maximum,
        upper.maximum.as_ref(),
        upper.exclusive_maximum,
    );
    if is_empty_interval(
        minimum.as_ref(),
        exclusive_minimum,
        maximum.as_ref(),
        exclusive_maximum,
    ) {
        return Intersection::Empty;
    }
    let multiple_of = match (left.multiple_of(), right.multiple_of()) {
        (Some(l), Some(r)) => match L::combine_multiple_of(l, r) {
            Some(value) => Some(value),
            None => return Intersection::Residual,
        },
        (Some(value), None) | (None, Some(value)) => Some(value.clone()),
        (None, None) => None,
    };
    let mut not_multiple_of = left.not_multiple_of().to_vec();
    not_multiple_of.extend_from_slice(right.not_multiple_of());
    let leaf = L::from_parts(
        Bounds {
            minimum,
            maximum,
            exclusive_minimum,
            exclusive_maximum,
        },
        multiple_of,
        not_multiple_of,
    );
    match leaf.normalize() {
        Some(leaf) => Intersection::Merged(leaf),
        None => Intersection::Empty,
    }
}

/// Which direction is "tighter" - larger for lower bounds, smaller for upper bounds.
#[derive(Copy, Clone)]
pub(super) enum Tighter {
    Larger,
    Smaller,
}

/// `true` when `[minimum, maximum]` is empty under the given exclusivity.
pub(super) fn is_empty_interval<T: Ord>(
    minimum: Option<&T>,
    exclusive_minimum: bool,
    maximum: Option<&T>,
    exclusive_maximum: bool,
) -> bool {
    let (Some(minimum), Some(maximum)) = (minimum, maximum) else {
        return false;
    };
    match minimum.cmp(maximum) {
        Ordering::Greater => true,
        Ordering::Equal => exclusive_minimum || exclusive_maximum,
        Ordering::Less => false,
    }
}

/// Intersect two optional sub-schemas; a missing side imposes no constraint.
pub(super) fn intersect_optional(
    left: Option<&SharedSchema>,
    right: Option<&SharedSchema>,
    ctx: &CanonicalizationContext,
) -> Option<SharedSchema> {
    match (left, right) {
        (Some(lhs), Some(rhs)) => Some(intersect_internal(lhs, rhs, ctx)),
        (Some(value), None) | (None, Some(value)) => Some(Arc::clone(value)),
        (None, None) => None,
    }
}

/// Keep the larger of two optional bounds (loosest floor wins when one is absent).
pub(super) fn max_option<T: Ord + Clone>(left: Option<&T>, right: Option<&T>) -> Option<T> {
    match (left, right) {
        (Some(lhs), Some(rhs)) => Some(lhs.max(rhs).clone()),
        (Some(value), None) | (None, Some(value)) => Some(value.clone()),
        (None, None) => None,
    }
}

/// Keep the smaller of two optional bounds (tightest cap wins when one is absent).
pub(super) fn min_option<T: Ord + Clone>(left: Option<&T>, right: Option<&T>) -> Option<T> {
    match (left, right) {
        (Some(lhs), Some(rhs)) => Some(lhs.min(rhs).clone()),
        (Some(value), None) | (None, Some(value)) => Some(value.clone()),
        (None, None) => None,
    }
}

/// Pick the tighter bound; on a tie, OR the exclusivity flags.
pub(super) fn combine_bound<T: Ord + Clone>(
    tighter: Tighter,
    left_value: Option<&T>,
    left_exclusive: bool,
    right_value: Option<&T>,
    right_exclusive: bool,
) -> (Option<T>, bool) {
    let pick_left = match tighter {
        Tighter::Larger => Ordering::Greater,
        Tighter::Smaller => Ordering::Less,
    };
    match (left_value, right_value) {
        (Some(left), Some(right)) => match left.cmp(right) {
            ord if ord == pick_left => (Some(left.clone()), left_exclusive),
            Ordering::Equal => (Some(left.clone()), left_exclusive || right_exclusive),
            _ => (Some(right.clone()), right_exclusive),
        },
        (Some(value), None) => (Some(value.clone()), left_exclusive),
        (None, Some(value)) => (Some(value.clone()), right_exclusive),
        (None, None) => (None, false),
    }
}
