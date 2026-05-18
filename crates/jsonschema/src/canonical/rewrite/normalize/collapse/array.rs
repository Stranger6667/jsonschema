//! Array-domain collapse passes.

use std::sync::Arc;

use num_traits::Zero;

use crate::canonical::{
    context::CanonicalizationContext,
    coverage::any_sibling_covers,
    intern::shared,
    ir::{BoundCardinality, ContainsClause, LengthBounds, Schema, SharedSchema},
};

use super::combinators::fuse_ray_pair;

/// A demand-free `contains` clause (`minContains: 0`) drops when the arrays it excludes (those past
/// its `maxContains`) are already covered by a sibling - the wide overlapping spelling is canonical.
///
/// ```text
/// BEFORE: {"anyOf": [{"type": "array", "maxItems": 1,
///                     "contains": {"type": "integer"}, "minContains": 0, "maxContains": 0},
///                    {"type": "array", "maxItems": 2,
///                     "contains": {"type": "integer"}, "minContains": 0, "maxContains": 1}]}
/// AFTER:  {"anyOf": [{"type": "array", "maxItems": 1},
///                    {"type": "array", "maxItems": 2,
///                     "contains": {"type": "integer"}, "minContains": 0, "maxContains": 1}]}
/// ```
pub(super) fn drop_contains_upper_bound_covered_by_sibling(
    branches: &mut [SharedSchema],
    ctx: &CanonicalizationContext,
) -> bool {
    super::drop_facet_covered_by_sibling(branches, ctx, |schema| {
        let Schema::Array(leaf) = schema else {
            return None;
        };
        let position = leaf
            .contains
            .iter()
            .position(|clause| clause.min_contains.is_zero() && clause.max_contains.is_some())?;
        let clause = &leaf.contains[position];
        let above = clause
            .max_contains
            .clone()
            .expect("position matched an upper bound")
            + BoundCardinality::from(1_u8);
        let mut weakened = leaf.clone();
        weakened.contains.remove(position);
        // The dropped clause excluded exactly the arrays holding more matches than its cap.
        let mut delta = weakened.clone();
        delta.contains.push(ContainsClause {
            schema: Arc::clone(&clause.schema),
            min_contains: above,
            max_contains: None,
        });
        delta.contains.sort();
        Some((
            shared(Schema::Array(weakened)),
            shared(Schema::Array(delta)),
        ))
    })
}

/// An array beside `not: {uniqueItems: true}` folds the negation in as a "must contain a repeated item" constraint.
/// Requiring unique and repeated items at once is unsatisfiable, so it collapses to `false`.
///
/// ```text
/// BEFORE: {"allOf": [{"type": "array"}, {"not": {"type": "array", "uniqueItems": true}}]}
/// AFTER:  {"type": "array", "minItems": 2, "allOf": [{"not": {"type": "array", "uniqueItems": true}}]}
/// ```
pub(super) fn absorb_repeated_items_siblings(branches: &mut Vec<SharedSchema>) -> bool {
    // True if `schema` is `Not(array leaf with only uniqueItems set)`.
    fn not_only_unique_items(schema: &Schema) -> bool {
        let Schema::Not(inner) = schema else {
            return false;
        };
        let Schema::Array(leaf) = inner.as_schema() else {
            return false;
        };
        leaf.unique_items
            && !leaf.repeated_items
            && leaf.prefix.is_empty()
            && matches!(leaf.tail.as_schema(), Schema::True)
            && leaf.length == LengthBounds::default()
            && leaf.contains.is_empty()
    }

    if !branches
        .iter()
        .any(|b| matches!(b.as_schema(), Schema::Array(_)))
    {
        return false;
    }
    let before = branches.len();
    branches.retain(|b| !not_only_unique_items(b.as_schema()));
    if branches.len() == before {
        return false;
    }
    let (index, leaf) = branches
        .iter()
        .enumerate()
        .find_map(|(index, branch)| match branch.as_schema() {
            Schema::Array(leaf) => Some((index, leaf)),
            _ => None,
        })
        .expect("positive array leaf still present");
    let mut merged = leaf.clone();
    merged.repeated_items = true;
    if let Some(leaf) = merged.normalize_repeated_items() {
        branches[index] = shared(Schema::Array(leaf));
    } else {
        *branches = vec![shared(Schema::False)];
    }
    true
}

/// The `minItems` bound drops when the clipped slice (same facets, length below the bound) is covered by a sibling -
/// the wide overlapping spelling is canonical. Array analog of
/// [`drop_min_properties_covered_by_sibling`](super::object::drop_min_properties_covered_by_sibling).
///
/// ```text
/// BEFORE: {"anyOf": [{"type": "array", "uniqueItems": true, "minItems": 3},
///                    {"type": "array", "maxItems": 2}]}
/// AFTER:  {"anyOf": [{"type": "array", "uniqueItems": true},
///                    {"type": "array", "maxItems": 2}]}   // short unique arrays are covered
/// ```
/// A `not uniqueItems` ray floors at `minItems: 2`, but a bounded sibling covering that length raises the floor away
/// so the widened ray and negation's minimal spelling converge. Caps/reverts to avoid creeping; repeated-items only.
///
/// ```text
/// BEFORE: {"anyOf": [{"not": {"uniqueItems": true}, "minItems": 2}, {"type": "array", "maxItems": 2, "minItems": 1}]}
/// AFTER:  {"anyOf": [{"not": {"uniqueItems": true}, "minItems": 3}, {"type": "array", "maxItems": 2, "minItems": 1}]}
/// ```
pub(super) fn raise_array_min_items_covered_by_sibling(
    branches: &mut [SharedSchema],
    ctx: &CanonicalizationContext,
) -> bool {
    let mut changed = false;
    for index in 0..branches.len() {
        let Schema::Array(leaf) = branches[index].as_schema() else {
            continue;
        };
        if !leaf.repeated_items {
            continue;
        }
        let leaf = leaf.clone();
        let original = leaf.length.minimum.clone();
        let mut candidate = original.clone();
        let mut settled = false;
        for _ in 0..8_u8 {
            // Never raise to or past a finite maximum: a fully-covered leaf is left for subsumption.
            if leaf
                .length
                .maximum
                .as_ref()
                .is_some_and(|maximum| &candidate >= maximum)
            {
                break;
            }
            let mut slice = leaf.clone();
            slice.length.minimum.clone_from(&candidate);
            slice.length.maximum = Some(candidate.clone());
            let slice = shared(Schema::Array(slice));
            let covered = any_sibling_covers(branches, &[index], &slice, ctx);
            if !covered {
                settled = true;
                break;
            }
            candidate += BoundCardinality::from(1_u8);
        }
        if settled && candidate != original {
            let mut raised = leaf;
            raised.length.minimum = candidate;
            branches[index] = shared(Schema::Array(raised));
            changed = true;
        }
    }
    changed
}

/// A `not uniqueItems` window un-clips to the unbounded ray once a sibling covers every length above its `maxItems` -
/// the cap is an artefact. The `maxItems` mirror of [`raise_array_min_items_covered_by_sibling`], repeated-items only.
///
/// ```text
/// BEFORE: {"anyOf": [{"not": {"uniqueItems": true}, "maxItems": 2, "minItems": 2}, {"type": "array", "minItems": 3}]}
/// AFTER:  {"anyOf": [{"not": {"uniqueItems": true}, "minItems": 2}, {"type": "array", "minItems": 3}]}
/// ```
pub(super) fn unclip_repeated_items_ray_covered_by_sibling(
    branches: &mut [SharedSchema],
    ctx: &CanonicalizationContext,
) -> bool {
    let mut changed = false;
    for index in 0..branches.len() {
        let Schema::Array(leaf) = branches[index].as_schema() else {
            continue;
        };
        if !leaf.repeated_items {
            continue;
        }
        let Some(maximum) = leaf.length.maximum.clone() else {
            continue;
        };
        let leaf = leaf.clone();
        let mut tail = leaf.clone();
        tail.length.minimum = maximum + BoundCardinality::from(1_u8);
        tail.length.maximum = None;
        let tail = shared(Schema::Array(tail));
        if !any_sibling_covers(branches, &[index], &tail, ctx) {
            continue;
        }
        let mut unclipped = leaf;
        unclipped.length.maximum = None;
        branches[index] = shared(Schema::Array(unclipped));
        changed = true;
    }
    changed
}

pub(super) fn drop_min_items_covered_by_sibling(
    branches: &mut [SharedSchema],
    ctx: &CanonicalizationContext,
) -> bool {
    super::drop_facet_covered_by_sibling(branches, ctx, |schema| {
        let Schema::Array(leaf) = schema else {
            return None;
        };
        if leaf.repeated_items || leaf.length.minimum.is_zero() {
            return None;
        }
        let below = leaf.length.minimum.clone() - BoundCardinality::from(1_u8);
        let mut weakened = leaf.clone();
        weakened.length.minimum = BoundCardinality::from(0u64);
        // The below-bound slice keeps every other facet and caps the length at `minItems - 1`.
        let mut delta = weakened.clone();
        delta.length.maximum = Some(below);
        Some((
            shared(Schema::Array(weakened)),
            shared(Schema::Array(delta)),
        ))
    })
}

/// Negation splits a facet-carrying array branch into two length rays around a sibling-covered
/// window; rejoin them once the gap is covered.
///
/// ```text
/// BEFORE: {"anyOf": [{"type": "array", "uniqueItems": true, "maxItems": 2},
///                    {"type": "array", "uniqueItems": true, "minItems": 4},
///                    {"type": "array", "minItems": 3, "maxItems": 3}]}
/// AFTER:  {"anyOf": [{"type": "array", "uniqueItems": true},
///                    {"type": "array", "minItems": 3, "maxItems": 3}]}
/// ```
pub(super) fn rejoin_clipped_array_length_rays(
    branches: &mut Vec<SharedSchema>,
    ctx: &CanonicalizationContext,
) -> bool {
    for top_index in 0..branches.len() {
        let Schema::Array(top) = branches[top_index].as_schema() else {
            continue;
        };
        let Some(top_max) = &top.length.maximum else {
            continue;
        };
        let merged_minimum = top.length.minimum.clone();
        for bottom_index in 0..branches.len() {
            if bottom_index == top_index {
                continue;
            }
            let Schema::Array(bottom) = branches[bottom_index].as_schema() else {
                continue;
            };
            if bottom.length.maximum.is_some() || bottom.length.minimum.is_zero() {
                continue;
            }
            let gap_low = top_max.clone() + BoundCardinality::from(1_u8);
            let gap_high = bottom.length.minimum.clone() - BoundCardinality::from(1_u8);
            if gap_low > gap_high {
                continue;
            }
            // The rays must agree on every facet besides length.
            let mut joined = top.clone();
            joined.length = LengthBounds::default();
            let mut bottom_unbounded = bottom.clone();
            bottom_unbounded.length = LengthBounds::default();
            if joined != bottom_unbounded {
                continue;
            }
            let mut gap = joined.clone();
            gap.length = LengthBounds {
                minimum: gap_low,
                maximum: Some(gap_high),
            };
            let gap = shared(Schema::Array(gap));
            let covered = any_sibling_covers(branches, &[top_index, bottom_index], &gap, ctx);
            if !covered {
                continue;
            }
            // Keep the lower window's floor; the merged ray spans `[top.minimum, inf)`.
            joined.length.minimum.clone_from(&merged_minimum);
            return fuse_ray_pair(branches, top_index, bottom_index, Schema::Array(joined));
        }
    }
    false
}

/// An empty-array branch unioned with a `minItems: 1` branch widens it to `minItems: 0` and drops the empty branch
/// (prefix/tail/contains/`uniqueItems` are all vacuous on `[]`). Re-fuses the length partition negation produces.
///
/// ```text
/// BEFORE: {"anyOf": [{"type": "array", "maxItems": 0},
///                    {"type": "array", "minItems": 1, "items": {"type": "integer"}}]}
/// AFTER:  {"type": "array", "items": {"type": "integer"}}   // minItems drops to 0
/// ```
pub(super) fn merge_empty_array_branch(branches: &mut Vec<SharedSchema>) -> bool {
    let empty_index = branches.iter().position(|branch| {
        matches!(branch.as_schema(), Schema::Array(leaf)
            if leaf.length.maximum.as_ref().is_some_and(Zero::is_zero) && !leaf.repeated_items)
    });
    let Some(empty_index) = empty_index else {
        return false;
    };
    let target_index = branches.iter().position(|branch| {
        matches!(branch.as_schema(), Schema::Array(leaf)
            if leaf.length.minimum == 1u64
                && !leaf.repeated_items
                && leaf.contains.iter().all(|clause| clause.min_contains.is_zero()))
    });
    let Some(target_index) = target_index else {
        return false;
    };
    let Schema::Array(leaf) = branches[target_index].as_schema() else {
        // `position` matched on the Array variant.
        unreachable!("non-array at matched index");
    };
    let mut widened = leaf.clone();
    widened.length.minimum = BoundCardinality::from(0u64);
    branches[target_index] = shared(Schema::Array(widened));
    branches.remove(empty_index);
    true
}

/// `uniqueItems` is vacuous below length 2 (normalization strips it there), so a short array branch (`maxItems <= 1`)
/// and a `uniqueItems` branch starting one past it, with all other facets equal, are really one window.
///
/// ```text
/// BEFORE: {"anyOf": [{"type": "array", "maxItems": 1},
///                    {"type": "array", "minItems": 2, "uniqueItems": true}]}
/// AFTER:  {"type": "array", "uniqueItems": true}   // uniqueItems is vacuous at length <= 1
/// ```
pub(super) fn merge_vacuous_unique_array_windows(branches: &mut Vec<SharedSchema>) -> bool {
    for short_index in 0..branches.len() {
        let Schema::Array(short) = branches[short_index].as_schema() else {
            continue;
        };
        let Some(short_max) = short
            .length
            .maximum
            .as_ref()
            .and_then(BoundCardinality::to_u64)
        else {
            continue;
        };
        if short_max > 1 || short.unique_items || short.repeated_items {
            continue;
        }
        for unique_index in 0..branches.len() {
            if unique_index == short_index {
                continue;
            }
            let Schema::Array(unique) = branches[unique_index].as_schema() else {
                continue;
            };
            if !unique.unique_items
                || unique.repeated_items
                || unique.length.minimum != short_max + 1
                || unique.prefix != short.prefix
                || unique.tail != short.tail
                || unique.contains != short.contains
            {
                continue;
            }
            let mut merged = unique.clone();
            merged.length.minimum = short.length.minimum.owned();
            branches[unique_index] = shared(Schema::Array(merged));
            branches.remove(short_index);
            return true;
        }
    }
    false
}
