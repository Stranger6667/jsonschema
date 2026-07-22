//! A real-interval endpoint over the `number` domain.
use jsonschema_value::numeric_check::{check_bound, compile_bound, BoundOp};
use serde_json::Number;

use std::cmp::Ordering;

use super::normalized_number;

/// One end of a number interval: a limit and whether the limit itself is admitted. Membership goes
/// through the runtime's own numeric checks, so the canonical form and the validator agree on every
/// value by construction. The limit is stored in canonical spelling, so equality is value equality.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct BoundNumber {
    limit: Number,
    inclusive: bool,
}

impl BoundNumber {
    /// The limit is stored in the one spelling its value has, so two bounds are equal exactly when
    /// they admit the same values.
    pub(crate) fn new(limit: &Number, inclusive: bool) -> Self {
        Self {
            limit: normalized_number(limit),
            inclusive,
        }
    }

    /// The same limit with the limit itself no longer admitted.
    pub(crate) fn excluded(self) -> Self {
        Self {
            inclusive: false,
            ..self
        }
    }

    pub(crate) fn to_number(&self) -> Number {
        self.limit.clone()
    }

    pub(crate) fn is_inclusive(&self) -> bool {
        self.inclusive
    }

    /// Whether `value` lies on the admitted side of this bound.
    pub(crate) fn admits(&self, value: &Number, side: Side) -> bool {
        check_bound(&compile_bound(self.op(side), &self.limit), value)
    }

    /// Whether `other` admits every value this bound does, i.e. this one is at least as tight.
    pub(crate) fn is_tighter_than(&self, other: &Self, side: Side) -> bool {
        if self.limit == other.limit {
            // An excluded limit is tighter than the same limit included.
            return self.inclusive <= other.inclusive;
        }
        // With different limits, the tighter bound is the one the other still admits.
        other.admits(&self.limit, side) && !self.admits(&other.limit, side)
    }

    fn op(&self, side: Side) -> BoundOp {
        match (side, self.inclusive) {
            (Side::Lower, true) => BoundOp::Gte,
            (Side::Lower, false) => BoundOp::Gt,
            (Side::Upper, true) => BoundOp::Lte,
            (Side::Upper, false) => BoundOp::Lt,
        }
    }
}

impl PartialOrd for BoundNumber {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BoundNumber {
    fn cmp(&self, other: &Self) -> Ordering {
        // Limits are stored canonically, so matching text is matching value and needs no check.
        if self.limit == other.limit {
            return self.inclusive.cmp(&other.inclusive);
        }
        let less = check_bound(&compile_bound(BoundOp::Lt, &other.limit), &self.limit);
        let greater = check_bound(&compile_bound(BoundOp::Gt, &other.limit), &self.limit);
        match (less, greater) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            // Equal limits, or a pair the runtime declines to order. Falling back to the canonical
            // text keeps the order total either way, which sorting relies on.
            _ => self
                .limit
                .to_string()
                .cmp(&other.limit.to_string())
                .then(self.inclusive.cmp(&other.inclusive)),
        }
    }
}

/// Which end of an interval a bound sits at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Side {
    Lower,
    Upper,
}

#[cfg(test)]
mod tests {
    use super::*;

    // A total order needs exactly one of a < b, a > b, a == b for every pair.
    #[test]
    fn ordering_is_antisymmetric() {
        let bounds: Vec<BoundNumber> = ["2", "2.5", "-3", "0", "1e100"]
            .iter()
            .flat_map(|text| {
                let number: Number = text.parse().expect("number");
                [
                    BoundNumber::new(&number, true),
                    BoundNumber::new(&number, false),
                ]
            })
            .collect();
        for left in &bounds {
            for right in &bounds {
                assert_eq!(
                    left.cmp(right),
                    right.cmp(left).reverse(),
                    "{left:?} vs {right:?}"
                );
            }
        }
    }
}
