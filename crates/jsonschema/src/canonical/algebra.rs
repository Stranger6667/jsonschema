//! Set algebra over canonical IR nodes.
use std::sync::Arc;

use referencing::Draft;
use serde_json::Value;

use crate::{
    canonical::{
        context::{CanonicalizationContext, CompiledMatcher},
        ir::{
            tighter, ArrayLeaf, ArrayLeaves, AtLeastTwo, BoundCardinality, BoundInteger,
            BoundNumber, BoundRational, CanonicalJson, Discrete, Divisors, IntegerBounds,
            IntegerLeaf, IntegerLeaves, LengthBounds, NonEmpty, NumberLeaf, NumberLeaves,
            ObjectLeaf, ObjectLeaves, Round, Schema, SchemaKind, Side, StringLeaf, StringLeaves,
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
        // A string leaf shares no value with a typed group (a non-string type), an integer leaf or
        // a number leaf: nothing is two JSON types at once, so the result is `False`.
        | (
            SchemaKind::TypedGroup { .. } | SchemaKind::Integer(_) | SchemaKind::Number(_),
            SchemaKind::String(_),
        )
        | (
            SchemaKind::String(_),
            SchemaKind::TypedGroup { .. } | SchemaKind::Integer(_) | SchemaKind::Number(_),
        )
        // An array or object leaf shares no value with a leaf of any other type, nor with a typed
        // group (whose type is never `array` or `object`).
        | (
            SchemaKind::Array(_) | SchemaKind::Object(_),
            SchemaKind::String(_)
            | SchemaKind::Integer(_)
            | SchemaKind::Number(_)
            | SchemaKind::TypedGroup { .. },
        )
        | (
            SchemaKind::String(_)
            | SchemaKind::Integer(_)
            | SchemaKind::Number(_)
            | SchemaKind::TypedGroup { .. },
            SchemaKind::Array(_) | SchemaKind::Object(_),
        )
        | (SchemaKind::Array(_), SchemaKind::Object(_))
        | (SchemaKind::Object(_), SchemaKind::Array(_)) => {
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
        // A typed group holds `integer` values (Draft 4), and every integer is a number; keep the
        // ones the interval admits.
        (SchemaKind::TypedGroup { ty, body }, SchemaKind::Number(leaf))
        | (SchemaKind::Number(leaf), SchemaKind::TypedGroup { ty, body }) => {
            let kept = into_members(body.into_kind())
                .into_iter()
                .filter(|member| number_leaf_admits(leaf.get(), member))
                .collect();
            typed_group(ty, parse::canonicalize_value_set(kept))
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
        // A number interval keeps only the values both sides admit.
        (SchemaKind::Number(first), SchemaKind::Number(second)) => {
            number_leaf(
                intersect_number_leaves(first.into_inner(), second.into_inner()),
                ctx,
            )
        }
        // A number interval survives a type set only when the set covers `number`.
        (SchemaKind::MultiType(set), SchemaKind::Number(leaf))
        | (SchemaKind::Number(leaf), SchemaKind::MultiType(set)) => {
            if set.contains(JsonType::Number) {
                number_leaf(leaf.into_inner(), ctx)
            } else if set.contains(JsonType::Integer) {
                // `integer` is a subset of `number`, so the interval keeps its integers.
                integer_within(&leaf.into_inner(), ctx)
            } else {
                Schema::new(SchemaKind::False)
            }
        }
        // An array leaf constrains array values. A type set keeps it only when the set covers
        // `array`; otherwise the two share no value, so `False`.
        (SchemaKind::MultiType(set), SchemaKind::Array(leaf))
        | (SchemaKind::Array(leaf), SchemaKind::MultiType(set)) => {
            if set.contains(JsonType::Array) {
                array_leaf(leaf.into_inner())
            } else {
                Schema::new(SchemaKind::False)
            }
        }
        // Two array leaves: keep the arrays both accept - the narrower window, and distinct items
        // when either side asks for them.
        (SchemaKind::Array(first), SchemaKind::Array(second)) => {
            array_leaf(intersect_array_leaves(
                first.into_inner(),
                second.into_inner(),
            ))
        }
        // An object leaf constrains object values. A type set keeps it only when the set covers
        // `object`; otherwise the two share no value, so `False`.
        (SchemaKind::MultiType(set), SchemaKind::Object(leaf))
        | (SchemaKind::Object(leaf), SchemaKind::MultiType(set)) => {
            if set.contains(JsonType::Object) {
                object_leaf(leaf.into_inner())
            } else {
                Schema::new(SchemaKind::False)
            }
        }
        // Two object leaves: keep the objects both accept - the narrower window, every required key.
        (SchemaKind::Object(first), SchemaKind::Object(second)) => {
            object_leaf(intersect_object_leaves(
                first.into_inner(),
                second.into_inner(),
            ))
        }
        // An integer leaf inside a number interval keeps the integers the interval admits.
        (SchemaKind::Integer(integers), SchemaKind::Number(numbers))
        | (SchemaKind::Number(numbers), SchemaKind::Integer(integers)) => {
            let within = integer_within(&numbers.into_inner(), ctx);
            intersect(Schema::new(SchemaKind::Integer(integers)), within, ctx)
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
    let mut numbers = NumberLeaves::default();
    let mut arrays = ArrayLeaves::default();
    let mut objects = ObjectLeaves::default();

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
            // A number leaf accepts a real interval; collect it with the other number branches.
            SchemaKind::Number(leaf) => numbers.insert(leaf.into_inner()),
            // An array leaf accepts a length window; collect it with the other array branches.
            SchemaKind::Array(leaf) => arrays.insert(leaf.into_inner()),
            // An object leaf accepts a property-count window; collect it with the other object branches.
            SchemaKind::Object(leaf) => objects.insert(leaf.into_inner()),
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
    // A number leaf is redundant once the type set covers `number`.
    if cover.contains(JsonType::Number) {
        numbers.clear();
    }
    // An array leaf is redundant once the type set covers `array`.
    if cover.contains(JsonType::Array) {
        arrays.clear();
    }
    // An object leaf is redundant once the type set covers `object`.
    if cover.contains(JsonType::Object) {
        objects.clear();
    }

    // A single value is a one-value window spelled differently, so move it in beside the windows and
    // let it merge with a neighbour it touches.
    // e.g.  anyOf [
    //         {"type": "integer", "minimum": 6},
    //         {"const": 5}
    //       ]  =>  {"type": "integer", "minimum": 5}
    if !strings.is_empty() || !integers.is_empty() || !arrays.is_empty() || !objects.is_empty() {
        members.retain(|member| {
            !lift_degenerate_member(
                &mut strings,
                &mut integers,
                &mut arrays,
                &mut objects,
                member,
                ctx,
            )
        });
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
    debug_assert!(numbers.is_empty() || !cover.contains(JsonType::Number));
    debug_assert!(arrays.is_empty() || !cover.contains(JsonType::Array));
    debug_assert!(objects.is_empty() || !cover.contains(JsonType::Object));
    let mut widened = types;
    integers.retain(|leaf| {
        let spans_domain = leaf.bounds.is_unbounded() && leaf.multiple_of.is_empty();
        if spans_domain {
            widened = union_type_sets(widened, JsonTypeSet::from(JsonType::Integer));
        }
        !spans_domain
    });
    numbers.retain(|leaf| {
        let spans_domain =
            leaf.minimum.is_none() && leaf.maximum.is_none() && leaf.multiple_of.is_empty();
        if spans_domain {
            widened = union_type_sets(widened, JsonTypeSet::from(JsonType::Number));
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
    arrays.retain(|leaf| {
        let spans_domain = leaf.lengths.is_unbounded() && !leaf.unique;
        if spans_domain {
            widened = union_type_sets(widened, JsonTypeSet::from(JsonType::Array));
        }
        !spans_domain
    });
    objects.retain(|leaf| {
        let spans_domain = leaf.sizes.is_unbounded() && leaf.required.is_empty();
        if spans_domain {
            widened = union_type_sets(widened, JsonTypeSet::from(JsonType::Object));
        }
        !spans_domain
    });
    if widened != types {
        debug_assert!(widened.union(types) == widened, "type set lost a member");
        return rerun(
            widened, members, groups, strings, integers, numbers, arrays, objects, ctx,
        );
    }

    // A value one of the surviving windows already accepts adds nothing beside it.
    // e.g.  anyOf [
    //         {"type": "string", "minLength": 1},
    //         {"const": "abc"}
    //       ]  =>  {"type": "string", "minLength": 1}
    if !members.is_empty()
        && (!strings.is_empty()
            || !integers.is_empty()
            || !numbers.is_empty()
            || !arrays.is_empty()
            || !objects.is_empty())
    {
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
        let intervals = numbers.as_slice();
        let array_leaves = arrays.as_slice();
        let object_leaves = objects.as_slice();
        members.retain(|member| {
            !leaf_absorbs_member(
                &compiled,
                windows,
                intervals,
                array_leaves,
                object_leaves,
                member,
                ctx,
            )
        });
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
        return rerun(
            widened,
            Vec::new(),
            groups,
            strings,
            integers,
            numbers,
            arrays,
            objects,
            ctx,
        );
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
    // Each surviving number leaf becomes its own branch.
    for leaf in numbers {
        out.push(number_leaf(leaf, ctx));
    }
    // Each surviving string leaf becomes its own branch.
    for leaf in strings {
        out.push(string_leaf(leaf, ctx));
    }
    // Each surviving integer leaf becomes its own branch.
    for bounds in integers {
        out.push(integer_leaf(bounds, ctx));
    }
    // Each surviving array leaf becomes its own branch.
    for leaf in arrays {
        out.push(array_leaf(leaf));
    }
    // Each surviving object leaf becomes its own branch.
    for leaf in objects {
        out.push(object_leaf(leaf));
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
    arrays: &mut ArrayLeaves,
    objects: &mut ObjectLeaves,
    member: &CanonicalJson,
    ctx: &CanonicalizationContext,
) -> bool {
    match member.as_value() {
        // `maxItems: 0` accepts the empty array and nothing else, so `{"const": []}` is that window
        // written another way.
        Value::Array(items) if items.is_empty() && !arrays.is_empty() => {
            arrays.insert(ArrayLeaf {
                lengths: LengthBounds {
                    minimum: None,
                    maximum: Some(BoundCardinality::from(0)),
                },
                unique: false,
            });
            true
        }
        // `maxProperties: 0` accepts the empty object and nothing else, so `{"const": {}}` is that
        // window written another way.
        Value::Object(map) if map.is_empty() && !objects.is_empty() => {
            objects.insert(ObjectLeaf {
                sizes: LengthBounds {
                    minimum: None,
                    maximum: Some(BoundCardinality::from(0)),
                },
                required: Vec::new(),
            });
            true
        }
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
                        multiple_of: Divisors::default(),
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
    numbers: NumberLeaves,
    arrays: ArrayLeaves,
    objects: ObjectLeaves,
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
    rest.extend(numbers.into_iter().map(|leaf| number_leaf(leaf, ctx)));
    rest.extend(arrays.into_iter().map(array_leaf));
    rest.extend(objects.into_iter().map(object_leaf));
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
        | SchemaKind::Number(_)
        | SchemaKind::Array(_)
        | SchemaKind::Object(_)
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
        // `other` is a number interval: keep the numeric members it admits.
        SchemaKind::Number(leaf) => {
            let kept = members
                .into_iter()
                .filter(|member| number_leaf_admits(leaf.get(), member))
                .collect();
            parse::canonicalize_value_set(kept)
        }
        // `other` is an array leaf: keep the array members whose length fits its window.
        SchemaKind::Array(leaf) => {
            let kept = members
                .into_iter()
                .filter(|member| array_leaf_admits(leaf.get(), member))
                .collect();
            parse::canonicalize_value_set(kept)
        }
        // `other` is an object leaf: keep the object members whose property count fits its window.
        SchemaKind::Object(leaf) => {
            let kept = members
                .into_iter()
                .filter(|member| object_leaf_admits(leaf.get(), member))
                .collect();
            parse::canonicalize_value_set(kept)
        }
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
    numbers: &[NumberLeaf],
    arrays: &[ArrayLeaf],
    objects: &[ObjectLeaf],
    member: &CanonicalJson,
    ctx: &CanonicalizationContext,
) -> bool {
    match member.as_value() {
        Value::Array(_) => arrays.iter().any(|leaf| array_leaf_admits(leaf, member)),
        Value::Object(_) => objects.iter().any(|leaf| object_leaf_admits(leaf, member)),
        Value::String(_) => strings.iter().any(|(leaf, regexes)| {
            string_leaf_admits(leaf, regexes, member, ctx, UncheckableFormat::Rejects)
        }),
        // A number interval admits `7` and `7.0` alike, so no draft aliases them apart. Draft 4
        // keeps the value beside an `integer` interval, which rejects `7.0`.
        Value::Number(_) => {
            numbers.iter().any(|leaf| number_leaf_admits(leaf, member))
                || (!matches!(ctx.draft(), Draft::Draft4)
                    && integers
                        .iter()
                        .any(|leaf| integer_leaf_admits(leaf, member)))
        }
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
    IntegerLeaf {
        bounds: first.bounds.intersect(second.bounds),
        multiple_of: first.multiple_of.intersect(second.multiple_of),
    }
}

/// A `Number` node, collapsed to `False` when its interval admits no real value and to the value
/// itself when both ends admit the same one. Unlike `integer`, no draft tells `5` and `5.0` apart on
/// the number domain, so the value needs no type guard.
/// e.g.  {"type": "number", "minimum": 5, "maximum": 5}  =>  {"const": 5}
pub(crate) fn number_leaf(leaf: NumberLeaf, ctx: &CanonicalizationContext) -> Schema {
    let leaf = snap_to_progression(leaf);
    // Every draft after 4 counts `2.0` as an integer, so a whole divisor already restricts the leaf
    // to the integers it admits and both spellings denote one set.
    if ctx.draft() != Draft::Draft4
        && leaf
            .multiple_of
            .sole()
            .is_some_and(BoundRational::admits_only_whole)
    {
        // Snapping can move an end past the representable integers, leaving the number leaf as the
        // only form able to carry it.
        if let Some(bounds) = integer_bounds_within(&leaf) {
            return integer_leaf(
                IntegerLeaf {
                    bounds,
                    multiple_of: leaf.multiple_of,
                },
                ctx,
            );
        }
    }
    let Some(leaf) = NonEmpty::new(leaf) else {
        return Schema::new(SchemaKind::False);
    };
    if let (Some(min), Some(max)) = (&leaf.get().minimum, &leaf.get().maximum) {
        if min.is_inclusive() && max.is_inclusive() && min.to_number() == max.to_number() {
            let point = min.to_number();
            return if leaf.get().multiple_of.divide(&point) {
                Schema::new(SchemaKind::Const(CanonicalJson::from_value(
                    &Value::Number(point),
                )))
            } else {
                Schema::new(SchemaKind::False)
            };
        }
    }
    Schema::new(SchemaKind::Number(leaf))
}

/// Pack an array facet set into a node, collapsing the leaves that say something simpler.
pub(crate) fn array_leaf(mut leaf: ArrayLeaf) -> Schema {
    // An array of at most one item has nothing to repeat, so uniqueness says nothing more.
    // e.g.  {"type": "array", "maxItems": 1, "uniqueItems": true}
    //       =>  {"type": "array", "maxItems": 1}
    if leaf
        .lengths
        .maximum
        .as_ref()
        .is_some_and(|max| *max <= BoundCardinality::from(1))
    {
        leaf.unique = false;
    }
    let Some(leaf) = NonEmpty::new(leaf) else {
        return Schema::new(SchemaKind::False);
    };
    // `maxItems: 0` accepts the empty array and nothing else.
    // e.g.  {"type": "array", "maxItems": 0}  =>  {"const": []}
    if leaf
        .get()
        .lengths
        .maximum
        .as_ref()
        .is_some_and(BoundCardinality::is_zero)
    {
        return Schema::new(SchemaKind::Const(CanonicalJson::from_value(&Value::Array(
            Vec::new(),
        ))));
    }
    Schema::new(SchemaKind::Array(leaf))
}

/// Keep the arrays both leaves accept: the narrower window, and distinct items when either asks.
fn intersect_array_leaves(first: ArrayLeaf, second: ArrayLeaf) -> ArrayLeaf {
    ArrayLeaf {
        lengths: first.lengths.intersect(second.lengths),
        unique: first.unique || second.unique,
    }
}

/// Whether `member` is an array whose length sits in the window, with distinct items when asked.
fn array_leaf_admits(leaf: &ArrayLeaf, member: &CanonicalJson) -> bool {
    let Value::Array(items) = member.as_value() else {
        return false;
    };
    if !leaf
        .lengths
        .contains(&BoundCardinality::from(items.len() as u64))
    {
        return false;
    }
    // Members are normalized, so `1` and `1.0` compare equal here just as they do at validation.
    !leaf.unique
        || items
            .iter()
            .enumerate()
            .all(|(index, item)| !items[..index].contains(item))
}

/// Pack an object facet set into a node, collapsing the leaves that say something simpler.
pub(crate) fn object_leaf(mut leaf: ObjectLeaf) -> Schema {
    // A required key already demands a property, so a minimum it covers says nothing more.
    // e.g.  {"type": "object", "required": ["a", "b"], "minProperties": 2}
    //       =>  {"type": "object", "required": ["a", "b"]}

    if leaf
        .sizes
        .minimum
        .as_ref()
        .is_some_and(|min| *min <= leaf.required_count())
    {
        leaf.sizes.minimum = None;
    }
    let Some(leaf) = NonEmpty::new(leaf) else {
        return Schema::new(SchemaKind::False);
    };
    // `maxProperties: 0` accepts the empty object and nothing else; a required key would have
    // emptied the leaf above.
    // e.g.  {"type": "object", "maxProperties": 0}  =>  {"const": {}}
    if leaf
        .get()
        .sizes
        .maximum
        .as_ref()
        .is_some_and(BoundCardinality::is_zero)
    {
        return Schema::new(SchemaKind::Const(CanonicalJson::from_value(
            &Value::Object(serde_json::Map::new()),
        )));
    }
    Schema::new(SchemaKind::Object(leaf))
}

/// Keep the objects both leaves accept: the narrower window, and every key either demands.
fn intersect_object_leaves(first: ObjectLeaf, second: ObjectLeaf) -> ObjectLeaf {
    let mut required = first.required;
    required.extend(second.required);
    required.sort();
    required.dedup();
    ObjectLeaf {
        sizes: first.sizes.intersect(second.sizes),
        required,
    }
}

/// Whether `member` is an object carrying every required key with its property count in the window.
fn object_leaf_admits(leaf: &ObjectLeaf, member: &CanonicalJson) -> bool {
    let Value::Object(map) = member.as_value() else {
        return false;
    };
    leaf.sizes
        .contains(&BoundCardinality::from(map.len() as u64))
        && leaf.required.iter().all(|key| map.contains_key(&**key))
}

/// The number leaf admitting exactly the values both admit.
fn intersect_number_leaves(first: NumberLeaf, second: NumberLeaf) -> NumberLeaf {
    NumberLeaf {
        minimum: tightest(first.minimum, second.minimum, Side::Lower),
        maximum: tightest(first.maximum, second.maximum, Side::Upper),
        // Meeting both sets of divisors is meeting their union.
        multiple_of: first.multiple_of.intersect(second.multiple_of),
    }
}

/// Pull each end onto the progression, so an interval and its divisor have one spelling. Only a
/// lone divisor gives a progression to snap to; an end no decimal spells is left as it is.
/// e.g.  {"type": "number", "minimum": 1, "maximum": 4, "multipleOf": 1.5}
///         =>  {"type": "number", "minimum": 1.5, "maximum": 3, "multipleOf": 1.5}
fn snap_to_progression(leaf: NumberLeaf) -> NumberLeaf {
    let Some(step) = leaf.multiple_of.sole() else {
        return leaf;
    };
    let snap = |bound: Option<BoundNumber>, direction: Round| match bound {
        Some(bound) => step.multiple_beyond(&bound, direction).or(Some(bound)),
        None => None,
    };
    NumberLeaf {
        minimum: snap(leaf.minimum, Round::Up),
        maximum: snap(leaf.maximum, Round::Down),
        multiple_of: leaf.multiple_of,
    }
}

/// The bound admitting the fewer values on `side`.
fn tightest(
    first: Option<BoundNumber>,
    second: Option<BoundNumber>,
    side: Side,
) -> Option<BoundNumber> {
    tighter(first, second, |left, right| {
        if left.is_tighter_than(&right, side) {
            left
        } else {
            right
        }
    })
}

/// The integers a number interval admits. Endpoints are whole here, so an excluded one steps by one.
fn integer_within(leaf: &NumberLeaf, ctx: &CanonicalizationContext) -> Schema {
    let bounds = integer_bounds_within(leaf)
        .expect("interval bounds hold representable integers, checked during parsing");
    integer_leaf(
        IntegerLeaf {
            bounds,
            multiple_of: leaf.multiple_of.clone(),
        },
        ctx,
    )
}

/// The integers a number interval admits, or `None` when its ends leave the representable range.
pub(crate) fn integer_bounds_within(leaf: &NumberLeaf) -> Option<IntegerBounds> {
    // A fractional end rounds inward to the first integer the interval holds; a whole end is that
    // integer already, unless excluded, in which case it steps one further in.
    let step = |bound: &BoundNumber,
                direction: Round,
                inward: &dyn Fn(BoundInteger) -> Option<BoundInteger>| {
        let limit = bound.to_number();
        let rounded = BoundInteger::round_from_number(&limit, direction)?;
        if bound.is_inclusive() || BoundInteger::from_number(&limit).is_none() {
            Some(rounded)
        } else {
            inward(rounded)
        }
    };
    // Past the representable range there is no integer left to admit.
    let minimum = match &leaf.minimum {
        Some(bound) => Some(step(bound, Round::Up, &|value: BoundInteger| {
            value.checked_increment()
        })?),
        None => None,
    };
    let maximum = match &leaf.maximum {
        Some(bound) => Some(step(bound, Round::Down, &BoundInteger::checked_decrement)?),
        None => None,
    };
    Some(IntegerBounds { minimum, maximum })
}

/// Whether `member` is a number the interval admits.
fn number_leaf_admits(leaf: &NumberLeaf, member: &CanonicalJson) -> bool {
    let Value::Number(number) = member.as_value() else {
        return false;
    };
    leaf.minimum
        .as_ref()
        .is_none_or(|min| min.admits(number, Side::Lower))
        && leaf
            .maximum
            .as_ref()
            .is_none_or(|max| max.admits(number, Side::Upper))
        && leaf.multiple_of.divide(number)
}

/// An `Integer` node, collapsed to `False` when its interval is empty and to the value itself when the
/// interval holds exactly one. Draft 4 keeps the integer guard on that value, where `5.0` is not `5`.
pub(crate) fn integer_leaf(leaf: IntegerLeaf, ctx: &CanonicalizationContext) -> Schema {
    let leaf = IntegerLeaf {
        multiple_of: leaf.multiple_of.over_integers(),
        ..leaf
    };
    let Some(leaf) = snap_to_multiples(leaf).and_then(NonEmpty::new) else {
        return Schema::new(SchemaKind::False);
    };
    if let (Some(min), Some(max)) = (&leaf.get().bounds.minimum, &leaf.get().bounds.maximum) {
        if min == max {
            let point = min.to_number();
            // Only a divisor snapping could not pull onto the progression is left to check here.
            if !leaf.get().multiple_of.divide(&point) {
                return Schema::new(SchemaKind::False);
            }
            let value = Schema::new(SchemaKind::Const(CanonicalJson::from_value(
                &Value::Number(point),
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
    // Snapping is exact integer arithmetic, which only a lone whole divisor the validator reads the
    // same way justifies.
    let Some(step) = leaf
        .multiple_of
        .sole()
        .and_then(BoundRational::exact_integer)
    else {
        return Some(leaf);
    };
    // A bound whose next multiple is past the representable range still admits the multiples beyond
    // it, so the end stays where it is.
    let minimum = leaf
        .bounds
        .minimum
        .as_ref()
        .map(|min| step.multiple_beyond(min, Round::Up).unwrap_or(min.clone()));
    let maximum = leaf.bounds.maximum.as_ref().map(|max| {
        step.multiple_beyond(max, Round::Down)
            .unwrap_or(max.clone())
    });
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
        Some(value) => leaf.bounds.contains(&value) && leaf.multiple_of.divide(number),
        // A value past the representable range still gets a divisor verdict from the validator's
        // own arithmetic.
        None => admits_out_of_range(&leaf.bounds, number) && leaf.multiple_of.divide(number),
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
