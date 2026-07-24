//! Set algebra over canonical IR nodes.
use std::{collections::BTreeMap, sync::Arc};

use referencing::Draft;
use serde_json::Value;

use crate::{
    canonical::{
        context::{CanonicalizationContext, CompiledMatcher},
        ir::{
            canonicalize_value_set, tighter, type_set_schema, typed_group, ArrayLeaf, ArrayLeaves,
            AtLeastTwo, BoundCardinality, BoundInteger, BoundNumber, BoundRational, CanonicalJson,
            ContainsFacet, Discrete, Divisors, IntegerBounds, IntegerLeaf, IntegerLeaves,
            LengthBounds, NonEmpty, NumberLeaf, NumberLeaves, ObjectLeaf, ObjectLeaves, Round,
            Schema, SchemaKind, Side, StringLeaf, StringLeaves, Verdict,
        },
        negate, parse,
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
                type_set_schema(cover)
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
            typed_group(ty, canonicalize_value_set(kept))
        }
        // A typed group holds `integer` values (Draft 4); keep the ones within the leaf's interval.
        (SchemaKind::TypedGroup { ty, body }, SchemaKind::Integer(leaf))
        | (SchemaKind::Integer(leaf), SchemaKind::TypedGroup { ty, body }) => {
            let kept = into_members(body.into_kind())
                .into_iter()
                .filter(|member| integer_leaf_admits(leaf.get(), member))
                .collect();
            typed_group(ty, canonicalize_value_set(kept))
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
                ctx,
            ))
        }
        // An object leaf constrains object values. A type set keeps it only when the set covers
        // `object`; otherwise the two share no value, so `False`.
        (SchemaKind::MultiType(set), SchemaKind::Object(leaf))
        | (SchemaKind::Object(leaf), SchemaKind::MultiType(set)) => {
            if set.contains(JsonType::Object) {
                object_leaf(leaf.into_inner(), ctx)
            } else {
                Schema::new(SchemaKind::False)
            }
        }
        // Two object leaves: keep the objects both accept - the narrower window, every required key.
        (SchemaKind::Object(first), SchemaKind::Object(second)) => {
            object_leaf(
                intersect_object_leaves(first.into_inner(), second.into_inner(), ctx),
                ctx,
            )
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
    if !strings.is_empty()
        || !integers.is_empty()
        || !numbers.is_empty()
        || !arrays.is_empty()
        || !objects.is_empty()
    {
        members.retain(|member| {
            !lift_degenerate_member(
                &mut strings,
                &mut integers,
                &mut numbers,
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
    // Folding object leaves can produce a leaf spanning the whole domain even though its inputs
    // did not, so the folds run before the widening below picks such leaves up. Merging and
    // narrowing feed each other; each pass shrinks the leaf count or the requirement count, which
    // bounds the loop.
    let mut objects: Vec<ObjectLeaf> = objects.into_iter().collect();
    loop {
        merge_sole_differing_keys(&mut objects, ctx);
        if drop_object_branch_covered_by_sibling(&mut objects, ctx) {
            continue;
        }
        if drop_required_covered_by_sibling(&mut objects, ctx) {
            continue;
        }
        if drop_size_bound_covered_by_sibling(&mut objects, ctx) {
            continue;
        }
        if !widen_entry_covered_by_sibling(&mut objects, ctx) {
            break;
        }
    }
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
        let spans_domain = leaf.spans_domain();
        if spans_domain {
            widened = union_type_sets(widened, JsonTypeSet::from(JsonType::Array));
        }
        !spans_domain
    });
    objects.retain(|leaf| {
        let spans_domain = leaf.spans_domain();
        if spans_domain {
            widened = union_type_sets(widened, JsonTypeSet::from(JsonType::Object));
        }
        !spans_domain
    });
    if widened != types {
        // Widening canonicalizes as it grows: adding `number` beside an existing `integer` drops the
        // narrower bit, so containment holds on the semantic covers, not the raw bitsets.
        debug_assert!(
            SchemaKind::semantic_cover(widened).union(SchemaKind::semantic_cover(types))
                == SchemaKind::semantic_cover(widened),
            "type set lost a member"
        );
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

    let value_set = canonicalize_value_set(members);
    // Packing the loose values may fill a whole type's domain (all of `null`/`boolean`), turning them into a
    // type. As a type it can now absorb more values/groups, so fold it back in and re-run the whole pass.
    // e.g.  anyOf [
    //         {"const": null},
    //         {"const": false},
    //         {"const": true}
    //       ]  =>  {"type": ["null", "boolean"]}
    if let SchemaKind::MultiType(saturated) = value_set.kind() {
        let widened = union_type_sets(types, *saturated);
        debug_assert!(
            SchemaKind::semantic_cover(widened).union(SchemaKind::semantic_cover(types))
                == SchemaKind::semantic_cover(widened),
            "type set lost a member"
        );
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

    // Members saturating a whole finite domain join another type branch: `null` beside `string`
    // is the two-type list, not a loose value, and both booleans together are the `boolean` type.
    // Unsaturated members stay loose, and a lone value set keeps its `const`/`enum` spelling.
    // e.g.  anyOf [
    //         {"type": "number"},
    //         {"enum": [null, false]}
    //       ]  =>  anyOf: [{"type": ["null", "number"]}, {"enum": [false]}]
    if !types.is_empty() {
        if let Some(members) = value_set.kind().finite_values() {
            let mut saturated = JsonTypeSet::empty();
            if members.iter().any(|member| member.as_value().is_null()) {
                saturated = saturated.insert(JsonType::Null);
            }
            let holds = |wanted: bool| {
                members
                    .iter()
                    .any(|member| matches!(member.as_value(), Value::Bool(held) if *held == wanted))
            };
            if holds(false) && holds(true) {
                saturated = saturated.insert(JsonType::Boolean);
            }
            let widened = union_type_sets(types, saturated);
            if widened != types {
                let remaining: Vec<CanonicalJson> = members
                    .iter()
                    .filter(|member| match member.as_value() {
                        Value::Null => !saturated.contains(JsonType::Null),
                        Value::Bool(_) => !saturated.contains(JsonType::Boolean),
                        Value::Number(_)
                        | Value::String(_)
                        | Value::Array(_)
                        | Value::Object(_) => true,
                    })
                    .cloned()
                    .collect();
                return rerun(
                    widened, remaining, groups, strings, integers, numbers, arrays, objects, ctx,
                );
            }
        }
    }

    // Types with finite domains beside loose values dissolve into them: the values then spell the
    // whole branch one way. Only `null` and `boolean` have finite domains, and a surviving member
    // lies outside both, so the expanded set can never saturate back into a type list.
    // e.g.  anyOf [
    //         {"type": ["null", "boolean"]},
    //         {"const": 0}
    //       ]  =>  {"enum": [null, false, true, 0]}
    let finite_domains = JsonType::Null | JsonType::Boolean;
    let (types, value_set) = match value_set.kind().finite_values() {
        Some(members) if !types.is_empty() && finite_domains.union(types) == finite_domains => {
            let mut expanded = members.to_vec();
            if types.contains(JsonType::Null) {
                expanded.push(CanonicalJson::from_value(&Value::Null));
            }
            if types.contains(JsonType::Boolean) {
                expanded.push(CanonicalJson::from_value(&Value::Bool(false)));
                expanded.push(CanonicalJson::from_value(&Value::Bool(true)));
            }
            let dissolved = canonicalize_value_set(expanded);
            debug_assert!(
                dissolved.kind().finite_values().is_some(),
                "a dissolved type list saturated back into types"
            );
            (JsonTypeSet::empty(), dissolved)
        }
        _ => (types, value_set),
    };

    // Assemble the surviving branches. The collected types become one branch.
    let mut out: Vec<Schema> = Vec::new();
    if !types.is_empty() {
        out.push(type_set_schema(types));
    }
    // Each per-type group becomes a branch, unless the loose value set already accepts all its values.
    // e.g.  Draft 4, anyOf [
    //         {"type": "integer", "enum": [1]},
    //         {"enum": [1, "a"]}
    //       ]  =>  {"enum": [1, "a"]}
    for (ty, values) in groups {
        let body = canonicalize_value_set(values);
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
        debug_assert!(
            !leaf.spans_domain(),
            "a leaf spanning the object domain joins the type set before assembly"
        );
        out.push(object_leaf(leaf, ctx));
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
    numbers: &mut NumberLeaves,
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
                prefix: Vec::new(),
                items: None,
                contains: Vec::new(),
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
                property_names: None,
                properties: BTreeMap::new(),
                pattern_properties: BTreeMap::new(),
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
        Value::Number(number)
            if !integers.is_empty()
                && !matches!(ctx.draft(), Draft::Draft4)
                && BoundInteger::from_number(number).is_some() =>
        {
            let bound = BoundInteger::from_number(number).expect("checked in the guard");
            integers.insert(IntegerLeaf {
                bounds: IntegerBounds {
                    minimum: Some(bound.clone()),
                    maximum: Some(bound),
                },
                multiple_of: Divisors::default(),
            });
            true
        }
        // A number window admits every spelling of its values, so the one-value window says the
        // same thing in every draft; the pool fuses it with a window it touches. A window bound
        // can hold less precision than the value, so only a window collapsing back to the same
        // constant carries it.
        // e.g.  anyOf [
        //         {"type": "number", "exclusiveMinimum": 0},
        //         {"const": 0}
        //       ]  =>  {"type": "number", "minimum": 0}
        Value::Number(number) if !numbers.is_empty() => {
            let bound = BoundNumber::new(number, true);
            let window = NumberLeaf {
                minimum: Some(bound.clone()),
                maximum: Some(bound),
                multiple_of: Divisors::default(),
            };
            let collapses_back = matches!(
                number_leaf(window.clone(), ctx).kind(),
                SchemaKind::Const(point) if point.as_value() == &Value::Number(number.clone())
            );
            if collapses_back {
                numbers.insert(window);
            }
            collapses_back
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
    objects: Vec<ObjectLeaf>,
    ctx: &CanonicalizationContext,
) -> Schema {
    let mut rest: Vec<Schema> = vec![Schema::new(SchemaKind::MultiType(types))];
    rest.push(canonicalize_value_set(members));
    rest.extend(
        groups
            .into_iter()
            .map(|(ty, values)| typed_group(ty, canonicalize_value_set(values))),
    );
    rest.extend(strings.into_iter().map(|leaf| string_leaf(leaf, ctx)));
    rest.extend(integers.into_iter().map(|leaf| integer_leaf(leaf, ctx)));
    rest.extend(numbers.into_iter().map(|leaf| number_leaf(leaf, ctx)));
    rest.extend(arrays.into_iter().map(array_leaf));
    rest.extend(objects.into_iter().map(|leaf| object_leaf(leaf, ctx)));
    union(rest, ctx)
}

/// Fold leaves alike in every facet but one key's demands by uniting those demands: the key stays
/// required only when both sides demand it, and a held value satisfying either side's entry
/// satisfies the union of the entries, a missing entry admitting anything. Each fold removes a
/// leaf, so the loop is bounded.
/// ```text
/// e.g.  anyOf [
///         {"type": "object", "properties": {"a": {"type": "null"}}},
///         {"type": "object", "properties": {"a": {"type": "string"}}}
///       ]  =>  {"type": "object", "properties": {"a": {"type": ["null", "string"]}}}
/// e.g.  anyOf [
///         {"type": "object", "properties": {"a": {"type": "string"}}},
///         {"type": "object", "required": ["a"]}
///       ]  =>  {"type": "object"}
/// ```
fn merge_sole_differing_keys(leaves: &mut Vec<ObjectLeaf>, ctx: &CanonicalizationContext) {
    let mut folded = true;
    while folded {
        folded = false;
        'search: for first in 0..leaves.len() {
            for second in first + 1..leaves.len() {
                if let Some(merged) = united_sole_key(&leaves[first], &leaves[second], ctx) {
                    leaves[first] = merged;
                    leaves.remove(second);
                    folded = true;
                    break 'search;
                }
            }
        }
    }
}

/// The one leaf `left` and `right` spell together, when a single key's demands tell them apart.
fn united_sole_key(
    left: &ObjectLeaf,
    right: &ObjectLeaf,
    ctx: &CanonicalizationContext,
) -> Option<ObjectLeaf> {
    if left.sizes != right.sizes
        || left.property_names != right.property_names
        || left.pattern_properties != right.pattern_properties
    {
        return None;
    }
    let key = sole_differing_key(left, right)?;
    let required = if left.required.contains(&key) {
        if right.required.contains(&key) {
            left.required.clone()
        } else {
            right.required.clone()
        }
    } else {
        left.required.clone()
    };
    let united_entry = match (left.properties.get(&key), right.properties.get(&key)) {
        (Some(first), Some(second)) => {
            let schema = if first == second {
                first.clone()
            } else {
                union(vec![first.clone(), second.clone()], ctx)
            };
            if matches!(schema.kind(), SchemaKind::True) {
                None
            } else {
                Some(schema)
            }
        }
        // A side without an entry admits anything at the key, so the union does too.
        _ => None,
    };
    let mut properties = left.properties.clone();
    properties.remove(&key);
    if let Some(schema) = united_entry {
        properties.insert(Arc::clone(&key), schema);
    }
    Some(ObjectLeaf {
        sizes: left.sizes.clone(),
        required,
        property_names: left.property_names.clone(),
        properties,
        pattern_properties: left.pattern_properties.clone(),
    })
}

/// The single key whose required status or property entry separates the two leaves.
fn sole_differing_key(left: &ObjectLeaf, right: &ObjectLeaf) -> Option<Arc<str>> {
    let mut differing: Vec<Arc<str>> = Vec::new();
    let note = |key: &Arc<str>, differing: &mut Vec<Arc<str>>| {
        if !differing.iter().any(|seen| seen == key) {
            differing.push(Arc::clone(key));
        }
    };
    for key in &left.required {
        if !right.required.contains(key) {
            note(key, &mut differing);
        }
    }
    for key in &right.required {
        if !left.required.contains(key) {
            note(key, &mut differing);
        }
    }
    for (key, schema) in &left.properties {
        if right.properties.get(key) != Some(schema) {
            note(key, &mut differing);
        }
    }
    for (key, schema) in &right.properties {
        if left.properties.get(key) != Some(schema) {
            note(key, &mut differing);
        }
    }
    match differing.as_slice() {
        [_] => differing.pop(),
        _ => None,
    }
}

/// Drop a branch a sibling admits wholly: intersecting with the sibling then changes nothing.
/// The widening folds can leave one branch inside another, which the pairwise pool subsumption
/// does not see.
/// ```text
/// e.g.  anyOf [
///         {"type": "object", "properties": {"a": {"type": "null"}}},
///         {"type": "object", "minProperties": 2, "properties": {"a": {"type": "null"}}}
///       ]  =>  {"type": "object", "properties": {"a": {"type": "null"}}}
/// ```
fn drop_object_branch_covered_by_sibling(
    leaves: &mut Vec<ObjectLeaf>,
    ctx: &CanonicalizationContext,
) -> bool {
    for index in 0..leaves.len() {
        let branch = object_leaf(leaves[index].clone(), ctx);
        let covered = (0..leaves.len())
            .filter(|&sibling| sibling != index)
            .any(|sibling| {
                intersect(
                    branch.clone(),
                    object_leaf(leaves[sibling].clone(), ctx),
                    ctx,
                ) == branch
            });
        if covered {
            leaves.remove(index);
            return true;
        }
    }
    // A sibling's size window also splits a branch: the parts inside the window and on the rays
    // outside it partition the branch, and each part must fit some sibling on its own.
    // e.g.  anyOf [
    //         {"type": "object", "required": ["a"], "properties": {"a": {"type": "string"}}},
    //         {"type": "object", "maxProperties": 1, "required": ["a"]},
    //         {"type": "object", "minProperties": 2, "properties": {"a": {"type": "string"}}}
    //       ]  =>  the first branch dissolves: at one key the entry says nothing beside the
    //              filled slots, above that the third branch holds it
    for index in 0..leaves.len() {
        for divider in (0..leaves.len()).filter(|&divider| divider != index) {
            let Some(mut windows) = negate::length_windows(&leaves[divider].sizes) else {
                continue;
            };
            if windows.is_empty() {
                continue;
            }
            windows.push(leaves[divider].sizes.clone());
            let all_covered = windows.iter().all(|window| {
                let mut piece = leaves[index].clone();
                piece.sizes = LengthBounds {
                    minimum: tighter(piece.sizes.minimum.take(), window.minimum.clone(), Ord::max),
                    maximum: tighter(piece.sizes.maximum.take(), window.maximum.clone(), Ord::min),
                };
                let piece = object_leaf(piece, ctx);
                matches!(piece.kind(), SchemaKind::False)
                    || (0..leaves.len())
                        .filter(|&sibling| sibling != index)
                        .any(|sibling| {
                            intersect(
                                piece.clone(),
                                object_leaf(leaves[sibling].clone(), ctx),
                                ctx,
                            ) == piece
                        })
            });
            if all_covered {
                leaves.remove(index);
                return true;
            }
        }
    }
    // A key's presence splits a branch the same way: the part holding the key and the part
    // missing it are both plain leaves, and each must fit a sibling on its own.
    // e.g.  anyOf [
    //         {"type": "object", "required": ["a", "b"]},
    //         {"type": "object", "minProperties": 3, "properties": {"a": false}},
    //         {"type": "object", "minProperties": 3, "required": ["b"]}
    //       ]  =>  the third branch dissolves: with `a` it fits the first, without `a` the second
    for index in 0..leaves.len() {
        let mut keys: Vec<Arc<str>> = leaves
            .iter()
            .enumerate()
            .filter(|(sibling, _)| *sibling != index)
            .flat_map(|(_, leaf)| {
                leaf.required
                    .iter()
                    .chain(leaf.properties.keys())
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .collect();
        keys.sort();
        keys.dedup();
        for key in keys {
            let piece_covered = |piece: ObjectLeaf| {
                let piece = object_leaf(piece, ctx);
                matches!(piece.kind(), SchemaKind::False)
                    || (0..leaves.len())
                        .filter(|&sibling| sibling != index)
                        .any(|sibling| {
                            intersect(
                                piece.clone(),
                                object_leaf(leaves[sibling].clone(), ctx),
                                ctx,
                            ) == piece
                        })
            };
            let mut holding = leaves[index].clone();
            if let Err(position) = holding.required.binary_search(&key) {
                holding.required.insert(position, Arc::clone(&key));
            }
            let mut missing = leaves[index].clone();
            missing
                .properties
                .insert(Arc::clone(&key), Schema::new(SchemaKind::False));
            if piece_covered(holding) && piece_covered(missing) {
                leaves.remove(index);
                return true;
            }
        }
    }
    false
}

/// Drop a required key when the objects its absence would admit - those meeting the rest of the
/// leaf while missing the key - are covered by a sibling branch. That gained set is the leaf with
/// the key un-required and its entry pinned to `False`; a sibling covers it when intersecting
/// changes nothing. The bare drop goes first; when it admits too much, the floor the required
/// count implied is kept explicit and only the key demand is given up. One weakening per call, so
/// the caller re-merges before the next.
/// ```text
/// e.g.  anyOf [
///         {"type": "object", "properties": {"a": {"type": "string"}}},
///         {"type": "object", "required": ["a", "b"]}
///       ]  =>  anyOf [
///         {"type": "object", "properties": {"a": {"type": "string"}}},
///         {"type": "object", "required": ["b"]}
///       ]
/// e.g.  anyOf [
///         {"type": "object", "required": ["a", "b"]},
///         {"type": "object", "minProperties": 2, "properties": {"a": false}}
///       ]  =>  anyOf [
///         {"type": "object", "minProperties": 2, "properties": {"a": false}},
///         {"type": "object", "minProperties": 2, "required": ["b"]}
///       ]
/// e.g.  anyOf [
///         {"type": "object", "required": ["a", "b"]},
///         {"type": "object", "properties": {"a": {"type": "string"}}, "required": ["c"]}
///       ]  =>  unchanged: an object missing `a` and `c` while holding `b` fits neither branch
/// ```
fn drop_required_covered_by_sibling(
    leaves: &mut [ObjectLeaf],
    ctx: &CanonicalizationContext,
) -> bool {
    for index in 0..leaves.len() {
        for key_index in 0..leaves[index].required.len() {
            let implied_floor = BoundCardinality::from(leaves[index].required.len() as u64);
            for keep_floor in [false, true] {
                // An explicit minimum survives the bare drop, so the fallback adds nothing.
                if keep_floor && leaves[index].sizes.minimum.is_some() {
                    break;
                }
                let leaf = &leaves[index];
                let key = Arc::clone(&leaf.required[key_index]);
                let mut weakened = leaf.clone();
                weakened.required.remove(key_index);
                if keep_floor {
                    weakened.sizes.minimum = Some(implied_floor.clone());
                }
                let mut gained = weakened.clone();
                gained
                    .properties
                    .insert(Arc::clone(&key), Schema::new(SchemaKind::False));
                let gained = object_leaf(gained, ctx);
                // An empty gained set means the two spellings tie, and the constructor's
                // required spelling stays; rewriting here would depend on the route taken.
                if matches!(gained.kind(), SchemaKind::False) {
                    continue;
                }
                let covered =
                    (0..leaves.len())
                        .filter(|&sibling| sibling != index)
                        .any(|sibling| {
                            intersect(
                                gained.clone(),
                                object_leaf(leaves[sibling].clone(), ctx),
                                ctx,
                            ) == gained
                        });
                if covered {
                    leaves[index] = weakened;
                    return true;
                }
            }
        }
    }
    false
}

/// Drop a size bound when the slice of counts it excludes - the leaf clipped to the other side of
/// the bound - is covered by a sibling branch. An empty slice is a spelling tie left to the
/// constructor, as with the required drops. One weakening per call.
/// ```text
/// e.g.  anyOf [
///         {"type": "object", "properties": {"a": false}},
///         {"type": "object", "minProperties": 2, "required": ["b"]}
///       ]  =>  anyOf [
///         {"type": "object", "properties": {"a": false}},
///         {"type": "object", "required": ["b"]}
///       ]
/// ```
fn drop_size_bound_covered_by_sibling(
    leaves: &mut [ObjectLeaf],
    ctx: &CanonicalizationContext,
) -> bool {
    for index in 0..leaves.len() {
        let slice_covered = |slice: ObjectLeaf, leaves: &[ObjectLeaf]| {
            let slice = object_leaf(slice, ctx);
            !matches!(slice.kind(), SchemaKind::False)
                && (0..leaves.len())
                    .filter(|&sibling| sibling != index)
                    .any(|sibling| {
                        intersect(
                            slice.clone(),
                            object_leaf(leaves[sibling].clone(), ctx),
                            ctx,
                        ) == slice
                    })
        };
        if let Some(below_ceiling) = leaves[index]
            .sizes
            .minimum
            .as_ref()
            .and_then(|minimum| minimum.clone().checked_decrement())
        {
            let mut slice = leaves[index].clone();
            slice.sizes.minimum = None;
            slice.sizes.maximum = Some(below_ceiling);
            if slice_covered(slice, leaves) {
                leaves[index].sizes.minimum = None;
                return true;
            }
        }
        if let Some(above_floor) = leaves[index]
            .sizes
            .maximum
            .as_ref()
            .and_then(|maximum| maximum.clone().checked_increment())
        {
            let mut slice = leaves[index].clone();
            slice.sizes.minimum = Some(above_floor.clone());
            slice.sizes.maximum = None;
            if slice_covered(slice, leaves) {
                leaves[index].sizes.maximum = None;
                return true;
            }
            // A ceiling filled by the required keys makes every other entry vacuous on this leaf,
            // so the leaf may adopt a sibling's entries for free and shed the ceiling when that
            // sibling holds the slice above it.
            let slots_filled =
                leaves[index].sizes.maximum.as_ref() == Some(&leaves[index].required_count());
            if !slots_filled {
                continue;
            }
            for sibling in (0..leaves.len()).filter(|&sibling| sibling != index) {
                let mut enriched = leaves[index].clone();
                enriched.sizes.maximum = None;
                for (key, entry) in &leaves[sibling].properties {
                    if enriched.required.binary_search(key).is_err() {
                        enriched
                            .properties
                            .entry(Arc::clone(key))
                            .or_insert_with(|| entry.clone());
                    }
                }
                let mut slice = enriched.clone();
                slice.sizes.minimum = Some(above_floor.clone());
                let slice = object_leaf(slice, ctx);
                let held = matches!(slice.kind(), SchemaKind::False)
                    || intersect(
                        slice.clone(),
                        object_leaf(leaves[sibling].clone(), ctx),
                        ctx,
                    ) == slice;
                if held {
                    leaves[index] = enriched;
                    return true;
                }
            }
        }
    }
    false
}

/// Widen a property entry by the union with a sibling's entry at the same key when the sibling
/// covers the difference, so intersection images and direct spellings of one union agree. The
/// objects the widening admits all hold the key with a value the sibling's entry accepts, so the
/// check needs no complement: the widened leaf with the key required under the sibling's entry
/// must sit inside the sibling. A union with the sibling entry lifted to `True` drops the entry.
/// Widening is monotone over the finite entry lattice, so the loop is bounded.
/// ```text
/// e.g.  anyOf [
///         {"type": "object", "properties": {"a": {"type": "string"}}},
///         {"type": "object", "minProperties": 2, "properties": {"a": {"type": "null"}}}
///       ]  =>  anyOf [
///         {"type": "object", "properties": {"a": {"type": "string"}}},
///         {"type": "object", "minProperties": 2, "properties": {"a": {"type": ["null", "string"]}}}
///       ]
/// ```
fn widen_entry_covered_by_sibling(
    leaves: &mut [ObjectLeaf],
    ctx: &CanonicalizationContext,
) -> bool {
    for index in 0..leaves.len() {
        let keys: Vec<Arc<str>> = leaves[index].properties.keys().cloned().collect();
        for key in keys {
            for sibling in (0..leaves.len()).filter(|&sibling| sibling != index) {
                let entry = &leaves[index].properties[&key];
                let sibling_entry = leaves[sibling].properties.get(&key);
                // `None` spells the sibling admitting anything at the key, lifting the union to `True`.
                let widened_entry = match sibling_entry {
                    Some(other) if other == entry => continue,
                    Some(other) => {
                        let united = union(vec![entry.clone(), other.clone()], ctx);
                        if &united == entry {
                            continue;
                        }
                        Some(united).filter(|united| !matches!(united.kind(), SchemaKind::True))
                    }
                    None => None,
                };
                let mut widened = leaves[index].clone();
                match widened_entry {
                    Some(united) => {
                        widened.properties.insert(Arc::clone(&key), united);
                    }
                    None => {
                        widened.properties.remove(&key);
                    }
                }
                if widened == leaves[index] {
                    continue;
                }
                let mut gained = widened.clone();
                match leaves[sibling].properties.get(&key) {
                    Some(other) => {
                        gained.properties.insert(Arc::clone(&key), other.clone());
                    }
                    None => {
                        gained.properties.remove(&key);
                    }
                }
                if let Err(position) = gained.required.binary_search(&key) {
                    gained.required.insert(position, Arc::clone(&key));
                }
                let gained = object_leaf(gained, ctx);
                let covered = matches!(gained.kind(), SchemaKind::False)
                    || intersect(
                        gained.clone(),
                        object_leaf(leaves[sibling].clone(), ctx),
                        ctx,
                    ) == gained;
                if covered {
                    leaves[index] = widened;
                    return true;
                }
            }
        }
    }
    false
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
            canonicalize_value_set(
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
                // Dropping a member narrows the schema, so only a definite rejection drops one.
                .filter(|member| {
                    !matches!(
                        string_leaf_admits(leaf.get(), &regexes, member, ctx),
                        Verdict::Rejects
                    )
                })
                .collect();
            canonicalize_value_set(kept)
        }
        // `other` is an integer leaf: keep the integer members within its interval. Draft 4 keeps the
        // integer type guard so `1.0` cannot match `1` through value equality.
        SchemaKind::Integer(leaf) => {
            let kept = members
                .into_iter()
                .filter(|member| integer_leaf_admits(leaf.get(), member))
                .collect();
            let value_set = canonicalize_value_set(kept);
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
            typed_group(ty, canonicalize_value_set(kept))
        }
        // Intersect dispatch already handled `True`/`False`/`AnyOf`/`Raw`, so `other` is a leaf here.
        // `other` is a number interval: keep the numeric members it admits.
        SchemaKind::Number(leaf) => {
            let kept = members
                .into_iter()
                .filter(|member| number_leaf_admits(leaf.get(), member))
                .collect();
            canonicalize_value_set(kept)
        }
        // `other` is an array leaf: keep the array members whose length fits its window.
        SchemaKind::Array(leaf) => {
            let kept = members
                .into_iter()
                // Dropping a member narrows the schema, so only a definite rejection drops one.
                .filter(|member| {
                    !matches!(array_leaf_admits(leaf.get(), member, ctx), Verdict::Rejects)
                })
                .collect();
            canonicalize_value_set(kept)
        }
        // `other` is an object leaf: keep the object members it fully admits, and pin a member a
        // property schema only partially admits to the admitted part of its equality class.
        SchemaKind::Object(leaf) => {
            let mut kept = Vec::new();
            let mut partial = Vec::new();
            for member in members {
                match restrict_object_member(leaf.get(), &member, ctx) {
                    MemberRestriction::Full => kept.push(member),
                    MemberRestriction::Empty => {}
                    MemberRestriction::Partial(schema) => partial.push(schema),
                }
            }
            let mut branches = vec![canonicalize_value_set(kept)];
            branches.extend(partial);
            union(branches, ctx)
        }
        other @ (SchemaKind::True
        | SchemaKind::False
        | SchemaKind::AnyOf(_)
        | SchemaKind::Raw(_)) => unreachable!("dispatch handles the remaining kinds: {other:?}"),
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
        // Absorbing a member narrows the schema, so only a definite admission absorbs one.
        Value::Array(_) => arrays
            .iter()
            .any(|leaf| matches!(array_leaf_admits(leaf, member, ctx), Verdict::Admits)),
        Value::Object(_) => objects
            .iter()
            .any(|leaf| matches!(object_leaf_admits(leaf, member, ctx), Verdict::Admits)),
        Value::String(_) => strings.iter().any(|(leaf, regexes)| {
            matches!(
                string_leaf_admits(leaf, regexes, member, ctx),
                Verdict::Admits
            )
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
    if !normalize_contains(&mut leaf) {
        return Schema::new(SchemaKind::False);
    }
    normalize_items(&mut leaf);
    if !reconcile_contains_window(&mut leaf) {
        return Schema::new(SchemaKind::False);
    }
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

/// Fold the `contains` demands into canonical form: merge the windows of one schema, turn a
/// demand every element meets into a length bound, and drop the vacuous ones. `false` when no
/// count can sit in a facet's window.
/// ```text
/// e.g.  {"type": "array", "contains": true, "minContains": 3}
///       =>  {"type": "array", "minItems": 3}
/// ```
fn normalize_contains(leaf: &mut ArrayLeaf) -> bool {
    if leaf.contains.is_empty() {
        return true;
    }
    let mut facets = std::mem::take(&mut leaf.contains);
    facets.sort_by(|left, right| left.schema.cmp(&right.schema));
    let mut merged: Vec<ContainsFacet> = Vec::with_capacity(facets.len());
    for facet in facets {
        match merged.last_mut() {
            // Conjunction of two demands on one schema: the tighter end on each side.
            Some(last) if last.schema == facet.schema => {
                let minimum = last.effective_minimum().max(facet.effective_minimum());
                last.minimum = Some(minimum);
                last.maximum = match (last.maximum.take(), facet.maximum) {
                    (Some(left), Some(right)) => Some(left.min(right)),
                    (one, None) | (None, one) => one,
                };
            }
            _ => merged.push(facet),
        }
    }
    for mut facet in merged {
        let minimum = facet.effective_minimum();
        if facet.maximum.as_ref().is_some_and(|max| minimum > *max) {
            return false;
        }
        // Every element matches, so the matching count is the length itself.
        if matches!(facet.schema.kind(), SchemaKind::True) {
            if !minimum.is_zero()
                && leaf
                    .lengths
                    .minimum
                    .as_ref()
                    .is_none_or(|current| *current < minimum)
            {
                leaf.lengths.minimum = Some(minimum);
            }
            if let Some(maximum) = facet.maximum {
                leaf.lengths.maximum = Some(match leaf.lengths.maximum.take() {
                    Some(current) => current.min(maximum),
                    None => maximum,
                });
            }
            continue;
        }
        // No element matches, so the count is zero: below any positive minimum.
        if matches!(facet.schema.kind(), SchemaKind::False) {
            if minimum.is_zero() {
                continue;
            }
            return false;
        }
        if minimum.is_zero() && facet.maximum.is_none() {
            continue;
        }
        facet.minimum = (minimum != BoundCardinality::from(1)).then_some(minimum);
        leaf.contains.push(facet);
    }
    true
}

/// Check the `contains` demands against the settled length window: matching elements are elements,
/// so the largest demanded count must fit under the ceiling, and any item minimum it implies is
/// dropped as redundant.
fn reconcile_contains_window(leaf: &mut ArrayLeaf) -> bool {
    let Some(implied) = leaf
        .contains
        .iter()
        .map(ContainsFacet::effective_minimum)
        .max()
    else {
        return true;
    };
    if leaf
        .lengths
        .maximum
        .as_ref()
        .is_some_and(|max| implied > *max)
    {
        return false;
    }
    if leaf
        .lengths
        .minimum
        .as_ref()
        .is_some_and(|min| *min <= implied)
    {
        leaf.lengths.minimum = None;
    }
    true
}

/// Fold an array leaf's per-index and tail element constraints into canonical form: drop a tail that
/// says nothing, turn a rejecting tail or prefix schema into a length ceiling, and fold trailing
/// prefix schemas that repeat the tail.
fn normalize_items(leaf: &mut ArrayLeaf) {
    // A tail accepting every value constrains no element beyond the prefix.
    if leaf
        .items
        .as_ref()
        .is_some_and(|tail| matches!(tail.kind(), SchemaKind::True))
    {
        leaf.items = None;
    }
    // A rejecting tail forbids every element beyond the prefix, capping the length at the prefix.
    // e.g.  {"type": "array", "prefixItems": [A, B], "items": false}
    //       =>  {"type": "array", "prefixItems": [A, B], "maxItems": 2}
    if leaf
        .items
        .as_ref()
        .is_some_and(|tail| matches!(tail.kind(), SchemaKind::False))
    {
        let prefix_len = leaf.prefix.len();
        cap_length(leaf, prefix_len);
    }
    // A rejecting prefix schema forbids any array reaching its index, capping the length there.
    // e.g.  {"type": "array", "prefixItems": [A, false]}
    //       =>  {"type": "array", "prefixItems": [A], "maxItems": 1}
    if let Some(rejecting) = leaf
        .prefix
        .iter()
        .position(|schema| matches!(schema.kind(), SchemaKind::False))
    {
        cap_length(leaf, rejecting);
    }
    // No array reaches a prefix index at or beyond the length ceiling, so those schemas never apply.
    if leaf.lengths.maximum.is_some() {
        let keep = reachable_prefix_len(leaf);
        if keep < leaf.prefix.len() {
            leaf.prefix.truncate(keep);
            leaf.items = None;
        }
    }
    // A trailing prefix schema that repeats the tail is already covered by it, tail-of-`true` included.
    // e.g.  {"type": "array", "prefixItems": [A, B], "items": B}
    //       =>  {"type": "array", "prefixItems": [A], "items": B}
    while leaf.prefix.last().is_some_and(|last| match &leaf.items {
        Some(tail) => last == tail,
        None => matches!(last.kind(), SchemaKind::True),
    }) {
        leaf.prefix.pop();
    }
    debug_assert!(
        !leaf
            .prefix
            .iter()
            .any(|schema| matches!(schema.kind(), SchemaKind::False)),
        "a rejecting prefix schema survived normalization"
    );
    debug_assert!(
        reachable_prefix_len(leaf) == leaf.prefix.len(),
        "a prefix schema beyond the length ceiling survived normalization"
    );
}

/// The number of leading prefix schemas an array within the window can actually reach.
fn reachable_prefix_len(leaf: &ArrayLeaf) -> usize {
    leaf.prefix
        .iter()
        .enumerate()
        .take_while(|(index, _)| {
            leaf.lengths
                .maximum
                .as_ref()
                .is_none_or(|max| BoundCardinality::from(*index as u64) < *max)
        })
        .count()
}

/// Cap the length window so no array reaches index `ceiling`, then drop the unreachable prefix tail
/// and the now-unreachable element tail.
fn cap_length(leaf: &mut ArrayLeaf, ceiling: usize) {
    let ceiling = BoundCardinality::from(ceiling as u64);
    leaf.lengths.maximum = Some(match leaf.lengths.maximum.take() {
        Some(max) => max.min(ceiling),
        None => ceiling,
    });
    let keep = reachable_prefix_len(leaf);
    leaf.prefix.truncate(keep);
    leaf.items = None;
}

/// Keep the arrays both leaves accept: the narrower window, distinct items when either asks, and
/// elements both leaves admit at every index.
fn intersect_array_leaves(
    first: ArrayLeaf,
    second: ArrayLeaf,
    ctx: &CanonicalizationContext,
) -> ArrayLeaf {
    let length = first.prefix.len().max(second.prefix.len());
    let mut prefix = Vec::with_capacity(length);
    for index in 0..length {
        // The longer prefix always supplies a schema at every index below `length`, so an index the
        // shorter one leaves open falls back to its tail, and the pair always has something to keep.
        let left = element_constraint(&first, index);
        let right = element_constraint(&second, index);
        prefix.push(intersect(left, right, ctx));
    }
    let items = match (first.items, second.items) {
        (Some(left), Some(right)) => Some(intersect(left, right, ctx)),
        (items, None) | (None, items) => items,
    };
    let mut contains = first.contains;
    contains.extend(second.contains);
    ArrayLeaf {
        lengths: first.lengths.intersect(second.lengths),
        unique: first.unique || second.unique,
        prefix,
        items,
        contains,
    }
}

/// The schema a leaf places on the element at `index`: its prefix schema there, else its tail, else
/// no constraint.
fn element_constraint(leaf: &ArrayLeaf, index: usize) -> Schema {
    leaf.prefix
        .get(index)
        .cloned()
        .or_else(|| leaf.items.clone())
        .unwrap_or_else(|| Schema::new(SchemaKind::True))
}

/// Whether `member` is an array whose length sits in the window, whose every element the item
/// schema admits, and with distinct items when asked.
fn array_leaf_admits(
    leaf: &ArrayLeaf,
    member: &CanonicalJson,
    ctx: &CanonicalizationContext,
) -> Verdict {
    let Value::Array(items) = member.as_value() else {
        return Verdict::Rejects;
    };
    if !leaf
        .lengths
        .contains(&BoundCardinality::from(items.len() as u64))
    {
        return Verdict::Rejects;
    }
    // An undecided element leaves the matching count an interval: `definite` counts sure matches,
    // `possible` also the undecided ones. A window missed at both readings rejects; one met only
    // at the right reading stays undecided.
    let mut contains_verdict = Verdict::Admits;
    for facet in &leaf.contains {
        let mut definite: u64 = 0;
        let mut possible: u64 = 0;
        for element in items {
            match admits_value(&facet.schema, element, ctx) {
                Verdict::Admits => {
                    definite += 1;
                    possible += 1;
                }
                Verdict::Unknown => possible += 1,
                Verdict::Rejects => {}
            }
        }
        let definite = BoundCardinality::from(definite);
        let possible = BoundCardinality::from(possible);
        if possible < facet.effective_minimum()
            || facet.maximum.as_ref().is_some_and(|max| definite > *max)
        {
            return Verdict::Rejects;
        }
        if definite < facet.effective_minimum()
            || facet.maximum.as_ref().is_some_and(|max| possible > *max)
        {
            contains_verdict = Verdict::Unknown;
        }
    }
    // Members are normalized, so `1` and `1.0` compare equal here just as they do at validation.
    if leaf.unique
        && !items
            .iter()
            .enumerate()
            .all(|(index, item)| !items[..index].contains(item))
    {
        return Verdict::Rejects;
    }
    // The element at each index answers to its prefix schema, or the tail once the prefix runs out.
    contains_verdict.and(Verdict::all(items.iter().enumerate().map(
        |(index, element)| match leaf.prefix.get(index).or(leaf.items.as_ref()) {
            Some(schema) => admits_value(schema, element, ctx),
            None => Verdict::Admits,
        },
    )))
}

/// Pack an object facet set into a node, collapsing the leaves that say something simpler.
pub(crate) fn object_leaf(mut leaf: ObjectLeaf, ctx: &CanonicalizationContext) -> Schema {
    normalize_property_names(&mut leaf, ctx);
    // A leaf no facet survives on admits every object, which the bare type set already spells;
    // keeping the leaf shape would give one value set two IR forms.
    if leaf.spans_domain() {
        return type_set_schema(JsonTypeSet::from(JsonType::Object));
    }
    // A stored key constraint says something about the keys: one admitting every string or none at
    // all was folded into the facets above, and leaving it here would spell those two another way.
    debug_assert!(
        !leaf.property_names.as_ref().is_some_and(|names| {
            matches!(names.kind(), SchemaKind::False)
                || matches!(names.kind(), SchemaKind::MultiType(set) if *set == JsonTypeSet::from(JsonType::String))
        }),
        "a key constraint survived normalization without constraining keys"
    );
    // A key no applicable schema leaves a value for can never be present, so demanding it admits
    // nothing. Several schemas can apply to one key, and each alone may still admit something.
    // e.g.  {"type": "object", "properties": {"a": false}, "required": ["a"]}  =>  {"not": {}}
    // e.g.  {"type": "object", "required": ["ab"],
    //        "patternProperties": {"^a": {"type": "string"}, "b$": {"type": "integer"}}}
    //       =>  {"not": {}}
    if leaf
        .required
        .iter()
        .any(|key| matches!(key_schema(&leaf, key, ctx).kind(), SchemaKind::False))
    {
        return Schema::new(SchemaKind::False);
    }
    // A key the property names reject can never be present, so demanding it admits nothing.
    // Collapsing to `False` narrows the schema, so only a definite rejection collapses.
    // e.g.  {"type": "object", "propertyNames": {"const": "foo"}, "required": ["bar"]}
    //       =>  {"not": {}}
    if let Some(names) = &leaf.property_names {
        if leaf
            .required
            .iter()
            .any(|key| matches!(admits_key(names, key, ctx), Verdict::Rejects))
        {
            return Schema::new(SchemaKind::False);
        }
    }
    // Property entries saying nothing go first, or a vacuous named key becomes a fold target and
    // carries the pattern schema as a permanent entry the pattern-only spelling lacks.
    normalize_properties(&mut leaf, ctx);
    normalize_pattern_properties(&mut leaf, ctx);
    // Required keys filling the whole size ceiling leave no slot for any other key, so an entry
    // outside them can never see its key present.
    // e.g.  {"type": "object", "maxProperties": 1, "required": ["b"],
    //        "properties": {"a": {"type": "string"}}}
    //       =>  {"type": "object", "maxProperties": 1, "required": ["b"]}
    if leaf
        .sizes
        .maximum
        .as_ref()
        .is_some_and(|max| *max == leaf.required_count())
    {
        let required = &leaf.required;
        leaf.properties
            .retain(|key, _| required.binary_search(key).is_ok());
    }
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
    // A finite set of admitted keys caps the property count, so a maximum it covers says nothing more.
    // e.g.  {"type": "object", "propertyNames": {"const": "foo"}, "maxProperties": 1}
    //       =>  {"type": "object", "propertyNames": {"const": "foo"}}
    if let Some(admitted) = leaf.admitted_key_count() {
        if leaf
            .sizes
            .maximum
            .as_ref()
            .is_some_and(|max| *max >= admitted)
        {
            leaf.sizes.maximum = None;
        }
    }
    let Some(leaf) = NonEmpty::new(leaf) else {
        return Schema::new(SchemaKind::False);
    };
    // A ceiling of zero present keys accepts the empty object and nothing else, whether spelled as
    // `maxProperties: 0` or as a finite key set whose every key is forbidden; a required key would
    // have emptied the leaf above.
    // e.g.  {"type": "object", "maxProperties": 0}  =>  {"const": {}}
    // e.g.  {"type": "object", "propertyNames": {"const": "a"}, "properties": {"a": false}}
    //       =>  {"const": {}}
    if leaf
        .get()
        .effective_sizes()
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

/// Bring a key constraint into normal form: dropped when it admits every string, and read as an
/// empty object when it admits none.
fn normalize_property_names(leaf: &mut ObjectLeaf, ctx: &CanonicalizationContext) {
    let Some(names) = leaf.property_names.take() else {
        return;
    };
    // Narrowing first is what lets one pass reach normal form: a constraint admitting no string,
    // such as `{"type": "integer"}`, only becomes `False` once the other types are cut away.
    // A constraint already in the string domain skips the intersection it would be an identity of;
    // every stored constraint passes through here again on each union or intersection.
    let names = if is_string_domain(names.kind()) {
        names
    } else {
        narrow_to_strings(names, ctx)
    };
    // Every key is a string, so a constraint admitting all of them constrains nothing.
    if matches!(names.kind(), SchemaKind::MultiType(set) if *set == JsonTypeSet::from(JsonType::String))
    {
        return;
    }
    // No key can be present, which is what an empty object says.
    // e.g.  {"type": "object", "propertyNames": false}  =>  {"const": {}}
    if matches!(names.kind(), SchemaKind::False) {
        leaf.sizes = leaf.sizes.clone().intersect(LengthBounds {
            minimum: None,
            maximum: Some(BoundCardinality::from(0)),
        });
        return;
    }
    leaf.property_names = Some(names);
}

/// Drop the property schemas that say nothing: one accepting every value, and one whose key the
/// key constraint rejects, since that key can never be present to be checked.
fn normalize_properties(leaf: &mut ObjectLeaf, ctx: &CanonicalizationContext) {
    let names = leaf.property_names.clone();
    leaf.properties.retain(|key, schema| {
        // Dropping the entry loses what it says about the key, so only a key the constraint
        // definitely rejects lets the entry go.
        !matches!(schema.kind(), SchemaKind::True)
            && names
                .as_ref()
                .is_none_or(|names| !matches!(admits_key(names, key, ctx), Verdict::Rejects))
    });
}

/// Fold the pattern map into the facets able to hold what it says: an entry saying nothing goes,
/// and a pattern matching a named key moves onto that key's schema.
fn normalize_pattern_properties(leaf: &mut ObjectLeaf, ctx: &CanonicalizationContext) {
    leaf.pattern_properties
        .retain(|_, schema| !matches!(schema.kind(), SchemaKind::True));
    if leaf.pattern_properties.is_empty() {
        return;
    }
    // A key constraint admitting a finite set leaves no key outside it for a pattern to reach, so
    // the pattern schemas move onto the keys they match and the patterns themselves go.
    // e.g.  {"type": "object", "propertyNames": {"const": "b"},
    //        "patternProperties": {"^a": {"type": "integer"}}}
    //       =>  {"type": "object", "propertyNames": {"const": "b"}}
    if let Some(keys) = admitted_keys(leaf) {
        let patterns = std::mem::take(&mut leaf.pattern_properties);
        for key in keys {
            merge_matching_patterns(&mut leaf.properties, &patterns, &key, ctx);
        }
        return;
    }
    // A named key is checked by its own schema and by every pattern matching it, so the two fold
    // together. The pattern stays: it still reaches the keys the property map does not name.
    // e.g.  {"type": "object", "properties": {"ab": {"type": "string"}},
    //        "patternProperties": {"^a": {"minLength": 2}}}
    //       =>  properties `ab` carries both, and `^a` still governs `ac`
    let patterns = leaf.pattern_properties.clone();
    let keys: Vec<Arc<str>> = leaf.properties.keys().cloned().collect();
    for key in keys {
        merge_matching_patterns(&mut leaf.properties, &patterns, &key, ctx);
    }
}

/// Intersect into `properties` what every pattern matching `key` demands of it.
fn merge_matching_patterns(
    properties: &mut BTreeMap<Arc<str>, Schema>,
    patterns: &BTreeMap<Arc<str>, Schema>,
    key: &Arc<str>,
    ctx: &CanonicalizationContext,
) {
    for (pattern, schema) in patterns {
        if !matches_key(pattern, key, ctx) {
            continue;
        }
        let merged = match properties.remove(key) {
            Some(existing) => intersect(existing, schema.clone(), ctx),
            None => schema.clone(),
        };
        properties.insert(Arc::clone(key), merged);
    }
}

/// The keys a finite key constraint admits, when the leaf carries one.
fn admitted_keys(leaf: &ObjectLeaf) -> Option<Vec<Arc<str>>> {
    let values = leaf.property_names.as_ref()?.kind().finite_values()?;
    Some(
        values
            .iter()
            .map(|value| {
                let Value::String(key) = value.as_value() else {
                    unreachable!(
                        "a key constraint survives normalization only in the string domain"
                    )
                };
                Arc::from(key.as_str())
            })
            .collect(),
    )
}

/// What the leaf demands of `key`: its property schema met with every pattern schema matching it.
fn key_schema(leaf: &ObjectLeaf, key: &str, ctx: &CanonicalizationContext) -> Schema {
    let mut schema = leaf
        .properties
        .get(key)
        .cloned()
        .unwrap_or_else(|| Schema::new(SchemaKind::True));
    for (pattern, pattern_schema) in &leaf.pattern_properties {
        if matches_key(pattern, key, ctx) {
            schema = intersect(schema, pattern_schema.clone(), ctx);
        }
    }
    schema
}

/// Whether the pattern reaches `key`; a pattern matches anywhere in it, as `pattern` does.
fn matches_key(pattern: &Arc<str>, key: &str, ctx: &CanonicalizationContext) -> bool {
    ctx.compile_regex(pattern)
        .expect("pattern validated during parsing")
        .is_match(key)
}

/// Restrict a key constraint to the string domain: keys are always strings, so the branches a bare
/// facet keeps for other types say nothing about them.
fn narrow_to_strings(names: Schema, ctx: &CanonicalizationContext) -> Schema {
    let strings = Schema::new(SchemaKind::MultiType(JsonTypeSet::from(JsonType::String)));
    intersect(names, strings, ctx)
}

/// Whether every value the schema admits is a string, making a narrowing intersection an identity.
fn is_string_domain(kind: &SchemaKind) -> bool {
    match kind {
        SchemaKind::Const(value) => value.as_value().is_string(),
        SchemaKind::Enum(values) => values
            .as_slice()
            .iter()
            .all(|value| value.as_value().is_string()),
        SchemaKind::String(_) | SchemaKind::False => true,
        SchemaKind::MultiType(set) => *set == JsonTypeSet::from(JsonType::String),
        SchemaKind::AnyOf(branches) => branches
            .as_slice()
            .iter()
            .all(|branch| is_string_domain(branch.kind())),
        // A typed group exists only under Draft 4, which has no `propertyNames`; grouping it here
        // keeps the answer conservative, and narrowing is the identity on any string-domain schema.
        SchemaKind::True
        | SchemaKind::TypedGroup { .. }
        | SchemaKind::Integer(_)
        | SchemaKind::Number(_)
        | SchemaKind::Array(_)
        | SchemaKind::Object(_)
        | SchemaKind::Raw(_) => false,
    }
}

/// Whether the key constraint admits `key`.
fn admits_key(names: &Schema, key: &str, ctx: &CanonicalizationContext) -> Verdict {
    match names.kind() {
        SchemaKind::Const(value) => {
            Verdict::from_bool(matches!(value.as_value(), Value::String(text) if text == key))
        }
        SchemaKind::Enum(values) => Verdict::from_bool(
            values
                .as_slice()
                .iter()
                .any(|value| matches!(value.as_value(), Value::String(text) if text == key)),
        ),
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
            string_leaf_admits_text(leaf.get(), &regexes, key, ctx)
        }
        SchemaKind::AnyOf(branches) => Verdict::any(
            branches
                .as_slice()
                .iter()
                .map(|branch| admits_key(branch, key, ctx)),
        ),
        // Normalization stores a key constraint only as a string value set, a string leaf, or a
        // union of those: everything else was narrowed or folded away.
        SchemaKind::MultiType(_)
        | SchemaKind::TypedGroup { .. }
        | SchemaKind::True
        | SchemaKind::False
        | SchemaKind::Integer(_)
        | SchemaKind::Number(_)
        | SchemaKind::Array(_)
        | SchemaKind::Object(_)
        | SchemaKind::Raw(_) => {
            unreachable!("a key constraint survives normalization only in the string domain")
        }
    }
}

/// Whether `schema` admits every value in `value`'s equality class.
fn admits_value(schema: &Schema, value: &Value, ctx: &CanonicalizationContext) -> Verdict {
    let member = Schema::new(SchemaKind::Const(CanonicalJson::from_value(value)));
    // Non-`False` is not enough: under Draft 4 the intersection can pin a nested whole number to
    // its integer spelling (a typed group), a strict subset of the member's equality class - the
    // member `1` also matches `1.0`, which an integer-typed property schema rejects.
    if intersect(schema.clone(), member.clone(), ctx) != member {
        return Verdict::Rejects;
    }
    // Intersection reads a format no checker covers as admitting, so its "yes" is definite only
    // when the schema carries none.
    if has_uncheckable_format(schema, ctx) {
        return Verdict::Unknown;
    }
    Verdict::Admits
}

/// Whether `schema` asserts a format this draft has no checker for.
fn has_uncheckable_format(schema: &Schema, ctx: &CanonicalizationContext) -> bool {
    match schema.kind() {
        SchemaKind::String(leaf) => leaf
            .get()
            .formats
            .iter()
            .any(|format| crate::keywords::format::is_valid(ctx.draft(), format, "").is_none()),
        SchemaKind::AnyOf(branches) => branches
            .as_slice()
            .iter()
            .any(|branch| has_uncheckable_format(branch, ctx)),
        SchemaKind::Object(leaf) => leaf
            .get()
            .property_names
            .iter()
            .chain(leaf.get().properties.values())
            .chain(leaf.get().pattern_properties.values())
            .any(|nested| has_uncheckable_format(nested, ctx)),
        SchemaKind::Array(leaf) => leaf
            .get()
            .prefix
            .iter()
            .chain(leaf.get().items.iter())
            .chain(leaf.get().contains.iter().map(|facet| &facet.schema))
            .any(|nested| has_uncheckable_format(nested, ctx)),

        // A typed group's body is a value set, which carries no format.
        SchemaKind::TypedGroup { .. }
        | SchemaKind::MultiType(_)
        | SchemaKind::Integer(_)
        | SchemaKind::Number(_)
        | SchemaKind::Const(_)
        | SchemaKind::Enum(_)
        | SchemaKind::True
        | SchemaKind::False
        | SchemaKind::Raw(_) => false,
    }
}

/// Keep the objects both leaves accept: the narrower window, and every key either demands.
fn intersect_object_leaves(
    first: ObjectLeaf,
    second: ObjectLeaf,
    ctx: &CanonicalizationContext,
) -> ObjectLeaf {
    let mut required = first.required;
    required.extend(second.required);
    required.sort();
    required.dedup();
    let property_names = match (first.property_names, second.property_names) {
        (Some(left), Some(right)) => Some(intersect(left, right, ctx)),
        (names, None) | (None, names) => names,
    };
    let mut properties = first.properties;
    for (key, schema) in second.properties {
        match properties.remove(&key) {
            Some(existing) => properties.insert(key, intersect(existing, schema, ctx)),
            None => properties.insert(key, schema),
        };
    }
    let mut pattern_properties = first.pattern_properties;
    for (pattern, schema) in second.pattern_properties {
        match pattern_properties.remove(&pattern) {
            Some(existing) => pattern_properties.insert(pattern, intersect(existing, schema, ctx)),
            None => pattern_properties.insert(pattern, schema),
        };
    }
    ObjectLeaf {
        sizes: first.sizes.intersect(second.sizes),
        required,
        property_names,
        properties,
        pattern_properties,
    }
}

/// How an object leaf restricts a candidate object member: kept whole, emptied, or pinned to the
/// part of its equality class the property schemas admit.
enum MemberRestriction {
    Full,
    Empty,
    Partial(Schema),
}

/// Restrict `member` to the objects the leaf admits. `Partial` arises only under Draft 4, where a
/// property schema pins a nested whole number to its integer spelling - a strict subset of the
/// member's equality class that only an object leaf demanding exactly the member's keys can spell.
// e.g.  Draft 4, allOf [
//         {"enum": [{"a": 1}]},
//         {"type": "object", "properties": {"a": {"type": "integer"}}}
//       ]  =>  {"type": "object", "required": ["a"], "maxProperties": 1,
//              "properties": {"a": {"type": "integer", "enum": [1]}}}
fn restrict_object_member(
    leaf: &ObjectLeaf,
    member: &CanonicalJson,
    ctx: &CanonicalizationContext,
) -> MemberRestriction {
    let Value::Object(map) = member.as_value() else {
        return MemberRestriction::Empty;
    };
    if !leaf
        .sizes
        .contains(&BoundCardinality::from(map.len() as u64))
        || !leaf.required.iter().all(|key| map.contains_key(&**key))
    {
        return MemberRestriction::Empty;
    }
    // Returning `Empty` drops the member, which narrows the schema, so only a definite rejection
    // rules a key out.
    if !leaf.property_names.as_ref().is_none_or(|names| {
        map.keys()
            .all(|key| !matches!(admits_key(names, key, ctx), Verdict::Rejects))
    }) {
        return MemberRestriction::Empty;
    }
    let mut full = true;
    let mut restricted: BTreeMap<Arc<str>, Schema> = BTreeMap::new();
    for (key, value) in map {
        let pin = Schema::new(SchemaKind::Const(CanonicalJson::from_value(value)));
        let applicable = key_schema(leaf, key, ctx);
        let entry = if matches!(applicable.kind(), SchemaKind::True) {
            pin
        } else {
            let entry = intersect(applicable, pin.clone(), ctx);
            if matches!(entry.kind(), SchemaKind::False) {
                return MemberRestriction::Empty;
            }
            if entry != pin {
                full = false;
            }
            entry
        };
        restricted.insert(Arc::from(key.as_str()), entry);
    }
    if full {
        return MemberRestriction::Full;
    }
    MemberRestriction::Partial(object_leaf(
        ObjectLeaf {
            sizes: LengthBounds {
                minimum: None,
                maximum: Some(BoundCardinality::from(map.len() as u64)),
            },
            required: restricted.keys().cloned().collect(),
            property_names: None,
            properties: restricted,
            pattern_properties: BTreeMap::new(),
        },
        ctx,
    ))
}

/// Whether `member` is an object carrying every required key, every key admitted by the key
/// constraint, and its property count in the window.
fn object_leaf_admits(
    leaf: &ObjectLeaf,
    member: &CanonicalJson,
    ctx: &CanonicalizationContext,
) -> Verdict {
    let Value::Object(map) = member.as_value() else {
        return Verdict::Rejects;
    };
    if !leaf
        .sizes
        .contains(&BoundCardinality::from(map.len() as u64))
        || !leaf.required.iter().all(|key| map.contains_key(&**key))
    {
        return Verdict::Rejects;
    }
    let keys = match &leaf.property_names {
        Some(names) => Verdict::all(map.keys().map(|key| admits_key(names, key, ctx))),
        None => Verdict::Admits,
    };
    if keys == Verdict::Rejects {
        return Verdict::Rejects;
    }
    let values = Verdict::all(map.iter().map(|(key, value)| {
        let named = match leaf.properties.get(key.as_str()) {
            Some(schema) => admits_value(schema, value, ctx),
            None => Verdict::Admits,
        };
        if named == Verdict::Rejects {
            return Verdict::Rejects;
        }
        named.and(Verdict::all(leaf.pattern_properties.iter().map(
            |(pattern, schema)| {
                if matches_key(pattern, key, ctx) {
                    admits_value(schema, value, ctx)
                } else {
                    Verdict::Admits
                }
            },
        )))
    }));
    keys.and(values)
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
    // A leaf no facet survives on admits every integer, which the bare type set already spells;
    // keeping the leaf shape would give one value set two IR forms.
    if leaf.bounds.minimum.is_none() && leaf.bounds.maximum.is_none() && leaf.multiple_of.is_empty()
    {
        return type_set_schema(JsonTypeSet::from(JsonType::Integer));
    }
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
fn string_leaf_admits(
    leaf: &StringLeaf,
    regexes: &[Arc<CompiledMatcher>],
    member: &CanonicalJson,
    ctx: &CanonicalizationContext,
) -> Verdict {
    let Value::String(text) = member.as_value() else {
        return Verdict::Rejects;
    };
    string_leaf_admits_text(leaf, regexes, text, ctx)
}

/// Whether `text` falls within the leaf's length window and matches every pattern and format.
fn string_leaf_admits_text(
    leaf: &StringLeaf,
    regexes: &[Arc<CompiledMatcher>],
    text: &str,
    ctx: &CanonicalizationContext,
) -> Verdict {
    let length = BoundCardinality::from(bytecount::num_chars(text.as_bytes()) as u64);
    if !leaf.lengths.contains(&length) || !regexes.iter().all(|regex| regex.is_match(text)) {
        return Verdict::Rejects;
    }
    Verdict::all(leaf.formats.iter().map(|format| {
        match crate::keywords::format::is_valid(ctx.draft(), format, text) {
            Some(admitted) => Verdict::from_bool(admitted),
            None => Verdict::Unknown,
        }
    }))
}
