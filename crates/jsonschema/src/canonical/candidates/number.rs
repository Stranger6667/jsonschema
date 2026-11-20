//! Candidates for an integer or a number leaf.
use std::ops::ControlFlow;

use serde_json::{Number, Value};

use super::Visit;
use crate::canonical::ir::{
    BoundInteger, BoundNumber, BoundRational, Discrete, IntegerLeaf, NumberLeaf, Round, Side,
};

/// How many steps up from the minimum are tried.
const DIVISOR_MULTIPLES: u32 = 3;

/// Instances worth trying against an integer leaf.
pub(super) fn integer(leaf: &IntegerLeaf, visit: &mut Visit<'_>) -> ControlFlow<()> {
    // An integer bound is stored as the first integer it includes.
    let bounds = &leaf.bounds;
    let end = |bound: &BoundInteger| WindowEnd {
        limit: bound.to_number(),
        accepted: true,
    };
    let candidates = whole_number_candidates(
        bounds.minimum.as_ref().map(end),
        bounds.maximum.as_ref().map(end),
    );
    // Canonicalization reads the first candidate, which must not depend on the divisors.
    candidates.into_iter().try_for_each(&mut *visit)?;
    on_integer_grid(leaf)
        .into_iter()
        .filter(|value| {
            BoundInteger::from_number(value).is_some_and(|whole| bounds.contains(&whole))
        })
        .map(Value::Number)
        .try_for_each(visit)
}

/// Instances worth trying against a number leaf.
pub(super) fn number(leaf: &NumberLeaf, visit: &mut Visit<'_>) -> ControlFlow<()> {
    let end = |bound: &BoundNumber| WindowEnd {
        limit: bound.to_number(),
        accepted: bound.is_inclusive(),
    };
    let candidates = whole_number_candidates(
        leaf.minimum.as_ref().map(end),
        leaf.maximum.as_ref().map(end),
    );
    // Canonicalization reads the first candidate, which must not depend on the divisors.
    candidates.into_iter().try_for_each(&mut *visit)?;
    off_number_grid(leaf)
        .into_iter()
        .filter(|value| {
            leaf.minimum
                .as_ref()
                .is_none_or(|bound| bound.accepts(value, Side::Lower))
                && leaf
                    .maximum
                    .as_ref()
                    .is_none_or(|bound| bound.accepts(value, Side::Upper))
        })
        .map(Value::Number)
        .try_for_each(visit)
}

/// Multiples of each demanded divisor. With no divisor demanded, the integers just above the
/// minimum, or above zero without one, dodge a barred divisor.
fn on_integer_grid(leaf: &IntegerLeaf) -> Vec<Number> {
    let mut found = Vec::new();
    let minimum = leaf
        .bounds
        .minimum
        .as_ref()
        .map(|minimum| BoundNumber::new(&minimum.to_number(), true));
    for step in leaf.multiple_of.as_slice() {
        // A fractional divisor is no integer itself, so without a minimum its grid is stepped from it.
        let from = minimum.clone().or_else(|| {
            (!step.accepts_only_whole()).then(|| BoundNumber::new(&step.to_number(), false))
        });
        found.extend(multiples(step, from));
    }
    if leaf.multiple_of.is_empty() && !leaf.not_multiple_of.is_empty() {
        let mut current = leaf
            .bounds
            .minimum
            .clone()
            .or_else(|| BoundInteger::from_number(&Number::from(0)));
        for _ in 0..DIVISOR_MULTIPLES {
            current = current.and_then(Discrete::checked_increment);
            found.extend(current.as_ref().map(BoundInteger::to_number));
        }
    }
    found
}

/// Values off the integers and off each barred divisor's grid, and multiples of each demanded one.
fn off_number_grid(leaf: &NumberLeaf) -> Vec<Number> {
    let mut found = Vec::new();
    // Draft 4 bars its integers with no divisor to halve, so a value halfway between two stands in.
    if leaf.excludes_integers {
        found.extend(Number::from_f64(0.5));
        found.extend(
            leaf.minimum
                .as_ref()
                .and_then(|bound| Number::from_f64(bound.to_number().as_f64()? + 0.5)),
        );
    }
    // Half a divisor is never one of its multiples.
    found.extend(
        leaf.not_multiple_of
            .as_slice()
            .iter()
            .filter_map(|step| Number::from_f64(step.to_number().as_f64()? / 2.0)),
    );
    for step in leaf.multiple_of.as_slice() {
        found.extend(multiples(step, leaf.minimum.clone()));
    }
    found
}

/// The divisor itself, and the first multiples within the bound `from`.
fn multiples(step: &BoundRational, from: Option<BoundNumber>) -> Vec<Number> {
    let mut found = vec![step.to_number()];
    let mut last = from;
    for _ in 0..DIVISOR_MULTIPLES {
        let Some(next) = last.and_then(|bound| step.multiple_beyond(&bound, Round::Up)) else {
            break;
        };
        found.push(next.to_number());
        last = Some(next.excluded());
    }
    found
}

/// The window's included ends, a point inside it, then zero, which every divisor takes.
fn whole_number_candidates(minimum: Option<WindowEnd>, maximum: Option<WindowEnd>) -> Vec<Value> {
    // A point between the two ends, or one step in from the only end there is.
    let interior = interior_point(
        minimum.as_ref().map(|end| &end.limit),
        maximum.as_ref().map(|end| &end.limit),
    );
    // Included ends first: a caller taking one candidate needs one the window holds, and an
    // excluded end is no value of it.
    let mut candidates: Vec<Value> = [minimum, maximum]
        .into_iter()
        .flatten()
        .filter(|end| end.accepted)
        .map(|end| Value::Number(end.limit))
        .collect();
    candidates.extend(interior);
    candidates.push(Value::Number(0.into()));
    candidates
}

/// One end of a numeric window, and whether the window includes it.
struct WindowEnd {
    limit: Number,
    accepted: bool,
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
