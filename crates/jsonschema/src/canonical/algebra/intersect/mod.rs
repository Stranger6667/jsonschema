//! Schema intersection - `A ∩ B` validates iff both inputs do.
//!
//! Inputs are canonicalized first, then `AnyOf` distributes via `(A ∨ B) ∩ X = (A ∩ X) ∨ (B ∩ X)`.

mod array;
mod bounds;
mod dispatch;
mod integer;
mod normalize;
mod number;
mod string;
mod value_set;

pub(crate) mod object;

use std::sync::Arc;

use crate::canonical::{
    canonicalize_ir,
    context::CanonicalizationContext,
    ir::{Schema, SharedSchema},
};

use bounds::{intersect_numeric_leaf, intersect_optional, max_option, min_option};
pub(crate) use dispatch::{intersect_internal, multi_type_or_false, open_typed_leaf};
pub(crate) use normalize::{
    normalize_integer_not_multiple_of, normalize_number_not_multiple_of,
    normalize_string_not_patterns,
};
pub(crate) use string::is_unmergeable_format_pair;
pub(crate) use value_set::{
    intersect_typed_with_value_set, intersect_value_sets, is_unmergeable_guard_value_pair,
    value_matches_typed,
};

/// Canonical intersection of `left` and `right` - the values both accept. Memoised on pointer identity; the
/// type-specific merge rules live in the `integer`/`number`/`string`/`array`/`object` submodules.
///
/// ```text
/// BEFORE: {"type": "integer", "minimum": 5}  and  {"type": "integer", "maximum": 10}
/// AFTER:  {"type": "integer", "minimum": 5, "maximum": 10}
///
/// BEFORE: {"multipleOf": 4}  and  {"multipleOf": 6}
/// AFTER:  {"multipleOf": 12}
///
/// BEFORE: {"type": "integer"}  and  {"type": "string"}
/// AFTER:  false
/// ```
pub(crate) fn intersect_canonical(
    left: &SharedSchema,
    right: &SharedSchema,
    ctx: &CanonicalizationContext,
) -> SharedSchema {
    if Arc::ptr_eq(left, right) {
        return Arc::clone(left);
    }
    // Structural equality (hash-gated) shortcuts idempotent intersections that pointer identity
    // misses, e.g. operands rebuilt by separate parses. Inside the memo so a repeated pair pays
    // the deep comparison once.
    ctx.with_intersect_memo(left, right, || {
        if left == right {
            return Arc::clone(left);
        }
        canonicalize_ir(&intersect_internal(left, right, ctx), ctx)
    })
}

/// True when `predicate` holds for any unordered pair of branches.
fn any_unordered_pair(
    branches: &[SharedSchema],
    mut predicate: impl FnMut(&SharedSchema, &SharedSchema) -> bool,
) -> bool {
    branches.iter().enumerate().any(|(index, left)| {
        branches[index + 1..]
            .iter()
            .any(|right| predicate(left, right))
    })
}

/// True when intersecting two same-kind numeric leaves would yield an unrepresentable combined
/// `multipleOf` and therefore bail back to `AllOf`.
pub(crate) fn is_unsafe_numeric_pair(left: &SharedSchema, right: &SharedSchema) -> bool {
    match (left.as_schema(), right.as_schema()) {
        (Schema::Integer(left), Schema::Integer(right)) => {
            match (left.multiple_of.as_ref(), right.multiple_of.as_ref()) {
                (Some(l), Some(r)) => l.checked_lcm(r).is_none(),
                _ => false,
            }
        }
        (Schema::Number(left), Schema::Number(right)) => {
            match (left.multiple_of.as_ref(), right.multiple_of.as_ref()) {
                (Some(l), Some(r)) => l.checked_lcm(r).is_none(),
                _ => false,
            }
        }
        _ => false,
    }
}

/// True when any pair of branches is an integer or number pair whose intersection bails back to `AllOf` (an
/// unrepresentable combined `multipleOf`).
pub(crate) fn has_unsafe_numeric_pair(branches: &[SharedSchema]) -> bool {
    any_unordered_pair(branches, is_unsafe_numeric_pair)
}

/// True when intersecting some `Const`/`Enum` branch with a typed sibling bails to `AllOf` from an
/// undecidable membership verdict (asserted format, extended regex, constrained array/object body).
pub(crate) fn has_unmergeable_value_set_pair(
    branches: &[SharedSchema],
    ctx: &CanonicalizationContext,
) -> bool {
    any_unordered_pair(branches, |left, right| {
        is_unmergeable_value_set_pair(left, right, ctx)
            || is_unmergeable_value_set_pair(right, left, ctx)
    })
}

fn is_unmergeable_value_set_pair(
    value_set: &SharedSchema,
    other: &SharedSchema,
    ctx: &CanonicalizationContext,
) -> bool {
    if !matches!(value_set.as_schema(), Schema::Const(_) | Schema::Enum(_)) {
        return false;
    }
    match other.as_schema() {
        Schema::AnyOf(members) => members
            .iter()
            .any(|member| is_unmergeable_value_set_pair(value_set, member, ctx)),
        // A composite-bodied `TypedGroup` makes `ty_and_body` bail, so the pair must stay split too.
        Schema::TypedGroup { .. } => {
            !value_set::value_set_merge_is_decidable(other, value_set, ctx)
        }
        _ if value_set::ty_and_body(other).is_some() => {
            !value_set::value_set_merge_is_decidable(other, value_set, ctx)
        }
        _ => false,
    }
}

/// True when any pair of string branches carries differing formats that must remain separate because the
/// intersection cannot represent both in one `StringLeaf`.
pub(crate) fn has_unmergeable_string_format_pair(
    branches: &[SharedSchema],
    ctx: &CanonicalizationContext,
) -> bool {
    any_unordered_pair(branches, |left, right| {
        is_unmergeable_string_pair(left, right, ctx)
    })
}

pub(crate) fn is_unmergeable_string_pair(
    left: &SharedSchema,
    right: &SharedSchema,
    ctx: &CanonicalizationContext,
) -> bool {
    let (Schema::String(left), Schema::String(right)) = (left.as_schema(), right.as_schema())
    else {
        return false;
    };
    is_unmergeable_format_pair(left, right, ctx)
}
