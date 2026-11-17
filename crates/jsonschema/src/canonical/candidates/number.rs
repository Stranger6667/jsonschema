//! Candidates for an integer or a number leaf.
use serde_json::{Number, Value};

use crate::canonical::ir::{BoundInteger, BoundNumber, IntegerLeaf, NumberLeaf};

/// Instances worth trying against an integer leaf.
pub(super) fn integer(leaf: &IntegerLeaf) -> Vec<Value> {
    // An integer bound is stored as the first integer it admits.
    let bounds = &leaf.bounds;
    let end = |bound: &BoundInteger| WindowEnd {
        limit: bound.to_number(),
        admitted: true,
    };
    whole_number_candidates(
        bounds.minimum.as_ref().map(end),
        bounds.maximum.as_ref().map(end),
    )
}

/// Instances worth trying against a number leaf.
pub(super) fn number(leaf: &NumberLeaf) -> Vec<Value> {
    let end = |bound: &BoundNumber| WindowEnd {
        limit: bound.to_number(),
        admitted: bound.is_inclusive(),
    };
    whole_number_candidates(
        leaf.minimum.as_ref().map(end),
        leaf.maximum.as_ref().map(end),
    )
}

/// Zero, which every divisor takes, and the ends of the window.
fn whole_number_candidates(minimum: Option<WindowEnd>, maximum: Option<WindowEnd>) -> Vec<Value> {
    // A point between the two ends, or one step in from the only end there is.
    let interior = interior_point(
        minimum.as_ref().map(|end| &end.limit),
        maximum.as_ref().map(|end| &end.limit),
    );
    // Admitted ends first: a caller taking one candidate needs one the window holds, and an
    // excluded end is no value of it.
    let mut candidates: Vec<Value> = [minimum, maximum]
        .into_iter()
        .flatten()
        .filter(|end| end.admitted)
        .map(|end| Value::Number(end.limit))
        .collect();
    candidates.extend(interior);
    // Zero, which every divisor takes.
    candidates.push(Value::Number(0.into()));
    candidates
}

/// One end of a numeric window, and whether the window admits it.
struct WindowEnd {
    limit: Number,
    admitted: bool,
}

/// A number strictly inside the window, where one can be written. A window narrower than the gap
/// between two neighbouring floats has none.
fn interior_point(minimum: Option<&Number>, maximum: Option<&Number>) -> Option<Value> {
    let point = match (
        minimum.and_then(Number::as_f64),
        maximum.and_then(Number::as_f64),
    ) {
        (Some(low), Some(high)) => low + (high - low) / 2.0,
        (Some(low), None) => low + 1.0,
        (None, Some(high)) => high - 1.0,
        (None, None) => return None,
    };
    Number::from_f64(point).map(Value::Number)
}
