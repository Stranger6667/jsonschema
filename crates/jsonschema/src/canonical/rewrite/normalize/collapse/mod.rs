#![cfg_attr(not(feature = "arbitrary-precision"), allow(clippy::clone_on_copy))]

//! Collapse: remove redundant wrappers and merge equivalent `allOf`/`anyOf` branches.
//!
//! Each pass runs once per pipeline sweep, in table order (no local fixpoint); convergence comes
//! from `canonicalize_ir` repeating the whole pipeline.

mod array;
mod combinators;
mod guards;
mod negation;
mod object;
mod string;
mod value_set;

pub(crate) use combinators::drop_strictly_dominated;

use std::sync::Arc;

use crate::{
    canonical::{
        context::{CanonicalizationContext, WalkStage},
        intern::shared,
        intersect::multi_type_or_false,
        ir::{BooleanBounds, IntegerLeaf, NumberLeaf, Schema, SharedSchema, StringLeaf},
        leaves::Leaf,
        numeric,
        walk::map_children,
    },
    JsonType, JsonTypeSet,
};

use super::intervals::{
    merge_array_count_windows, merge_object_count_windows, merge_string_length_windows,
};

/// Weaken one facet per branch when a sibling covers the values the weakening newly admits.
/// `build` returns the weakened branch and the newly-admitted delta, or `None` to skip the branch.
fn drop_facet_covered_by_sibling(
    branches: &mut [SharedSchema],
    ctx: &CanonicalizationContext,
    build: impl Fn(&Schema) -> Option<(SharedSchema, SharedSchema)>,
) -> bool {
    let mut changed = false;
    for index in 0..branches.len() {
        let Some((weakened, delta)) = build(branches[index].as_schema()) else {
            continue;
        };
        if crate::canonical::coverage::any_sibling_covers(branches, &[index], &delta, ctx) {
            branches[index] = weakened;
            changed = true;
        }
    }
    changed
}

use self::{
    array::{
        absorb_repeated_items_siblings, drop_contains_upper_bound_covered_by_sibling,
        drop_min_items_covered_by_sibling, merge_empty_array_branch,
        merge_vacuous_unique_array_windows, raise_array_min_items_covered_by_sibling,
        rejoin_clipped_array_length_rays, unclip_repeated_items_ray_covered_by_sibling,
    },
    combinators::{
        drop_covering_branches, drop_subsumed_branches, intersect_any_of_siblings,
        intersect_any_of_with_typed_sibling, intersect_same_kind_siblings, sort_dedup,
    },
    guards::{
        all_of_type_domain_empty, covers_every_json_type, disjoint_kind_guards_cover_everything,
        fold_guard_conjunction_any_of, fold_to_multi_type, fold_type_guard_any_of,
        intersect_multi_type_siblings, intersect_multi_type_with_any_of_sibling,
        intersect_multi_type_with_typed_sibling, intersect_type_guard_siblings,
        merge_dual_facet_leaves, split_guard_conjunction_in_any_of, split_type_guard_in_any_of,
    },
    negation::{
        any_of_contains_complementary_pair, combine_negations, distribute_not_through_any_of,
        drop_disjoint_negations, drop_value_disjoint_negations, not_branch_contradicts_positive,
    },
    object::{
        drop_min_properties_covered_by_sibling, drop_object_conjunct_implied_by_sibling,
        drop_property_constraint_covered_by_sibling,
        drop_required_properties_covered_by_optional_property_branch,
        drop_required_property_covered_by_sibling, object_siblings_contradict,
        saturate_stalled_object_conjuncts,
    },
    string::{
        absorb_not_pattern_siblings, canonicalize_string_format_conjunction,
        drop_string_pattern_facet_covered_by_sibling,
    },
    value_set::{
        drop_covered_value_set_members, factor_value_set_conjunctions,
        intersect_value_set_siblings, intersect_value_set_with_typed_sibling, merge_enum_branches,
        split_finite_type_values_from_enum,
    },
};

/// Branch rewrite pass over a growable member list.
type BranchPass = fn(&mut Vec<SharedSchema>) -> bool;
/// Branch rewrite pass over a growable member list, consulting the canonicalization context.
type ContextBranchPass = fn(&mut Vec<SharedSchema>, &CanonicalizationContext) -> bool;
/// Branch rewrite pass over a fixed-length member list, consulting the canonicalization context.
type ContextSlicePass = fn(&mut [SharedSchema], &CanonicalizationContext) -> bool;

/// Simplify a canonical schema by removing redundant wrappers and merging equivalent branches.
///
/// Collapses `allOf`/`anyOf` nodes whose branches merge into a single typed leaf, and strips internal type-guard
/// wrappers already satisfied by the inner node.
///
/// ```text
/// BEFORE: {"allOf": [{"type": "integer", "minimum": 5},
///                    {"type": "integer", "maximum": 10}]}
/// AFTER:  {"type": "integer", "minimum": 5, "maximum": 10}
///
/// BEFORE: {"anyOf": [{"type": "integer", "minimum": 5},
///                    {"type": "integer", "minimum": 0}]}
/// AFTER:  {"type": "integer", "minimum": 0}
///
/// BEFORE: {"allOf": [{"type": "integer"}]}
/// AFTER:  {"type": "integer"}
/// ```
#[must_use]
pub(crate) fn collapse(schema: &SharedSchema, ctx: &CanonicalizationContext) -> SharedSchema {
    ctx.with_walk_memo(WalkStage::Collapse, schema, || match schema.as_schema() {
        Schema::AllOf(branches) => collapse_all_of(schema, branches, ctx),
        Schema::AnyOf(branches) => collapse_any_of(schema, branches, ctx),
        Schema::TypedGroup { ty, body } => collapse_typed_group(schema, *ty, body, ctx),
        Schema::TypeGuard { ty, body } => collapse_type_guard(schema, *ty, body, ctx),
        _ => map_children(schema, |child| collapse(child, ctx)),
    })
}

/// Drop a `TypedGroup` whose body already pins the same type. Union bodies first shed members
/// outside `ty` (unreachable under `ty ∧ body`); if every survivor is `ty`-typed the wrapper is redundant.
fn collapse_typed_group(
    schema: &SharedSchema,
    ty: JsonType,
    body: &SharedSchema,
    ctx: &CanonicalizationContext,
) -> SharedSchema {
    let mut recursed_body = collapse(body, ctx);
    if let Schema::AnyOf(members) = recursed_body.as_schema() {
        if let Some(filtered) = filter_group_body_members(ty, members) {
            recursed_body = collapse(&shared(Schema::AnyOf(filtered)), ctx);
        }
    }
    if body_matches_ty(&recursed_body, ty) {
        return recursed_body;
    }
    if Arc::ptr_eq(&recursed_body, body) {
        return Arc::clone(schema);
    }
    shared(Schema::TypedGroup {
        ty,
        body: recursed_body,
    })
}

/// `Some(survivors)` when some union member is disjoint from `ty` and can be dropped; `None`
/// when nothing changes.
fn filter_group_body_members(ty: JsonType, members: &[SharedSchema]) -> Option<Vec<SharedSchema>> {
    let group_set = JsonTypeSet::from(ty);
    let mut survivors: Vec<SharedSchema> = Vec::with_capacity(members.len());
    let mut changed = false;
    for member in members {
        match member.as_schema() {
            Schema::MultiType(set) => {
                let narrowed = Schema::type_set_intersect(*set, group_set);
                if narrowed == *set {
                    survivors.push(Arc::clone(member));
                } else {
                    changed = true;
                    if narrowed.is_empty() {
                        continue;
                    }
                    survivors.push(multi_type_or_false(narrowed));
                }
            }
            other => {
                if other.pinned_kind().is_some_and(|kind| !ty.overlaps(kind)) {
                    changed = true;
                    continue;
                }
                survivors.push(Arc::clone(member));
            }
        }
    }
    changed.then_some(survivors)
}

fn collapse_type_guard(
    schema: &SharedSchema,
    ty: JsonType,
    body: &SharedSchema,
    ctx: &CanonicalizationContext,
) -> SharedSchema {
    let recursed_body = collapse(body, ctx);
    if body_accepts_all_guarded_values(&recursed_body, ty) {
        return shared(Schema::True);
    }
    if Arc::ptr_eq(&recursed_body, body) {
        return Arc::clone(schema);
    }
    shared(Schema::TypeGuard {
        ty,
        body: recursed_body,
    })
}

fn body_accepts_all_guarded_values(body: &SharedSchema, ty: JsonType) -> bool {
    match body.as_schema() {
        Schema::True => true,
        Schema::Null => ty == JsonType::Null,
        Schema::Boolean(BooleanBounds::Any) => ty == JsonType::Boolean,
        Schema::Integer(leaf) => ty == JsonType::Integer && *leaf == IntegerLeaf::default(),
        Schema::Number(leaf) => {
            ty.guard_slot() == JsonType::Number && *leaf == NumberLeaf::default()
        }
        Schema::String(leaf) => ty == JsonType::String && *leaf == StringLeaf::default(),
        Schema::Array(leaf) => ty == JsonType::Array && leaf.is_open(),
        Schema::Object(leaf) => ty == JsonType::Object && leaf.is_open(),
        _ => false,
    }
}

fn body_matches_ty(body: &SharedSchema, ty: JsonType) -> bool {
    // Union is `ty`-only when every arm is; intersection when any conjunct pins `ty` (it bars all other types).
    if let Schema::AnyOf(members) = body.as_schema() {
        return members.iter().all(|member| body_matches_ty(member, ty));
    }
    if let Schema::AllOf(members) = body.as_schema() {
        return members.iter().any(|member| body_matches_ty(member, ty));
    }
    let schema = body.as_schema();
    // Integers are numbers, so a `number` pin adds nothing to an integer body.
    schema.is_typed_leaf_of(ty) || (ty == JsonType::Number && matches!(schema, Schema::Integer(_)))
}

const SIBLING_INTERSECTION_PASSES: &[ContextBranchPass] = &[
    intersect_any_of_with_typed_sibling,
    intersect_multi_type_with_any_of_sibling,
    intersect_multi_type_with_typed_sibling,
    intersect_value_set_siblings,
    intersect_value_set_with_typed_sibling,
    intersect_any_of_siblings,
    intersect_type_guard_siblings,
    intersect_same_kind_siblings,
    canonicalize_string_format_conjunction,
];

fn collapse_all_of(
    schema: &SharedSchema,
    branches: &[SharedSchema],
    ctx: &CanonicalizationContext,
) -> SharedSchema {
    let mut changed = false;
    let mut cleaned: Vec<SharedSchema> = Vec::with_capacity(branches.len());
    for branch in branches {
        let next = collapse(branch, ctx);
        if !Arc::ptr_eq(&next, branch) {
            changed = true;
        }
        if matches!(next.as_schema(), Schema::True) {
            changed = true;
            continue;
        }
        if let Schema::AllOf(inner) = next.as_schema() {
            changed = true;
            for nested in inner {
                cleaned.push(Arc::clone(nested));
            }
            continue;
        }
        cleaned.push(next);
    }
    if cleaned
        .iter()
        .any(|branch| matches!(branch.as_schema(), Schema::False))
    {
        return shared(Schema::False);
    }
    if all_of_type_domain_empty(&cleaned) {
        return shared(Schema::False);
    }
    if not_branch_contradicts_positive(&cleaned, ctx) {
        return shared(Schema::False);
    }
    if object_siblings_contradict(&cleaned, ctx) {
        return shared(Schema::False);
    }
    // Before `combine_negations`, which would fuse `Not` siblings into one `Not(AnyOf)` the absorber can't recognize.
    changed |= numeric::simplify_intersection_branches(&mut cleaned);
    changed |= absorb_not_pattern_siblings(&mut cleaned);
    changed |= absorb_repeated_items_siblings(&mut cleaned);
    changed |= distribute_not_through_any_of(&mut cleaned, ctx);
    changed |= drop_disjoint_negations(&mut cleaned);
    changed |= drop_value_disjoint_negations(&mut cleaned, ctx);
    changed |= combine_negations(&mut cleaned, Schema::AnyOf);
    for pass in SIBLING_INTERSECTION_PASSES {
        changed |= pass(&mut cleaned, ctx);
    }
    changed |= saturate_stalled_object_conjuncts(&mut cleaned, ctx);
    changed |= drop_object_conjunct_implied_by_sibling(&mut cleaned);
    changed |= intersect_multi_type_siblings(&mut cleaned);
    changed |= drop_covering_branches(&mut cleaned, ctx);
    match cleaned.len() {
        0 => shared(Schema::True),
        1 => cleaned.into_iter().next().expect("len == 1"),
        _ => {
            changed |= sort_dedup(&mut cleaned);
            match cleaned.len() {
                1 => cleaned.into_iter().next().expect("len == 1"),
                _ if changed => shared(Schema::AllOf(cleaned)),
                _ => Arc::clone(schema),
            }
        }
    }
}

const MERGE_PASSES: &[BranchPass] = &[
    merge_enum_branches,
    merge_object_count_windows,
    merge_array_count_windows,
    merge_string_length_windows,
    merge_empty_array_branch,
    merge_vacuous_unique_array_windows,
];

const OBJECT_WEAKENING_PASSES: &[ContextSlicePass] = &[
    drop_required_property_covered_by_sibling,
    drop_min_properties_covered_by_sibling,
    drop_property_constraint_covered_by_sibling,
];

const ARRAY_WEAKENING_PASSES: &[ContextSlicePass] = &[
    drop_min_items_covered_by_sibling,
    raise_array_min_items_covered_by_sibling,
    unclip_repeated_items_ray_covered_by_sibling,
    drop_contains_upper_bound_covered_by_sibling,
];

const COVERAGE_PASSES: &[ContextBranchPass] =
    &[drop_subsumed_branches, drop_covered_value_set_members];

const GUARD_FOLD_PASSES: &[BranchPass] = &[
    fold_to_multi_type,
    split_type_guard_in_any_of,
    split_guard_conjunction_in_any_of,
    fold_type_guard_any_of,
    fold_guard_conjunction_any_of,
];

fn collapse_any_of(
    schema: &SharedSchema,
    branches: &[SharedSchema],
    ctx: &CanonicalizationContext,
) -> SharedSchema {
    let mut changed = false;
    let mut cleaned: Vec<SharedSchema> = Vec::with_capacity(branches.len());
    for branch in branches {
        let next = collapse(branch, ctx);
        if !Arc::ptr_eq(&next, branch) {
            changed = true;
        }
        if matches!(next.as_schema(), Schema::False) {
            changed = true;
            continue;
        }
        if let Schema::AnyOf(inner) = next.as_schema() {
            changed = true;
            for nested in inner {
                cleaned.push(Arc::clone(nested));
            }
            continue;
        }
        cleaned.push(next);
    }
    if cleaned
        .iter()
        .any(|branch| matches!(branch.as_schema(), Schema::True))
    {
        return shared(Schema::True);
    }
    if covers_every_json_type(&cleaned) {
        return shared(Schema::True);
    }
    if any_of_contains_complementary_pair(&cleaned, ctx) {
        return shared(Schema::True);
    }
    changed |= merge_dual_facet_leaves(&mut cleaned);
    if disjoint_kind_guards_cover_everything(&cleaned) {
        return shared(Schema::True);
    }
    for pass in MERGE_PASSES {
        changed |= pass(&mut cleaned);
    }
    changed |= rejoin_clipped_array_length_rays(&mut cleaned, ctx);
    changed |= factor_value_set_conjunctions(&mut cleaned);
    changed |= drop_required_properties_covered_by_optional_property_branch(&mut cleaned);
    for pass in OBJECT_WEAKENING_PASSES {
        changed |= pass(&mut cleaned, ctx);
    }
    changed |= drop_string_pattern_facet_covered_by_sibling(&mut cleaned, ctx, true);
    changed |= drop_string_pattern_facet_covered_by_sibling(&mut cleaned, ctx, false);
    for pass in ARRAY_WEAKENING_PASSES {
        changed |= pass(&mut cleaned, ctx);
    }
    changed |= split_finite_type_values_from_enum(&mut cleaned);
    for pass in COVERAGE_PASSES {
        changed |= pass(&mut cleaned, ctx);
    }
    changed |= combine_negations(&mut cleaned, Schema::AllOf);
    for pass in GUARD_FOLD_PASSES {
        changed |= pass(&mut cleaned);
    }
    match cleaned.len() {
        0 => shared(Schema::False),
        1 => cleaned.into_iter().next().expect("len == 1"),
        _ => {
            changed |= sort_dedup(&mut cleaned);
            match cleaned.len() {
                1 => cleaned.into_iter().next().expect("len == 1"),
                _ if changed => shared(Schema::AnyOf(cleaned)),
                _ => Arc::clone(schema),
            }
        }
    }
}
