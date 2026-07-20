//! Set algebra over canonical IR nodes.
use referencing::Draft;

use crate::{
    canonical::{
        ir::{CanonicalJson, Schema, SchemaKind},
        parse,
    },
    JsonType, JsonTypeSet,
};

/// The schema accepting exactly the values that BOTH `left` and `right` accept (set intersection, `allOf`).
pub(crate) fn intersect(left: Schema, right: Schema, draft: Draft) -> Schema {
    match (left.into_kind(), right.into_kind()) {
        // `False` accepts no value. If either side is `False`, nothing can satisfy both, so the result is `False`.
        (SchemaKind::False, _) | (_, SchemaKind::False) => Schema::new(SchemaKind::False),
        // `True` accepts every value, so "must satisfy both" collapses to just the other side.
        (SchemaKind::True, right) => Schema::new(right),
        // Same as above with the sides swapped: `True` on the right keeps the left side.
        (left, SchemaKind::True) => Schema::new(left),
        // One side is an `AnyOf` (matches if any branch matches). Push the intersection inside the union:
        // (A or B) and C = (A and C) or (B and C). Intersect each branch with the other side, then re-union.
        // e.g.  allOf [
        //         {"anyOf": [{"const": 1}, {"type": "string"}]},
        //         {"type": "string"}
        //       ]  =>  {"type": "string"}
        (SchemaKind::AnyOf(branches), other) | (other, SchemaKind::AnyOf(branches)) => {
            distribute(branches, Schema::new(other), draft)
        }
        // `Const`/`Enum` is a fixed set of allowed values. Keep only those values the other side also accepts.
        (left @ (SchemaKind::Const(_) | SchemaKind::Enum(_)), right) => {
            restrict_members(into_members(left), Schema::new(right), draft)
        }
        // Same as above with the fixed value set on the right.
        (left, right @ (SchemaKind::Const(_) | SchemaKind::Enum(_))) => {
            restrict_members(into_members(right), Schema::new(left), draft)
        }
        // Each side is a set of allowed JSON types (e.g. string, number). Keep the types allowed by both;
        // `Number` also allows every `Integer`. If they share no type, nothing matches, so `False`.
        // e.g.  allOf [
        //         {"type": ["integer", "string"]},
        //         {"type": ["string", "null"]}
        //       ]  =>  {"type": "string"}
        (SchemaKind::MultiType(first), SchemaKind::MultiType(second)) => {
            let cover =
                SchemaKind::semantic_cover(first).intersect(SchemaKind::semantic_cover(second));
            if cover.is_empty() {
                Schema::new(SchemaKind::False)
            } else {
                parse::type_set_schema(cover)
            }
        }
        // A `TypedGroup` accepts values of one JSON type that also lie in a value set. If the type set
        // includes that type, keep the group unchanged; otherwise they share no value, so `False`.
        // e.g.  Draft 4, allOf [
        //         {"type": "integer", "enum": [1, 2]},
        //         {"type": "string"}
        //       ]  =>  {"not": {}}
        (SchemaKind::MultiType(set), SchemaKind::TypedGroup { ty, body })
        | (SchemaKind::TypedGroup { ty, body }, SchemaKind::MultiType(set)) => {
            if SchemaKind::semantic_cover(set).contains(ty) {
                Schema::new(SchemaKind::TypedGroup { ty, body })
            } else {
                Schema::new(SchemaKind::False)
            }
        }
        // Two `TypedGroup`s can overlap only if they use the same type. Same type: keep it and intersect
        // their value sets. Different types share no value (nothing is two types at once), so `False`.
        // e.g.  Draft 4, allOf [
        //         {"type": "integer", "enum": [1, 2]},
        //         {"type": "integer", "enum": [2, 3]}
        //       ]  =>  {"type": "integer", "enum": [2]}
        (
            SchemaKind::TypedGroup { ty: first, body },
            SchemaKind::TypedGroup {
                ty: second,
                body: other,
            },
        ) => {
            if first == second {
                typed_group(first, intersect(body, other, draft))
            } else {
                Schema::new(SchemaKind::False)
            }
        }
        // `Raw` is an unmodeled schema kept verbatim. It only ever appears as the whole document (parse keeps
        // the entire document `Raw` when it cannot model it), never nested in a combinator, so intersect never sees it.
        (SchemaKind::Raw(_), _) | (_, SchemaKind::Raw(_)) => {
            unreachable!("`Raw` is whole-document; combinators never contain it")
        }
    }
}

/// The schema accepting every value that ANY of the `branches` accepts (set union, `anyOf`), in normal form.
pub(crate) fn union(branches: Vec<Schema>, draft: Draft) -> Schema {
    // Each branch is sorted into an accumulator: allowed JSON types, loose values, and per-type value groups.
    let mut members: Vec<CanonicalJson> = Vec::new();
    let mut types = JsonTypeSet::empty();
    let mut groups: Vec<(JsonType, Vec<CanonicalJson>)> = Vec::new();

    let mut stack = branches;
    while let Some(branch) = stack.pop() {
        match branch.into_kind() {
            // A branch that accepts everything makes the whole union accept everything.
            SchemaKind::True => return Schema::new(SchemaKind::True),
            // A branch that accepts nothing contributes nothing to the union.
            SchemaKind::False => {}
            // A nested union flattens into this one: `anyOf` of `anyOf` is a single `anyOf`.
            SchemaKind::AnyOf(inner) => stack.extend(inner),
            // Collect the JSON types this branch allows.
            SchemaKind::MultiType(set) => {
                types = union_type_sets(types, set);
            }
            // Collect a single allowed value.
            SchemaKind::Const(value) => members.push(value),
            // Collect a finite set of allowed values.
            SchemaKind::Enum(values) => members.extend(values),
            // A `TypedGroup` accepts values of one JSON type that lie in a value set; collect those values
            // into that type's pool.
            SchemaKind::TypedGroup { ty, body } => {
                let values = into_members(body.into_kind());
                match groups.iter_mut().find(|(existing, _)| *existing == ty) {
                    Some((_, pool)) => pool.extend(values),
                    None => groups.push((ty, values)),
                }
            }
            // `Raw` is whole-document and never nested in a combinator, so union never sees it.
            SchemaKind::Raw(_) => {
                unreachable!("`Raw` is whole-document; combinators never contain it")
            }
        }
    }

    let cover = SchemaKind::semantic_cover(types);
    // Once the collected types span every JSON type there is nothing left to exclude: accept everything.
    if cover == JsonTypeSet::all() {
        return Schema::new(SchemaKind::True);
    }

    // A loose value or a group is redundant when the type set already accepts its whole type; drop those.
    // e.g.  anyOf [
    //         {"type": "string"},
    //         {"const": "x"}
    //       ]  =>  {"type": "string"}
    // Draft 4 keeps such a value beside its type, since `1` also matches `1.0` (which `integer` rejects), so
    // anyOf [{"type": "integer"}, {"enum": [1]}] stays whole.
    members.retain(|member| !type_set_absorbs_member(cover, member, draft));
    groups.retain(|(ty, _)| !cover.contains(*ty));

    let value_set = parse::canonicalize_value_set(members);
    // Packing the loose values may fill a whole type's domain (all of `null`/`boolean`), turning them into a
    // type. As a type it can now absorb more values/groups, so fold it back in and re-run the whole pass.
    // e.g.  anyOf [
    //         {"const": null},
    //         {"const": false},
    //         {"const": true}
    //       ]  =>  {"type": ["null", "boolean"]}
    if let SchemaKind::MultiType(saturated) = value_set.kind() {
        let mut rest: Vec<Schema> = vec![Schema::new(SchemaKind::MultiType(union_type_sets(
            types, *saturated,
        )))];
        rest.extend(
            groups
                .into_iter()
                .map(|(ty, pool)| typed_group(ty, parse::canonicalize_value_set(pool))),
        );
        return union(rest, draft);
    }

    // Assemble the surviving branches. The collected types become one branch.
    let mut out: Vec<Schema> = Vec::new();
    if !types.is_empty() {
        out.push(parse::type_set_schema(types));
    }
    // Each per-type group becomes a branch, unless the loose value set already accepts all its values.
    // e.g.  Draft 4, anyOf [
    //         {"type": "integer", "enum": [1]},
    //         {"enum": [1, "a"]}
    //       ]  =>  {"enum": [1, "a"]}
    for (ty, pool) in groups {
        let body = parse::canonicalize_value_set(pool);
        if body.kind().finite_values().is_some() && !value_set_admits_group(&value_set, &body) {
            out.push(typed_group(ty, body));
        }
    }
    // The loose value set becomes a branch, unless it collapsed to empty.
    if !matches!(value_set.kind(), SchemaKind::False) {
        out.push(value_set);
    }

    // Sort and dedup for a stable form, then collapse: no branches accept nothing (`False`), a lone branch
    // needs no `anyOf` wrapper, and two or more stay an `AnyOf`.
    out.sort();
    out.dedup();
    match out.len() {
        0 => Schema::new(SchemaKind::False),
        1 => out.into_iter().next().expect("len == 1"),
        _ => Schema::new(SchemaKind::AnyOf(out)),
    }
}

/// Intersect `other` with each union branch; the last branch moves `other` instead of cloning it.
fn distribute(mut branches: Vec<Schema>, other: Schema, draft: Draft) -> Schema {
    let last = branches.pop().expect("AnyOf carries at least two branches");
    let mut out: Vec<Schema> = branches
        .into_iter()
        .map(|branch| intersect(branch, other.clone(), draft))
        .collect();
    out.push(intersect(last, other, draft));
    union(out, draft)
}

fn into_members(kind: SchemaKind) -> Vec<CanonicalJson> {
    match kind {
        SchemaKind::Const(value) => vec![value],
        SchemaKind::Enum(values) => values,
        other => unreachable!("value-set kind expected: {other:?}"),
    }
}

/// Keep only the `members` that `other` also accepts, packed back into a canonical value set.
fn restrict_members(members: Vec<CanonicalJson>, other: Schema, draft: Draft) -> Schema {
    match other.into_kind() {
        // `other` is itself a value set: keep the members present in both.
        kind @ (SchemaKind::Const(_) | SchemaKind::Enum(_)) => {
            let admitted = into_members(kind);
            parse::canonicalize_value_set(
                members
                    .into_iter()
                    .filter(|member| admitted.binary_search(member).is_ok())
                    .collect(),
            )
        }
        // `other` allows a set of JSON types: keep the members whose type is allowed.
        SchemaKind::MultiType(set) => parse::restrict_values_to_types(members, set, draft),
        // `other` is a typed group: keep the members that match its type AND sit in its value set.
        SchemaKind::TypedGroup { ty, body } => {
            let admitted = into_members(body.into_kind());
            let kept: Vec<_> = members
                .into_iter()
                .filter(|member| member.json_type() == ty && admitted.binary_search(member).is_ok())
                .collect();
            typed_group(ty, parse::canonicalize_value_set(kept))
        }
        // Intersect dispatch already handled `True`/`False`/`AnyOf`/`Raw`, so `other` is a leaf here.
        other => unreachable!("dispatch handles the remaining kinds: {other:?}"),
    }
}

fn typed_group(ty: JsonType, body: Schema) -> Schema {
    if matches!(body.kind(), SchemaKind::False) {
        Schema::new(SchemaKind::False)
    } else {
        Schema::new(SchemaKind::TypedGroup { ty, body })
    }
}

/// Whether the type set already accepts everything `member` does, making `member` redundant beside it.
///
/// Usually true when `member`'s JSON type is in the set. Draft 4 is the one exception: a value is matched
/// by equality, so an integer value also accepts its float spelling `1.0`, but Draft 4's `integer` type
/// rejects `1.0`. The type set then does not fully cover the value, so `member` is kept.
fn type_set_absorbs_member(cover: JsonTypeSet, member: &CanonicalJson, draft: Draft) -> bool {
    let ty = member.json_type();
    if !cover.contains(ty) {
        return false;
    }
    !(matches!(draft, Draft::Draft4)
        && ty == JsonType::Integer
        && !cover.contains(JsonType::Number))
}

/// Whether the plain value set already accepts every value the typed group does, making the group
/// redundant beside it.
///
/// Only this direction holds, never the reverse: a value is matched by equality, so it also accepts the
/// float spelling `1.0`, while the group's type constraint can reject `1.0`. That makes the plain value
/// set the more permissive of the two.
fn value_set_admits_group(value_set: &Schema, body: &Schema) -> bool {
    let (Some(admitted), Some(values)) = (
        value_set.kind().finite_values(),
        body.kind().finite_values(),
    ) else {
        return false;
    };
    values
        .iter()
        .all(|value| admitted.binary_search(value).is_ok())
}

/// Union of two type sets, dropping `Integer` when `Number` is present.
fn union_type_sets(left: JsonTypeSet, right: JsonTypeSet) -> JsonTypeSet {
    SchemaKind::canonical_type_set(left.union(right))
}
