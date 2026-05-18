//! `anyOf` interval merging: contiguous cardinality windows (string length, array/object counts) fuse into one leaf.

#![cfg_attr(not(feature = "arbitrary-precision"), allow(clippy::clone_on_copy))]

use std::sync::Arc;

use ahash::AHashSet;

use crate::canonical::{
    intern::shared,
    ir::{BoundCardinality, LengthBounds, ObjectLeaf, ObjectRequirement, Schema, SharedSchema},
};
/// Length windows over otherwise-identical string branches merge when contiguous:
/// `{0..0 chars} ∪ {1..1 chars}` has the single-leaf spelling negation recomposition produces.
///
/// ```text
/// BEFORE: {"anyOf": [{"type": "string", "maxLength": 0}, {"type": "string", "minLength": 1, "maxLength": 1}]}
/// AFTER:  {"type": "string", "maxLength": 1}
/// ```
pub(super) fn merge_string_length_windows(branches: &mut Vec<SharedSchema>) -> bool {
    merge_intervals(
        branches,
        |schema| match schema {
            Schema::String(leaf) => {
                // Blank the length facets: the remainder is the merge key (patterns, format, ...).
                let mut key = leaf.clone();
                key.min_length = None;
                key.max_length = None;
                Some(CardinalityInterval {
                    key,
                    minimum: leaf
                        .min_length
                        .clone()
                        .unwrap_or_else(|| BoundCardinality::from(0_u8)),
                    maximum: leaf.max_length.clone(),
                })
            }
            _ => None,
        },
        |interval| {
            let mut leaf = interval.key;
            leaf.min_length = (!interval.minimum.is_zero()).then_some(interval.minimum);
            leaf.max_length = interval.maximum;
            Schema::String(leaf)
        },
    )
}

/// Length windows over otherwise-identical array branches merge when contiguous:
/// `{0..1 items} ∪ {1..2 items}` has the single-leaf spelling negation recomposition produces.
///
/// ```text
/// BEFORE: {"anyOf": [{"type": "array", "maxItems": 1}, {"type": "array", "minItems": 1, "maxItems": 2}]}
/// AFTER:  {"type": "array", "maxItems": 2}
/// ```
pub(super) fn merge_array_count_windows(branches: &mut Vec<SharedSchema>) -> bool {
    merge_intervals(
        branches,
        |schema| match schema {
            Schema::Array(leaf) => {
                // Blank the length: the remainder is the merge key (prefix, tail, uniqueItems, ...).
                let mut key = leaf.clone();
                key.length = LengthBounds {
                    minimum: BoundCardinality::from(0u64),
                    maximum: None,
                };
                Some(CardinalityInterval {
                    key,
                    minimum: leaf.length.minimum.clone(),
                    maximum: leaf.length.maximum.clone(),
                })
            }
            _ => None,
        },
        |interval| {
            let mut leaf = interval.key;
            leaf.length = LengthBounds {
                minimum: interval.minimum,
                maximum: interval.maximum,
            };
            Schema::Array(leaf)
        },
    )
}

/// Property-count windows over otherwise-identical object branches merge when contiguous:
/// `{exactly 1 property} ∪ {exactly 2 properties}` has the single-leaf spelling negation
/// recomposition produces.
///
/// ```text
/// BEFORE: {"anyOf": [{"type": "object", "minProperties": 1, "maxProperties": 1},
///                    {"type": "object", "minProperties": 2, "maxProperties": 2}]}
/// AFTER:  {"type": "object", "minProperties": 1, "maxProperties": 2}
/// ```
pub(super) fn merge_object_count_windows(branches: &mut Vec<SharedSchema>) -> bool {
    // `Some((minimum, maximum))` only when the requirements are purely a count window.
    fn count_window(leaf: &ObjectLeaf) -> Option<(BoundCardinality, Option<BoundCardinality>)> {
        let mut minimum = BoundCardinality::from(0_u8);
        let mut maximum: Option<BoundCardinality> = None;
        for requirement in &leaf.requirements {
            match requirement {
                ObjectRequirement::MinProperties(bound) => minimum.clone_from(bound),
                ObjectRequirement::MaxProperties(bound) => maximum = Some(bound.clone()),
                _ => return None,
            }
        }
        Some((minimum, maximum))
    }
    merge_intervals(
        branches,
        |schema| match schema {
            Schema::Object(leaf) => {
                let (minimum, maximum) = count_window(leaf)?;
                // Blank the count requirements: the rest is the merge key (constraints, ...).
                let mut key = leaf.clone();
                key.requirements = Vec::new();
                Some(CardinalityInterval {
                    key,
                    minimum,
                    maximum,
                })
            }
            _ => None,
        },
        |interval| {
            let mut leaf = interval.key;
            if !interval.minimum.is_zero() {
                leaf.requirements
                    .push(ObjectRequirement::MinProperties(interval.minimum));
            }
            if let Some(maximum) = interval.maximum {
                leaf.requirements
                    .push(ObjectRequirement::MaxProperties(maximum));
            }
            Schema::Object(leaf)
        },
    )
}

fn merge_intervals<K: Clone + Eq + Ord>(
    branches: &mut Vec<SharedSchema>,
    extract: impl Fn(&Schema) -> Option<CardinalityInterval<K>>,
    into_schema: impl Fn(CardinalityInterval<K>) -> Schema,
) -> bool {
    let mut indices: Vec<usize> = Vec::new();
    let mut intervals: Vec<CardinalityInterval<K>> = Vec::new();
    for (index, branch) in branches.iter().enumerate() {
        if let Some(interval) = extract(branch.as_schema()) {
            indices.push(index);
            intervals.push(interval);
        }
    }
    if intervals.len() < 2 {
        return false;
    }
    // Group by merge key (the discriminant that must match to fuse), then order by lower bound.
    intervals.sort_by(|a, b| a.key.cmp(&b.key).then_with(|| a.minimum.cmp(&b.minimum)));
    let mut merged: Vec<CardinalityInterval<K>> = Vec::with_capacity(intervals.len());
    for interval in intervals {
        match merged.last_mut() {
            Some(last) if last.key == interval.key && last.can_merge_with(&interval) => {
                last.absorb_upper(&interval);
            }
            _ => merged.push(interval),
        }
    }
    if merged.len() == indices.len() {
        return false;
    }
    let drop: AHashSet<usize> = indices.into_iter().collect();
    let mut new_branches: Vec<SharedSchema> = branches
        .iter()
        .enumerate()
        .filter(|(index, _)| !drop.contains(index))
        .map(|(_, branch)| Arc::clone(branch))
        .collect();
    for interval in merged {
        new_branches.push(shared(into_schema(interval)));
    }
    *branches = new_branches;
    true
}

/// A count/length window over a string, array, or object leaf: a `[minimum, maximum]` range alongside `key`,
/// the bundle of every non-count facet. Windows merge only on equal keys, so the key doubles as the grouping
/// discriminant `merge_intervals` sorts and gates on.
#[derive(Clone)]
struct CardinalityInterval<K> {
    key: K,
    minimum: BoundCardinality,
    maximum: Option<BoundCardinality>,
}

impl<K> CardinalityInterval<K> {
    /// Contiguous when the higher window starts no later than one past the lower window's end; an
    /// unbounded lower window abuts everything above it.
    fn can_merge_with(&self, other: &Self) -> bool {
        match &self.maximum {
            None => true,
            Some(end) => other.minimum <= end.clone() + BoundCardinality::from(1_u8),
        }
    }

    fn absorb_upper(&mut self, other: &Self) {
        self.maximum = match (&self.maximum, &other.maximum) {
            (Some(a), Some(b)) => Some(if a > b { a.clone() } else { b.clone() }),
            _ => None,
        };
    }
}
