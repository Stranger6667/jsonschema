use std::sync::Arc;

use crate::{
    canonical::{
        const_enum::intern_value_set,
        context::CanonicalizationContext,
        intern::{allof_pair, shared},
        ir::{
            ArrayLeaf, BooleanBounds, IntegerLeaf, NumberLeaf, ObjectLeaf, Schema, SharedSchema,
            StringLeaf,
        },
        leaves::{Intersection, TypedLeaf},
    },
    JsonType, JsonTypeSet,
};

use super::{
    intersect_canonical, intersect_typed_with_value_set, intersect_value_sets,
    normalize_integer_not_multiple_of,
    value_set::{filter_values_through_guard, intersect_multi_type_with_value_set},
};
use crate::canonical::numeric::{
    number_bounds_to_integer, number_multiple_of_to_integer, number_not_multiple_of_to_integer,
};

/// Canonical schema for "the value's JSON type is in `set`".
///
/// Empty -> `False`; full cover -> `True`; singleton -> that type's open leaf; everything-but-`Number` -> the
/// integer-valued-number guard (canonical type-less `multipleOf: 1`); otherwise a `MultiType`.
pub(crate) fn multi_type_or_false(set: JsonTypeSet) -> SharedSchema {
    if set.is_empty() {
        shared(Schema::False)
    } else if Schema::semantic_cover(set).complement().is_empty() {
        // Every JSON value has a type in the set.
        shared(Schema::True)
    } else if set == JsonTypeSet::all().remove(JsonType::Number) {
        shared(Schema::TypeGuard {
            ty: JsonType::Number,
            body: open_typed_leaf(JsonType::Integer),
        })
    } else if set.len() == 1 {
        let only = set.iter().next().expect("len == 1");
        open_typed_leaf(only)
    } else {
        shared(Schema::MultiType(set))
    }
}

pub(crate) fn open_typed_leaf(ty: JsonType) -> SharedSchema {
    match ty {
        JsonType::Null => shared(Schema::Null),
        JsonType::Boolean => shared(Schema::Boolean(BooleanBounds::Any)),
        JsonType::Integer => shared(Schema::Integer(IntegerLeaf::default())),
        JsonType::Number => shared(Schema::Number(NumberLeaf::default())),
        JsonType::String => shared(Schema::String(StringLeaf::default())),
        JsonType::Array => shared(Schema::Array(ArrayLeaf::default())),
        JsonType::Object => shared(Schema::Object(ObjectLeaf::default())),
    }
}

/// Non-canonicalising intersect.
pub(crate) fn intersect_internal(
    left: &SharedSchema,
    right: &SharedSchema,
    ctx: &CanonicalizationContext,
) -> SharedSchema {
    match (left.as_schema(), right.as_schema()) {
        // True is the intersection identity: A ∩ True = A.
        (_, Schema::True) => Arc::clone(left),
        (Schema::True, _) => Arc::clone(right),
        // False is the intersection absorber: A ∩ False = False.
        (Schema::False, _) | (_, Schema::False) => shared(Schema::False),
        (
            Schema::TypeGuard {
                ty: left_ty,
                body: left_body,
            },
            Schema::TypeGuard {
                ty: right_ty,
                body: right_body,
            },
        ) if left_ty == right_ty => shared(Schema::TypeGuard {
            ty: *left_ty,
            body: intersect_canonical(left_body, right_body, ctx),
        }),
        (Schema::TypeGuard { ty, body }, _) => {
            intersect_type_guard_with_schema(*ty, body, right, left, ctx)
        }
        (_, Schema::TypeGuard { ty, body }) => {
            intersect_type_guard_with_schema(*ty, body, left, right, ctx)
        }
        // De Morgan: Not(X) ∩ Not(Y) = Not(X ∪ Y).
        (Schema::Not(left), Schema::Not(right)) => {
            let union = shared(Schema::AnyOf(vec![Arc::clone(left), Arc::clone(right)]));
            shared(Schema::Not(union))
        }
        // `MultiType(a) ∩ MultiType(b)` via subtype-aware set intersection.
        (Schema::MultiType(a), Schema::MultiType(b)) => {
            multi_type_or_false(Schema::type_set_intersect(*a, *b))
        }
        // `MultiType(set) ∩ value-set`: keep the values the type set admits, empty -> `False`.
        (Schema::MultiType(set), Schema::Const(_) | Schema::Enum(_)) => {
            intersect_multi_type_with_value_set(*set, right)
        }
        (Schema::Const(_) | Schema::Enum(_), Schema::MultiType(set)) => {
            intersect_multi_type_with_value_set(*set, left)
        }
        // `MultiType(a) ∩ Not(type-set b) = a - cover(b)` when representable; else `AllOf` so later passes see the residual.
        (Schema::MultiType(a), Schema::Not(other)) | (Schema::Not(other), Schema::MultiType(a))
            if other.as_schema().as_type_set().is_some() =>
        {
            let b = other
                .as_schema()
                .as_type_set()
                .expect("guarded by match condition");
            match Schema::type_set_subtract(*a, b) {
                Some(set) => multi_type_or_false(set),
                None => allof_pair(left, right),
            }
        }
        // `MultiType(set)` against a single-typed leaf: distribute over members, narrowing the leaf to each. The
        // `Number`<->`Integer` narrowing lifts bounds/`multipleOf`, exposing emptiness a deferred `AllOf` would stall on.
        (Schema::MultiType(set), other) | (other, Schema::MultiType(set))
            if other.pinned_kind().is_some() =>
        {
            let node = if matches!(left.as_schema(), Schema::MultiType(_)) {
                right
            } else {
                left
            };
            let view = node
                .as_typed_view()
                .expect("pinned kind implies a typed view");
            let wrap = matches!(node.as_schema(), Schema::TypedGroup { .. });
            let mut branches: Vec<SharedSchema> = Vec::new();
            for member in set {
                let Some(target) = member.intersect(view.ty) else {
                    continue;
                };
                let narrowed = lift_to_ty(target, view.ty, &view.schema);
                if matches!(narrowed.as_schema(), Schema::False) {
                    continue;
                }
                branches.push(typed_group_if(wrap, target, narrowed));
            }
            anyof_or_collapse(branches)
        }
        // Distribute ∩ over ∨ to reach typed-leaf pairs where per-type merging applies.
        (Schema::AnyOf(branches), _) => distribute_over(branches, right, ctx),
        (_, Schema::AnyOf(branches)) => distribute_over(branches, left, ctx),
        _ => intersect_typed_pair(left, right, ctx),
    }
}

fn intersect_type_guard_with_schema(
    guard_ty: JsonType,
    guard_body: &SharedSchema,
    other: &SharedSchema,
    guard: &SharedSchema,
    ctx: &CanonicalizationContext,
) -> SharedSchema {
    if let Schema::MultiType(set) = other.as_schema() {
        return intersect_type_guard_with_multi_type(guard_ty, guard_body, *set);
    }
    // Distribute ∩ over ∨: the guard narrows each branch independently (a deferred `AllOf` would stall the union
    // spelling double negation must return to).
    if let Schema::AnyOf(branches) = other.as_schema() {
        return distribute_over(branches, guard, ctx);
    }
    // Kind-pinned leaves without a typed view (`null`, booleans) outside the guard kind pass it unchanged.
    if let Some(kind) = other.as_schema().pinned_kind() {
        if !guard_ty.overlaps(kind) {
            return Arc::clone(other);
        }
    }
    // A value set filters through the guard: non-`ty` values pass freely, `ty` values keep body membership.
    let values = match other.as_schema() {
        Schema::Const(value) => Some(std::slice::from_ref(value)),
        Schema::Enum(values) => Some(values.as_slice()),
        _ => None,
    };
    if let Some(values) = values {
        return match filter_values_through_guard(guard_ty, guard_body, values, ctx) {
            Some(kept) => intern_value_set(kept),
            None => allof_pair(guard, other),
        };
    }
    let Some(other_view) = other.as_typed_view() else {
        return allof_pair(guard, other);
    };
    // `other_view.ty` is the already-checked `pinned_kind`, so the guard overlaps it. A partial overlap that isn't
    // full coverage keeps both operands for the validator.
    if !guard_ty.covers(other_view.ty) {
        return allof_pair(guard, other);
    }
    let merged_body = intersect_bodies(
        other_view.ty,
        guard_ty,
        guard_body,
        other_view.ty,
        &other_view.schema,
        ctx,
    );
    if matches!(merged_body.as_schema(), Schema::False) {
        return shared(Schema::False);
    }
    typed_group_if(
        matches!(other.as_schema(), Schema::TypedGroup { .. }),
        other_view.ty,
        merged_body,
    )
}

/// Distribute the set's members through the guard.
///
/// Members outside the guarded kind pass free, fully guarded members narrow to the body, a partially guarded member
/// (`number` over an `integer` guard) keeps the guard under its pin.
fn intersect_type_guard_with_multi_type(
    guard_ty: JsonType,
    guard_body: &SharedSchema,
    set: JsonTypeSet,
) -> SharedSchema {
    let mut branches: Vec<SharedSchema> = Vec::new();
    for member in set {
        match member.intersect(guard_ty) {
            // Disjoint kinds: the guard imposes nothing on this member.
            None => branches.push(open_typed_leaf(member)),
            // A guard's kind is never a strict subtype of a member (guards are Number/String/Array/Object, never
            // Integer), so the overlap is always the member, narrowed by the guard body.
            Some(_) => branches.push(lift_to_ty(member, guard_ty, guard_body)),
        }
    }
    anyof_or_collapse(branches)
}

/// Typed-leaf or value-set merge, with `AllOf` as fallback.
fn intersect_typed_pair(
    left: &SharedSchema,
    right: &SharedSchema,
    ctx: &CanonicalizationContext,
) -> SharedSchema {
    if let (Some(left_view), Some(right_view)) = (left.as_typed_view(), right.as_typed_view()) {
        let Some(target_ty) = left_view.ty.intersect(right_view.ty) else {
            return shared(Schema::False);
        };
        let merged_body = intersect_bodies(
            target_ty,
            left_view.ty,
            &left_view.schema,
            right_view.ty,
            &right_view.schema,
            ctx,
        );
        if matches!(merged_body.as_schema(), Schema::False) {
            return shared(Schema::False);
        }
        let wrap_in_typed_group = matches!(left.as_schema(), Schema::TypedGroup { .. })
            || matches!(right.as_schema(), Schema::TypedGroup { .. });
        return typed_group_if(wrap_in_typed_group, target_ty, merged_body);
    }
    if let Some(filtered) = intersect_typed_with_value_set(left, right, ctx) {
        return filtered;
    }
    if let Some(merged) = intersect_value_sets(left.as_schema(), right.as_schema()) {
        return shared(merged);
    }
    allof_pair(left, right)
}

fn intersect_bodies(
    target_ty: JsonType,
    left_ty: JsonType,
    left_body: &SharedSchema,
    right_ty: JsonType,
    right_body: &SharedSchema,
    ctx: &CanonicalizationContext,
) -> SharedSchema {
    let left_lifted = lift_to_ty(target_ty, left_ty, left_body);
    let right_lifted = lift_to_ty(target_ty, right_ty, right_body);
    if matches!(left_lifted.as_schema(), Schema::False)
        || matches!(right_lifted.as_schema(), Schema::False)
    {
        return shared(Schema::False);
    }
    intersect_typed(target_ty, &left_lifted, &right_lifted, ctx)
}

/// Rewrite a Number leaf as the equivalent Integer leaf when the intersection narrows to Integer.
///
/// Fractional bounds round inward (lower up, upper down) and `multipleOf` lifts to its integer equivalent, exposing
/// emptiness an unmerged `AllOf` would stall on.
///
/// ```text
/// BEFORE: {"type": "integer"}  and  {"type": "number", "minimum": 0.5, "maximum": 2.5}
/// AFTER:  {"type": "integer", "minimum": 1, "maximum": 2}   // 0.5 rounds up to 1, 2.5 down to 2
/// ```
fn lift_to_ty(target_ty: JsonType, body_ty: JsonType, body: &SharedSchema) -> SharedSchema {
    // Only a `Number` leaf narrowing to `Integer` rewrites the body. A same-type lift, a widening, or a
    // number-typed view whose body isn't a bare `Number` leaf (a value set, a union) keeps the body as-is.
    let (JsonType::Integer, JsonType::Number, Schema::Number(number)) =
        (target_ty, body_ty, body.as_schema())
    else {
        return Arc::clone(body);
    };
    // `not_multiple_of` holds finite positive moduli, so the integer conversion never fails in practice;
    // the fallback keeps the number leaf rather than guess on an unrepresentable modulus.
    let Some(not_multiple_of) = number
        .not_multiple_of
        .iter()
        .map(number_not_multiple_of_to_integer)
        .collect()
    else {
        return Arc::clone(body);
    };
    let leaf = IntegerLeaf {
        bounds: number_bounds_to_integer(&number.bounds),
        multiple_of: number_multiple_of_to_integer(number.multiple_of.as_ref()),
        not_multiple_of,
    };
    normalize_integer_not_multiple_of(leaf).map_or_else(
        || shared(Schema::False),
        |leaf| shared(Schema::Integer(leaf)),
    )
}

fn distribute_over(
    branches: &[SharedSchema],
    other: &SharedSchema,
    ctx: &CanonicalizationContext,
) -> SharedSchema {
    let combined: Vec<SharedSchema> = branches
        .iter()
        .map(|branch| intersect_internal(branch, other, ctx))
        .collect();
    shared(Schema::AnyOf(combined))
}

/// Finish a leaf intersection outcome: wrap the merged leaf, collapse `Empty` to `False`, or
/// keep both operands as `AllOf` for `Residual`.
fn finish_leaf_intersection<T>(
    intersection: Intersection<T>,
    build: impl FnOnce(T) -> Schema,
    left: &SharedSchema,
    right: &SharedSchema,
) -> SharedSchema {
    match intersection {
        Intersection::Merged(leaf) => shared(build(leaf)),
        Intersection::Empty => shared(Schema::False),
        Intersection::Residual => allof_pair(left, right),
    }
}

/// Merge two same-type canonical leaf nodes; returns `Schema::False` when unsatisfiable.
#[must_use]
pub(crate) fn intersect_typed(
    ty: JsonType,
    left: &SharedSchema,
    right: &SharedSchema,
    ctx: &CanonicalizationContext,
) -> SharedSchema {
    // Null and boolean types normalize to a constant or finite value set before intersection, so they never appear as
    // typed leaves here.
    let merged = match ty {
        JsonType::Integer => intersect_same_leaf::<IntegerLeaf>(left, right, ctx),
        JsonType::Number => intersect_same_leaf::<NumberLeaf>(left, right, ctx),
        JsonType::String => intersect_same_leaf::<StringLeaf>(left, right, ctx),
        JsonType::Array => intersect_same_leaf::<ArrayLeaf>(left, right, ctx),
        JsonType::Object => intersect_same_leaf::<ObjectLeaf>(left, right, ctx),
        JsonType::Null | JsonType::Boolean => None,
    };
    if let Some(merged) = merged {
        return merged;
    }
    match (left.as_schema(), right.as_schema()) {
        // Reduce two value-set bodies via the Const/Enum intersect rules
        (Schema::Const(_) | Schema::Enum(_), Schema::Const(_) | Schema::Enum(_)) => {
            intersect_value_sets(left.as_schema(), right.as_schema())
                .map_or_else(|| allof_pair(left, right), shared)
        }
        // Typed leaf vs value-set: filter values by the typed bounds; `None` falls back to `AllOf` for the validator to
        // check strictly.
        _ => intersect_typed_with_value_set(left, right, ctx)
            .unwrap_or_else(|| allof_pair(left, right)),
    }
}

/// Merge two `L` leaves via the leaf algebra, or `None` when either side is not an `L` (the caller
/// then tries the value-set rules).
fn intersect_same_leaf<L: TypedLeaf>(
    left: &SharedSchema,
    right: &SharedSchema,
    ctx: &CanonicalizationContext,
) -> Option<SharedSchema> {
    let (l, r) = (
        L::project(left.as_schema())?,
        L::project(right.as_schema())?,
    );
    Some(finish_leaf_intersection(
        l.intersect(r, ctx),
        L::wrap,
        left,
        right,
    ))
}

/// Collapse intersection branches: `False` when none survive, the lone branch when one does, else `AnyOf`.
fn anyof_or_collapse(mut branches: Vec<SharedSchema>) -> SharedSchema {
    match branches.len() {
        0 => shared(Schema::False),
        1 => branches.pop().expect("len == 1"),
        _ => shared(Schema::AnyOf(branches)),
    }
}

/// Re-wrap a merged body under the `TypedGroup` pin an operand carried; bare body otherwise.
fn typed_group_if(wrap: bool, ty: JsonType, body: SharedSchema) -> SharedSchema {
    if wrap {
        shared(Schema::TypedGroup { ty, body })
    } else {
        body
    }
}
