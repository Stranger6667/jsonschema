use std::sync::Arc;

use crate::canonical::{
    cardinality::finite_universe_size,
    context::{CanonicalizationContext, WalkStage},
    intern::shared,
    ir::{
        ArrayLeaf, BoundCardinality, CanonicalKind, ContainsClause, LengthBounds, Schema,
        SchemaKindSet, SharedSchema,
    },
};

/// Tightens array leaves: a `false` prefix slot caps `maxItems` and drops the tail, `minContains` lifts into
/// `minItems`, and a finite item universe caps `maxItems` under `uniqueItems`.
///
/// ```text
/// BEFORE: {"type": "array", "items": [true, false, true]}
/// AFTER:  {"type": "array", "items": [true], "maxItems": 1}
///
/// BEFORE: {"type": "array", "items": {"type": "boolean"}, "uniqueItems": true}
/// AFTER:  {"type": "array", "items": {"type": "boolean"}, "uniqueItems": true, "maxItems": 2}
///
/// BEFORE: {"type": "array", "contains": {"const": 7}, "minContains": 3}
/// AFTER:  {"type": "array", "contains": {"const": 7}, "minContains": 3, "minItems": 3}
/// ```
#[must_use]
pub(crate) fn normalize(schema: &SharedSchema, ctx: &CanonicalizationContext) -> SharedSchema {
    super::run::<ArrayStage>(schema, ctx)
}

pub(crate) struct ArrayStage;

impl super::NormalizeStage for ArrayStage {
    const WALK: WalkStage = WalkStage::Array;
    const MASK: SchemaKindSet = SchemaKindSet::of(CanonicalKind::Array);

    fn rewrite(recursed: SharedSchema, _ctx: &CanonicalizationContext) -> SharedSchema {
        let Schema::Array(leaf) = recursed.as_schema() else {
            return recursed;
        };
        let mut prefix = leaf.prefix.clone();
        let mut tail = Arc::clone(&leaf.tail);
        let mut length = leaf.length.clone();
        let mut contains = leaf.contains.clone();

        let mut changed = false;
        changed |= drop_false_tail(&mut prefix, &mut tail, &mut length);
        changed |= truncate_vacuous_positions(&mut prefix, &mut tail, &length);
        // Arrays of length <= 1 are trivially unique.
        let mut unique_items = leaf.unique_items;
        if unique_items && length.maximum.as_ref().is_some_and(|max| *max <= 1u64) {
            unique_items = false;
            changed = true;
        }
        changed |= lift_contains_min(&contains, &mut length);
        changed |= strip_trivial_contains(&mut contains, &length);
        changed |= sort_dedup_contains(&mut contains);
        changed |= cap_max_for_unique_universe(leaf, &prefix, &tail, &mut length);

        let mut next_leaf = ArrayLeaf {
            prefix,
            tail,
            length,
            unique_items,
            repeated_items: leaf.repeated_items,
            contains,
        };
        match next_leaf.clone().normalize_repeated_items() {
            Some(normalized_leaf) => {
                changed |= normalized_leaf != next_leaf;
                next_leaf = normalized_leaf;
            }
            None => return shared(Schema::False),
        }

        if !changed {
            return recursed;
        }
        shared(Schema::Array(next_leaf))
    }
}

/// A `false` prefix slot at index `n` caps `maxItems` to `n` and drops the tail.
fn drop_false_tail(
    prefix: &mut Vec<SharedSchema>,
    tail: &mut SharedSchema,
    length: &mut LengthBounds,
) -> bool {
    let Some(false_index) = prefix
        .iter()
        .position(|slot| matches!(slot.as_schema(), Schema::False))
    else {
        return false;
    };
    let derived_maximum = BoundCardinality::from(false_index);
    let new_maximum = match &length.maximum {
        Some(existing) => (existing.min(&derived_maximum)).owned(),
        None => derived_maximum,
    };
    let mut changed = false;
    if length.maximum.as_ref() != Some(&new_maximum) {
        length.maximum = Some(new_maximum);
        changed = true;
    }
    if prefix.len() > false_index {
        prefix.truncate(false_index);
        changed = true;
    }
    if !matches!(tail.as_schema(), Schema::True) {
        *tail = shared(Schema::True);
        changed = true;
    }
    changed
}

/// Positions at or beyond `maxItems` are unreachable: prefix slots there are vacuous, and the tail
/// is vacuous once `maxItems <= prefix.len()`.
fn truncate_vacuous_positions(
    prefix: &mut Vec<SharedSchema>,
    tail: &mut SharedSchema,
    length: &LengthBounds,
) -> bool {
    let Some(max) = &length.maximum else {
        return false;
    };
    let mut changed = false;
    if *max < BoundCardinality::from(prefix.len()) {
        // `max < prefix.len() <= usize::MAX`, so the conversion cannot fail.
        let max_slots = max.to_usize().expect("maximum fits in usize");
        prefix.truncate(max_slots);
        changed = true;
    }
    if *max <= BoundCardinality::from(prefix.len()) && !matches!(tail.as_schema(), Schema::True) {
        *tail = shared(Schema::True);
        changed = true;
    }
    changed
}

/// Lift `min_contains` into `length.minimum`. Lift `max_contains` into `length.maximum` only for `True` clauses, where
/// matching-item count equals total-item count.
fn lift_contains_min(contains: &[ContainsClause], length: &mut LengthBounds) -> bool {
    let mut changed = false;
    let min_target = contains
        .iter()
        .map(|clause| &clause.min_contains)
        .max()
        .map_or_else(|| BoundCardinality::from(0u64), BoundCardinality::owned);
    if min_target > length.minimum {
        length.minimum = min_target;
        changed = true;
    }
    for clause in contains {
        if !matches!(clause.schema.as_schema(), Schema::True) {
            continue;
        }
        if let Some(max_match) = &clause.max_contains {
            let next_maximum = length.maximum.as_ref().map_or_else(
                || (max_match).owned(),
                |existing| (existing.min(max_match)).owned(),
            );
            if length.maximum.as_ref() != Some(&next_maximum) {
                length.maximum = Some(next_maximum);
                changed = true;
            }
        }
    }
    changed
}

/// Drop clauses that add nothing: `True` (folded into `length` by [`lift_contains_min`]), `False` with
/// `min_contains == 0`, and ones every array satisfies (`min_contains == 0` uncapped, or cap >= max length).
fn strip_trivial_contains(contains: &mut Vec<ContainsClause>, length: &LengthBounds) -> bool {
    let original = contains.len();
    contains.retain(|clause| {
        if clause.min_contains.is_zero() {
            match (&clause.max_contains, &length.maximum) {
                (None, _) => return false,
                (Some(max_contains), Some(max_items)) if max_contains >= max_items => {
                    return false;
                }
                _ => {}
            }
        }
        match clause.schema.as_schema() {
            Schema::True => false,
            Schema::False => !clause.min_contains.is_zero(),
            _ => true,
        }
    });
    contains.len() != original
}

fn sort_dedup_contains(contains: &mut Vec<ContainsClause>) -> bool {
    if contains.len() <= 1 {
        return false;
    }
    let already_canonical = contains.windows(2).all(|window| window[0] < window[1]);
    if already_canonical {
        return false;
    }
    contains.sort();
    contains.dedup();
    true
}

/// With `uniqueItems` and a finite tail universe of size `N`, cap `maxItems` to `N`. Skipped when any prefix slot
/// is present.
fn cap_max_for_unique_universe(
    original: &ArrayLeaf,
    prefix: &[SharedSchema],
    tail: &SharedSchema,
    length: &mut LengthBounds,
) -> bool {
    if !original.unique_items || !prefix.is_empty() {
        return false;
    }
    let Some(size) = finite_universe_size(tail) else {
        return false;
    };
    let size = BoundCardinality::from(size);
    let new_maximum = match &length.maximum {
        Some(existing) => (existing.min(&size)).owned(),
        None => size,
    };
    if length.maximum.as_ref() == Some(&new_maximum) {
        return false;
    }
    length.maximum = Some(new_maximum);
    true
}
