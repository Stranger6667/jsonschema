//! Domain-agnostic branch-merging infrastructure.

use std::sync::Arc;

use crate::canonical::{
    context::CanonicalizationContext,
    coverage::covers_with,
    intern::shared,
    intersect::intersect_reduced,
    ir::{Schema, SharedSchema},
    prover::Prover,
};

/// Intersect `allOf` siblings that pin the same JSON kind into one leaf. Without this, N `if/then` rewrites
/// distributed over an `allOf` blow up to a 2^N cartesian.
///
/// ```text
/// BEFORE: {"allOf": [{"type": "integer", "minimum": 5}, {"type": "integer", "maximum": 10}]}
/// AFTER:  {"type": "integer", "minimum": 5, "maximum": 10}
/// ```
pub(super) fn intersect_same_kind_siblings(
    branches: &mut Vec<SharedSchema>,
    ctx: &CanonicalizationContext,
) -> bool {
    try_pairwise_intersect(
        branches,
        ctx,
        |branch| {
            branch
                .as_schema()
                .pinned_kind()
                .map(|kind| (kind, Arc::clone(branch)))
        },
        |(outer_kind, _), other| other.as_schema().pinned_kind() == Some(*outer_kind),
        true,
    )
}

/// A typed branch sitting next to a union distributes into the union: each member is intersected with the typed
/// branch, members of a disjoint type drop out, and same-type members merge.
///
/// ```text
/// BEFORE: {"allOf": [{"type": "integer", "minimum": 0},
///                    {"anyOf": [{"type": "integer", "maximum": 10}, {"type": "string"}]}]}
/// AFTER:  {"type": "integer", "minimum": 0, "maximum": 10}   // the string member is disjoint, dropped
/// ```
pub(super) fn intersect_any_of_with_typed_sibling(
    branches: &mut Vec<SharedSchema>,
    ctx: &CanonicalizationContext,
) -> bool {
    try_pairwise_intersect(
        branches,
        ctx,
        |branch| branch.as_typed_view().is_some().then_some(()),
        |(), other| matches!(other.as_schema(), Schema::AnyOf(_)),
        false,
    )
}

/// Above this many `AnyOf` siblings, skip pairwise merging: each attempt is a full pipeline pass, so large N dominates
/// even when every merge is rejected. Siblings then remain separate `AllOf` children - equivalent, less canonical.
const ANYOF_PAIRWISE_LIMIT: usize = 16;

/// Two unions side by side distribute to one flat union of pairwise intersections: cross-type pairs collapse to
/// `false`, same-type pairs merge.
///
/// ```text
/// BEFORE: {"allOf": [{"anyOf": [{"type": "integer"}, {"type": "string"}]},
///                    {"anyOf": [{"type": "integer"}, {"type": "boolean"}]}]}
/// AFTER:  {"type": "integer"}   // only the integer-with-integer pair survives
/// ```
pub(super) fn intersect_any_of_siblings(
    branches: &mut Vec<SharedSchema>,
    ctx: &CanonicalizationContext,
) -> bool {
    let anyof_count = branches
        .iter()
        .filter(|b| matches!(b.as_schema(), Schema::AnyOf(_)))
        .count();
    if anyof_count > ANYOF_PAIRWISE_LIMIT {
        return false;
    }
    try_pairwise_intersect_capped(
        branches,
        ctx,
        |branch| match branch.as_schema() {
            Schema::AnyOf(inner) => Some(inner.len()),
            _ => None,
        },
        |_, other| matches!(other.as_schema(), Schema::AnyOf(_)),
        true,
        // Reject merges that grow the branch count; N if/then clauses otherwise distribute to 2^N.
        |left_size, right, merged| {
            let right_size = match right.as_schema() {
                Schema::AnyOf(inner) => inner.len(),
                _ => 1,
            };
            let merged_size = match merged.as_schema() {
                Schema::AnyOf(inner) => inner.len(),
                _ => 1,
            };
            merged_size > *left_size + right_size
        },
    )
}

/// Replace an outer branch (`pick_outer`) and an inner one (`should_merge`) with their intersection, when it reduces
/// the pair rather than re-wrapping as `AllOf`. `forward_only` scans only past the outer index, for symmetric predicates.
pub(super) fn try_pairwise_intersect<O>(
    branches: &mut Vec<SharedSchema>,
    ctx: &CanonicalizationContext,
    pick_outer: impl Fn(&SharedSchema) -> Option<O>,
    should_merge: impl Fn(&O, &SharedSchema) -> bool,
    forward_only: bool,
) -> bool {
    try_pairwise_intersect_capped(
        branches,
        ctx,
        pick_outer,
        should_merge,
        forward_only,
        |_, _, _| false,
    )
}

/// As [`try_pairwise_intersect`], but rejects a merge that passes the "is unresolved" check while growing the structure
/// according to `is_blown_up(outer_info, inner, merged)`.
fn try_pairwise_intersect_capped<O>(
    branches: &mut Vec<SharedSchema>,
    ctx: &CanonicalizationContext,
    pick_outer: impl Fn(&SharedSchema) -> Option<O>,
    should_merge: impl Fn(&O, &SharedSchema) -> bool,
    forward_only: bool,
    is_blown_up: impl Fn(&O, &SharedSchema, &SharedSchema) -> bool,
) -> bool {
    if branches.len() <= 1 {
        return false;
    }
    for outer_idx in 0..branches.len() {
        let Some(outer_info) = pick_outer(&branches[outer_idx]) else {
            continue;
        };
        let inner_start = if forward_only { outer_idx + 1 } else { 0 };
        for inner_idx in inner_start..branches.len() {
            if inner_idx == outer_idx || !should_merge(&outer_info, &branches[inner_idx]) {
                continue;
            }
            let left = Arc::clone(&branches[outer_idx]);
            let right = Arc::clone(&branches[inner_idx]);
            // A pair the leaf algebra does not reduce must stay split: committing its residual
            // `AllOf` restructures under canonicalization and re-triggers this fold every sweep.
            let Some(merged) = intersect_reduced(&left, &right, ctx) else {
                continue;
            };
            if is_blown_up(&outer_info, &right, &merged) {
                continue;
            }
            return replace_pair_with(branches, outer_idx, inner_idx, merged);
        }
    }
    false
}

/// Drop entries strictly dominated by some sibling. `is_dominated(candidate, sibling)` returns `true` when `candidate`
/// carries no information `sibling` does not already imply; an already-dropped sibling never dominates, and a
/// non-strict predicate must break ties itself so two equivalent entries don't both drop.
pub(crate) fn drop_strictly_dominated<T>(
    branches: &mut Vec<T>,
    is_dominated: impl Fn(&T, &T) -> bool,
) -> bool {
    if branches.len() <= 1 {
        return false;
    }
    let original_len = branches.len();
    let mut keep = vec![true; branches.len()];
    for (index, candidate) in branches.iter().enumerate() {
        for (other_index, sibling) in branches.iter().enumerate() {
            if index == other_index || !keep[other_index] {
                continue;
            }
            if is_dominated(candidate, sibling) {
                keep[index] = false;
                break;
            }
        }
    }
    let mut keep_iter = keep.iter();
    branches.retain(|_| *keep_iter.next().expect("matching length"));
    branches.len() != original_len
}

/// In a conjunction, a broader branch is redundant when a stricter sibling already implies it.
///
/// ```text
/// BEFORE: {"allOf": [{"type": "integer", "minimum": 5}, {"type": "integer"}]}
/// AFTER:  {"type": "integer", "minimum": 5}
/// ```
pub(super) fn drop_covering_branches(
    branches: &mut Vec<SharedSchema>,
    ctx: &CanonicalizationContext,
) -> bool {
    let prover = Prover::without_definitions(ctx);
    drop_strictly_dominated(branches, |candidate, sibling| {
        covers_with(&prover, candidate, sibling) && !covers_with(&prover, sibling, candidate)
    })
}

/// In a disjunction, a narrower branch is redundant when a broader sibling already accepts everything it does.
///
/// ```text
/// BEFORE: {"anyOf": [{"type": "integer", "minimum": 5}, {"type": "integer"}]}
/// AFTER:  {"type": "integer"}
/// ```
pub(super) fn drop_subsumed_branches(
    branches: &mut Vec<SharedSchema>,
    ctx: &CanonicalizationContext,
) -> bool {
    let prover = Prover::without_definitions(ctx);
    drop_strictly_dominated(branches, |candidate, sibling| {
        covers_with(&prover, sibling, candidate) && !covers_with(&prover, candidate, sibling)
    })
}

/// Drop the two branches at `first`/`second` and append `merged` at the end. Removing the higher
/// index first is load-bearing: it leaves the lower index valid for the second removal.
pub(super) fn replace_pair_with(
    branches: &mut Vec<SharedSchema>,
    first: usize,
    second: usize,
    merged: SharedSchema,
) -> bool {
    branches.remove(first.max(second));
    branches.remove(first.min(second));
    branches.push(merged);
    true
}

/// Fuse a ray pair found at `first`/`second` into `merged`, keeping the lower index and dropping
/// the higher. Removing the higher index first is load-bearing: it leaves the kept index valid.
pub(super) fn fuse_ray_pair(
    branches: &mut Vec<SharedSchema>,
    first: usize,
    second: usize,
    merged: Schema,
) -> bool {
    let keep = first.min(second);
    let drop = first.max(second);
    branches[keep] = shared(merged);
    branches.remove(drop);
    true
}

/// Sort and dedup so `AllOf([A, B])` and `AllOf([B, A])` canonicalize the same. Skips the sort when input is already
/// strictly ascending.
pub(super) fn sort_dedup(branches: &mut Vec<SharedSchema>) -> bool {
    if branches.len() <= 1 {
        return false;
    }
    let already_canonical = branches.windows(2).all(|window| window[0] < window[1]);
    if already_canonical {
        return false;
    }
    branches.sort();
    branches.dedup();
    true
}
