//! Set algebra over canonical IR nodes.
use std::sync::Arc;

use referencing::Draft;
use serde_json::Value;

use crate::{
    canonical::{
        context::{CanonicalizationContext, CompiledMatcher},
        ir::{
            AtLeastTwo, BoundCardinality, BoundInteger, CanonicalJson, IntegerBounds, IntegerLeaf,
            IntegerLeaves, LengthBounds, NonEmpty, Round, Schema, SchemaKind, StringLeaf,
            StringLeaves,
        },
        parse,
    },
    JsonType, JsonTypeSet,
};

/// The schema accepting exactly the values that BOTH `left` and `right` accept (set intersection, `allOf`).
pub(crate) fn intersect(left: Schema, right: Schema, ctx: &CanonicalizationContext) -> Schema {
    match (left.into_kind(), right.into_kind()) {
        // `False` accepts no value, so nothing satisfies both sides.
        (SchemaKind::False, _)
        | (_, SchemaKind::False)
        // A string leaf shares no value with a typed group (a non-string type) or an integer leaf:
        // nothing is two JSON types at once, so the result is `False`.
        | (SchemaKind::TypedGroup { .. } | SchemaKind::Integer(_), SchemaKind::String(_))
        | (SchemaKind::String(_), SchemaKind::TypedGroup { .. } | SchemaKind::Integer(_)) => {
            Schema::new(SchemaKind::False)
        }
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
            distribute(branches, Schema::new(other), ctx)
        }
        // `Const`/`Enum` is a fixed set of allowed values. Keep only those values the other side also accepts.
        (left @ (SchemaKind::Const(_) | SchemaKind::Enum(_)), right) => {
            restrict_members(into_members(left), Schema::new(right), ctx)
        }
        // Same as above with the fixed value set on the right.
        (left, right @ (SchemaKind::Const(_) | SchemaKind::Enum(_))) => {
            restrict_members(into_members(right), Schema::new(left), ctx)
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
                typed_group(first, intersect(body, other, ctx))
            } else {
                Schema::new(SchemaKind::False)
            }
        }
        // A string leaf constrains string values. A type set keeps it only when the set covers `string`;
        // otherwise the two share no value, so `False`.
        (SchemaKind::MultiType(set), SchemaKind::String(leaf))
        | (SchemaKind::String(leaf), SchemaKind::MultiType(set)) => {
            if SchemaKind::semantic_cover(set).contains(JsonType::String) {
                string_leaf(leaf.into_inner(), ctx)
            } else {
                Schema::new(SchemaKind::False)
            }
        }
        // Two string leaves: keep the strings both accept by tightening to the narrower length window.
        (SchemaKind::String(first), SchemaKind::String(second)) => {
            string_leaf(
                intersect_string_leaves(first.into_inner(), second.into_inner()),
                ctx,
            )
        }
        // An integer leaf constrains integer values. A type set keeps it only when the set covers
        // `integer`; otherwise the two share no value, so `False`.
        (SchemaKind::MultiType(set), SchemaKind::Integer(bounds))
        | (SchemaKind::Integer(bounds), SchemaKind::MultiType(set)) => {
            if SchemaKind::semantic_cover(set).contains(JsonType::Integer) {
                integer_leaf(bounds.into_inner(), ctx)
            } else {
                Schema::new(SchemaKind::False)
            }
        }
        // Two integer leaves: keep the integers both accept by tightening to the narrower interval.
        (SchemaKind::Integer(first), SchemaKind::Integer(second)) => {
            integer_leaf(
                intersect_integer_leaves(first.into_inner(), second.into_inner()),
                ctx,
            )
        }
        // A typed group holds `integer` values (Draft 4); keep the ones within the leaf's interval.
        (SchemaKind::TypedGroup { ty, body }, SchemaKind::Integer(leaf))
        | (SchemaKind::Integer(leaf), SchemaKind::TypedGroup { ty, body }) => {
            let kept = into_members(body.into_kind())
                .into_iter()
                .filter(|member| integer_leaf_admits(leaf.get(), member))
                .collect();
            typed_group(ty, parse::canonicalize_value_set(kept))
        }
        // `Raw` is an unmodeled schema kept verbatim. It only ever appears as the whole document (parse keeps
        // the entire document `Raw` when it cannot model it), never nested in a combinator, so intersect never sees it.
        (SchemaKind::Raw(_), _) | (_, SchemaKind::Raw(_)) => {
            unreachable!("`Raw` is whole-document; combinators never contain it")
        }
    }
}

/// The schema accepting every value that ANY of the `branches` accepts (set union, `anyOf`), in normal form.
pub(crate) fn union(branches: Vec<Schema>, ctx: &CanonicalizationContext) -> Schema {
    // Every branch is sorted into one of these: the JSON types any branch allows, loose values, the
    // values each `TypedGroup` allows for its type, and the string/integer branches kept as windows.
    let mut members: Vec<CanonicalJson> = Vec::new();
    let mut types = JsonTypeSet::empty();
    let mut groups: Vec<(JsonType, Vec<CanonicalJson>)> = Vec::new();
    let mut strings = StringLeaves::default();
    let mut integers = IntegerLeaves::default();

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
            // A `TypedGroup` accepts values of one JSON type that lie in a value set; collect those
            // values under that type.
            SchemaKind::TypedGroup { ty, body } => {
                let values = into_members(body.into_kind());
                match groups.iter_mut().find(|(existing, _)| *existing == ty) {
                    Some((_, collected)) => collected.extend(values),
                    None => groups.push((ty, values)),
                }
            }
            // A string leaf accepts a length window; collect it with the other string branches.
            SchemaKind::String(leaf) => strings.insert(leaf.into_inner()),
            // An integer leaf accepts an interval; collect it with the other integer branches.
            SchemaKind::Integer(leaf) => integers.insert(leaf.into_inner()),
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
    members.retain(|member| !type_set_absorbs_member(cover, member, ctx.draft()));
    groups.retain(|(ty, _)| !cover.contains(*ty));
    // Any string matches the `string` type, so a string leaf is redundant once the type set covers it.
    if cover.contains(JsonType::String) {
        strings.clear();
    }
    // Likewise an integer leaf is redundant once the type set covers `integer`.
    if cover.contains(JsonType::Integer) {
        integers.clear();
    }

    // A single value is a one-value window spelled differently, so move it in beside the windows and
    // let it merge with a neighbour it touches.
    // e.g.  anyOf [
    //         {"type": "integer", "minimum": 6},
    //         {"const": 5}
    //       ]  =>  {"type": "integer", "minimum": 5}
    if !strings.is_empty() || !integers.is_empty() {
        members.retain(|member| !lift_degenerate_member(&mut strings, &mut integers, member, ctx));
    }

    // A Draft 4 `integer` group and an `integer` interval both reject `7.0`, so an interval holding
    // every value of the group makes it redundant.
    // e.g.  Draft 4, anyOf [
    //         {"type": "integer", "minimum": 2},
    //         {"type": "integer", "enum": [7]}
    //       ]  =>  {"type": "integer", "minimum": 2}
    // A loose `{"enum": [7]}` is not redundant the same way: it also matches `7.0`, which the interval
    // rejects, so anyOf [{"type": "integer", "minimum": 2}, {"enum": [7]}] stays whole.
    if !integers.is_empty() {
        let windows = integers.as_slice();
        groups.retain(|(ty, values)| {
            *ty != JsonType::Integer
                || !values
                    .iter()
                    .all(|member| windows.iter().any(|leaf| integer_leaf_admits(leaf, member)))
        });
    }

    // A window left unbounded on both sides - and, for a string, carrying no pattern - accepts every
    // value of its type, so it *is* that type. Fold it into the type set and re-run, which lets the
    // wider set absorb further branches.
    // e.g.  anyOf [
    //         {"type": "integer", "maximum": 0},
    //         {"type": "integer", "minimum": 1}
    //       ]  =>  {"type": "integer"}
    // Windows of a type the set already covers were cleared above, so widening here always adds a
    // bit. Were one to survive, it would be dropped without widening - a branch lost silently.
    debug_assert!(integers.is_empty() || !cover.contains(JsonType::Integer));
    debug_assert!(strings.is_empty() || !cover.contains(JsonType::String));
    let mut widened = types;
    integers.retain(|leaf| {
        let spans_domain = leaf.bounds.is_unbounded() && leaf.multiple_of.is_none();
        if spans_domain {
            widened = union_type_sets(widened, JsonTypeSet::from(JsonType::Integer));
        }
        !spans_domain
    });
    strings.retain(|leaf| {
        let spans_domain =
            leaf.lengths.is_unbounded() && leaf.patterns.is_empty() && leaf.formats.is_empty();
        if spans_domain {
            widened = union_type_sets(widened, JsonTypeSet::from(JsonType::String));
        }
        !spans_domain
    });
    if widened != types {
        debug_assert!(widened.union(types) == widened, "type set lost a member");
        return rerun(widened, members, groups, strings, integers, ctx);
    }

    // A value one of the surviving windows already accepts adds nothing beside it.
    // e.g.  anyOf [
    //         {"type": "string", "minLength": 1},
    //         {"const": "abc"}
    //       ]  =>  {"type": "string", "minLength": 1}
    if !members.is_empty() && (!strings.is_empty() || !integers.is_empty()) {
        let compiled: Vec<(&StringLeaf, Vec<Arc<CompiledMatcher>>)> = strings
            .as_slice()
            .iter()
            .map(|leaf| {
                let regexes = leaf
                    .patterns
                    .iter()
                    .map(|pattern| {
                        ctx.compile_regex(pattern)
                            .expect("pattern validated during parsing")
                    })
                    .collect();
                (leaf, regexes)
            })
            .collect();
        let windows = integers.as_slice();
        members.retain(|member| !leaf_absorbs_member(&compiled, windows, member, ctx));
    }

    let value_set = parse::canonicalize_value_set(members);
    // Packing the loose values may fill a whole type's domain (all of `null`/`boolean`), turning them into a
    // type. As a type it can now absorb more values/groups, so fold it back in and re-run the whole pass.
    // e.g.  anyOf [
    //         {"const": null},
    //         {"const": false},
    //         {"const": true}
    //       ]  =>  {"type": ["null", "boolean"]}
    if let SchemaKind::MultiType(saturated) = value_set.kind() {
        let widened = union_type_sets(types, *saturated);
        debug_assert!(widened.union(types) == widened, "type set lost a member");
        debug_assert!(widened != types, "re-run without a wider type set");
        return rerun(widened, Vec::new(), groups, strings, integers, ctx);
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
    for (ty, values) in groups {
        let body = parse::canonicalize_value_set(values);
        if body.kind().finite_values().is_some() && !value_set_admits_group(&value_set, &body) {
            out.push(typed_group(ty, body));
        }
    }
    // Each surviving string leaf becomes its own branch.
    for leaf in strings {
        out.push(string_leaf(leaf, ctx));
    }
    // Each surviving integer leaf becomes its own branch.
    for bounds in integers {
        out.push(integer_leaf(bounds, ctx));
    }
    // The loose value set becomes a branch, unless it collapsed to empty.
    if !matches!(value_set.kind(), SchemaKind::False) {
        out.push(value_set);
    }

    // Zero branches accept nothing, so the union is `False`; one branch needs no `anyOf` wrapper.
    match AtLeastTwo::new(out) {
        Ok(branches) => {
            // `intersect` dispatches on the assumption that a branch is none of these.
            debug_assert!(
                branches.as_slice().iter().all(|branch| !matches!(
                    branch.kind(),
                    SchemaKind::True | SchemaKind::False | SchemaKind::AnyOf(_)
                )),
                "union branch is not in normal form"
            );
            Schema::new(SchemaKind::AnyOf(branches))
        }
        Err(mut lone) => match lone.pop() {
            Some(only) => only,
            None => Schema::new(SchemaKind::False),
        },
    }
}

/// Move a value in beside the windows of its own type when a one-value window says the same thing.
/// Returns `true` when it moved, so the caller drops it from the loose values.
// The arms are guarded on what has been collected and on the draft, so they cannot be enumerated.
#[allow(clippy::wildcard_enum_match_arm)]
fn lift_degenerate_member(
    strings: &mut StringLeaves,
    integers: &mut IntegerLeaves,
    member: &CanonicalJson,
    ctx: &CanonicalizationContext,
) -> bool {
    match member.as_value() {
        // `maxLength: 0` accepts the empty string and nothing else, so `{"const": ""}` is that
        // window written another way.
        Value::String(text) if text.is_empty() && !strings.is_empty() => {
            strings.insert(StringLeaf {
                lengths: LengthBounds {
                    minimum: None,
                    maximum: Some(BoundCardinality::from(0)),
                },
                patterns: Vec::new(),
                formats: Vec::new(),
            });
            true
        }
        // Outside Draft 4 the value and the window accept the same instances. Draft 4 keeps the value
        // where it is: `7` there also matches `7.0`, which an `integer` window rejects.
        Value::Number(number) if !integers.is_empty() && !matches!(ctx.draft(), Draft::Draft4) => {
            match BoundInteger::from_number(number) {
                Some(bound) => {
                    integers.insert(IntegerLeaf {
                        bounds: IntegerBounds {
                            minimum: Some(bound.clone()),
                            maximum: Some(bound),
                        },
                        multiple_of: None,
                    });
                    true
                }
                None => false,
            }
        }
        _ => false,
    }
}

/// Re-run `union` with a wider type set: everything collected so far goes back in, so nothing is
/// dropped. `types` grows strictly on every re-run and holds at most one bit per JSON type, which
/// bounds the recursion; the callers assert that growth.
fn rerun(
    types: JsonTypeSet,
    members: Vec<CanonicalJson>,
    groups: Vec<(JsonType, Vec<CanonicalJson>)>,
    strings: StringLeaves,
    integers: IntegerLeaves,
    ctx: &CanonicalizationContext,
) -> Schema {
    let mut rest: Vec<Schema> = vec![Schema::new(SchemaKind::MultiType(types))];
    rest.push(parse::canonicalize_value_set(members));
    rest.extend(
        groups
            .into_iter()
            .map(|(ty, values)| typed_group(ty, parse::canonicalize_value_set(values))),
    );
    rest.extend(strings.into_iter().map(|leaf| string_leaf(leaf, ctx)));
    rest.extend(integers.into_iter().map(|leaf| integer_leaf(leaf, ctx)));
    union(rest, ctx)
}

/// Intersect `other` with each union branch; the last branch moves `other` instead of cloning it.
fn distribute(
    branches: AtLeastTwo<Schema>,
    other: Schema,
    ctx: &CanonicalizationContext,
) -> Schema {
    let (rest, last) = branches.split_last();
    let mut out: Vec<Schema> = rest
        .into_iter()
        .map(|branch| intersect(branch, other.clone(), ctx))
        .collect();
    out.push(intersect(last, other, ctx));
    union(out, ctx)
}

fn into_members(kind: SchemaKind) -> Vec<CanonicalJson> {
    match kind {
        SchemaKind::Const(value) => vec![value],
        SchemaKind::Enum(values) => values.into_vec(),
        other @ (SchemaKind::MultiType(_)
        | SchemaKind::TypedGroup { .. }
        | SchemaKind::String(_)
        | SchemaKind::Integer(_)
        | SchemaKind::AnyOf(_)
        | SchemaKind::True
        | SchemaKind::False
        | SchemaKind::Raw(_)) => unreachable!("value-set kind expected: {other:?}"),
    }
}

/// Keep only the `members` that `other` also accepts, packed back into a canonical value set.
fn restrict_members(
    members: Vec<CanonicalJson>,
    other: Schema,
    ctx: &CanonicalizationContext,
) -> Schema {
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
        SchemaKind::MultiType(set) => parse::restrict_values_to_types(members, set, ctx),
        // `other` is a string leaf: keep the members that fit its window and match every pattern.
        SchemaKind::String(leaf) => {
            let regexes: Vec<_> = leaf
                .get()
                .patterns
                .iter()
                .map(|pattern| {
                    ctx.compile_regex(pattern)
                        .expect("pattern validated during parsing")
                })
                .collect();
            let kept = members
                .into_iter()
                .filter(|member| {
                    string_leaf_admits(leaf.get(), &regexes, member, ctx, UncheckableFormat::Admits)
                })
                .collect();
            parse::canonicalize_value_set(kept)
        }
        // `other` is an integer leaf: keep the integer members within its interval. Draft 4 keeps the
        // integer type guard so `1.0` cannot match `1` through value equality.
        SchemaKind::Integer(leaf) => {
            let kept = members
                .into_iter()
                .filter(|member| integer_leaf_admits(leaf.get(), member))
                .collect();
            let value_set = parse::canonicalize_value_set(kept);
            if matches!(ctx.draft(), Draft::Draft4) {
                typed_group(JsonType::Integer, value_set)
            } else {
                value_set
            }
        }
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
        other @ (SchemaKind::True
        | SchemaKind::False
        | SchemaKind::AnyOf(_)
        | SchemaKind::Raw(_)) => unreachable!("dispatch handles the remaining kinds: {other:?}"),
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

/// Whether a surviving window already accepts `member`; only a window of its own JSON type can.
// The arms are guarded on the draft, so they cannot be enumerated.
#[allow(clippy::wildcard_enum_match_arm)]
fn leaf_absorbs_member(
    strings: &[(&StringLeaf, Vec<Arc<CompiledMatcher>>)],
    integers: &[IntegerLeaf],
    member: &CanonicalJson,
    ctx: &CanonicalizationContext,
) -> bool {
    match member.as_value() {
        Value::String(_) => strings.iter().any(|(leaf, regexes)| {
            string_leaf_admits(leaf, regexes, member, ctx, UncheckableFormat::Rejects)
        }),
        // Draft 4 keeps the value: `7` there also matches `7.0`, which an `integer` interval rejects.
        Value::Number(_) if !matches!(ctx.draft(), Draft::Draft4) => integers
            .iter()
            .any(|leaf| integer_leaf_admits(leaf, member)),
        _ => false,
    }
}

/// Union of two type sets, dropping `Integer` when `Number` is present.
fn union_type_sets(left: JsonTypeSet, right: JsonTypeSet) -> JsonTypeSet {
    SchemaKind::canonical_type_set(left.union(right))
}

/// A `String` node, collapsed to `False` when its length window is empty.
pub(crate) fn string_leaf(leaf: StringLeaf, ctx: &CanonicalizationContext) -> Schema {
    if formats_conflict(&leaf, ctx) {
        return Schema::new(SchemaKind::False);
    }
    let Some(leaf) = NonEmpty::new(leaf) else {
        return Schema::new(SchemaKind::False);
    };
    // `maxLength: 0` accepts the empty string and nothing else.
    // e.g.  {"type": "string", "maxLength": 0}  =>  {"const": ""}
    if leaf.get().patterns.is_empty()
        && leaf.get().formats.is_empty()
        && leaf
            .get()
            .lengths
            .maximum
            .as_ref()
            .is_some_and(BoundCardinality::is_zero)
    {
        return Schema::new(SchemaKind::Const(CanonicalJson::from_value(
            &Value::String(String::new()),
        )));
    }
    Schema::new(SchemaKind::String(leaf))
}

/// Tighten two integer leaves to the values both admit: the narrower interval and a divisor every
/// value of each must share. `None` when the least common multiple leaves the representable range,
/// which keeps the document unmodeled rather than guessing.
fn intersect_integer_leaves(first: IntegerLeaf, second: IntegerLeaf) -> IntegerLeaf {
    let bounds = first.bounds.intersect(second.bounds);
    match (first.multiple_of, second.multiple_of) {
        (Some(left), Some(right)) => {
            if let Some(step) = left.checked_lcm(&right) {
                IntegerLeaf {
                    bounds,
                    multiple_of: Some(step),
                }
            } else {
                // Every common multiple but zero is past the representable range, so zero is the
                // only value left to admit.
                let zero = BoundInteger::zero();
                IntegerLeaf {
                    bounds: bounds.intersect(IntegerBounds {
                        minimum: Some(zero.clone()),
                        maximum: Some(zero),
                    }),
                    multiple_of: None,
                }
            }
        }
        (Some(step), None) | (None, Some(step)) => IntegerLeaf {
            bounds,
            multiple_of: Some(step),
        },
        (None, None) => IntegerLeaf {
            bounds,
            multiple_of: None,
        },
    }
}

/// An `Integer` node, collapsed to `False` when its interval is empty and to the value itself when the
/// interval holds exactly one. Draft 4 keeps the integer guard on that value, where `5.0` is not `5`.
pub(crate) fn integer_leaf(leaf: IntegerLeaf, ctx: &CanonicalizationContext) -> Schema {
    let Some(leaf) = snap_to_multiples(leaf).and_then(NonEmpty::new) else {
        return Schema::new(SchemaKind::False);
    };
    if let (Some(min), Some(max)) = (&leaf.get().bounds.minimum, &leaf.get().bounds.maximum) {
        if min == max {
            let value = Schema::new(SchemaKind::Const(CanonicalJson::from_value(
                &Value::Number(min.to_number()),
            )));
            return if matches!(ctx.draft(), Draft::Draft4) {
                typed_group(JsonType::Integer, value)
            } else {
                value
            };
        }
    }
    Schema::new(SchemaKind::Integer(leaf))
}

/// Pull each present bound onto the progression, so an interval and its divisor have one spelling.
/// e.g.  {"type": "integer", "minimum": 4, "maximum": 6, "multipleOf": 5}
///         =>  {"const": 5}      (the interval holds exactly one multiple)
/// `None` when the interval holds no multiple at all, which the caller collapses to `false`.
fn snap_to_multiples(leaf: IntegerLeaf) -> Option<IntegerLeaf> {
    let Some(step) = &leaf.multiple_of else {
        return Some(leaf);
    };
    // A bound with no multiple beyond it is past the representable range, leaving nothing to admit.
    let minimum = match leaf.bounds.minimum.as_ref() {
        Some(min) => Some(step.multiple_beyond(min, Round::Up)?),
        None => None,
    };
    let maximum = match leaf.bounds.maximum.as_ref() {
        Some(max) => Some(step.multiple_beyond(max, Round::Down)?),
        None => None,
    };
    Some(IntegerLeaf {
        bounds: IntegerBounds { minimum, maximum },
        multiple_of: leaf.multiple_of,
    })
}

/// Whether `member` is an integer value within `bounds`.
fn integer_leaf_admits(leaf: &IntegerLeaf, member: &CanonicalJson) -> bool {
    let Value::Number(number) = member.as_value() else {
        return false;
    };
    match BoundInteger::from_number(number) {
        Some(value) => {
            leaf.bounds.contains(&value)
                && leaf
                    .multiple_of
                    .as_ref()
                    .is_none_or(|step| step.divides(&value))
        }
        // A value past the representable range has no divisor to test, so a leaf carrying one
        // cannot be shown to admit it.
        None => leaf.multiple_of.is_none() && admits_out_of_range(&leaf.bounds, number),
    }
}

/// Admittance for an integer `number` that [`BoundInteger::from_number`] cannot hold. In the default
/// build it lies beyond one end of the `i64` range: above every representable maximum, below every
/// representable minimum. A non-integer is never admitted.
#[cfg(not(feature = "arbitrary-precision"))]
fn admits_out_of_range(bounds: &IntegerBounds, number: &serde_json::Number) -> bool {
    if !jsonschema_value::types::number_is_integer(number) {
        return false;
    }
    if number.as_f64().is_some_and(|float| float > 0.0) {
        bounds.maximum.is_none()
    } else {
        bounds.minimum.is_none()
    }
}

// Arbitrary precision holds every integer, so `from_number` only returns `None` for a non-integer.
#[cfg(feature = "arbitrary-precision")]
fn admits_out_of_range(_bounds: &IntegerBounds, _number: &serde_json::Number) -> bool {
    false
}

/// Tighten two string leaves to the strings both accept: the narrower length window and every
/// pattern and format from both.
fn intersect_string_leaves(first: StringLeaf, second: StringLeaf) -> StringLeaf {
    let mut patterns = first.patterns;
    patterns.extend(second.patterns);
    patterns.sort();
    patterns.dedup();
    let mut formats = first.formats;
    formats.extend(second.formats);
    formats.sort();
    formats.dedup();
    StringLeaf {
        lengths: first.lengths.intersect(second.lengths),
        patterns,
        formats,
    }
}

/// Whether the leaf's formats and length window leave no string. A format whose grammar pins a
/// length narrows the window; two such formats of different lengths admit nothing.
/// e.g.  allOf [
///         {"type": "string", "format": "date"},
///         {"type": "string", "format": "uuid"}
///       ]  =>  false
fn formats_conflict(leaf: &StringLeaf, ctx: &CanonicalizationContext) -> bool {
    let mut window = leaf.lengths.clone();
    for format in &leaf.formats {
        let Some((minimum, maximum)) = crate::keywords::format::length_window(ctx.draft(), format)
        else {
            continue;
        };
        window = window.intersect(LengthBounds {
            minimum: Some(BoundCardinality::from(minimum)),
            maximum: Some(BoundCardinality::from(maximum)),
        });
    }
    window.is_empty()
}

/// Whether the string `member` falls within the leaf's length window and matches every pattern.
/// The verdict for a format the draft cannot check. Callers differ: dropping a member narrows the
/// schema, so intersection assumes the value fits; absorbing one narrows it too, so union assumes it
/// does not.
#[derive(Clone, Copy)]
pub(crate) enum UncheckableFormat {
    Admits,
    Rejects,
}

fn string_leaf_admits(
    leaf: &StringLeaf,
    regexes: &[Arc<CompiledMatcher>],
    member: &CanonicalJson,
    ctx: &CanonicalizationContext,
    uncheckable: UncheckableFormat,
) -> bool {
    let Value::String(text) = member.as_value() else {
        return false;
    };
    let length = BoundCardinality::from(bytecount::num_chars(text.as_bytes()) as u64);
    leaf.lengths.contains(&length)
        && regexes.iter().all(|regex| regex.is_match(text))
        && leaf.formats.iter().all(|format| {
            crate::keywords::format::is_valid(ctx.draft(), format, text)
                .unwrap_or(matches!(uncheckable, UncheckableFormat::Admits))
        })
}
