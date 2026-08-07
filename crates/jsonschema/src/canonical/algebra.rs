//! Set algebra over canonical IR nodes.
use std::sync::Arc;

use referencing::Draft;
use serde_json::Value;

use crate::{
    canonical::{
        context::{CanonicalizationContext, CompiledMatcher},
        ir::{
            canonicalize_value_set, tighter, type_set_schema, typed_group, ArrayLeaf, ArrayLeaves,
            AscendingMembership, AtLeastTwo, BoundCardinality, BoundInteger, BoundNumber,
            BoundRational, CanonicalJson, ContainsFacet, Discrete, Distinctness, Divisors,
            ExcludedDivisors, IntegerBounds, IntegerLeaf, IntegerLeaves, LengthBounds, NonEmpty,
            NumberLeaf, NumberLeaves, ObjectLeaf, ObjectLeaves, ObjectViolation, PropertyMap,
            Round, Schema, SchemaKind, Side, StringLeaf, StringLeaves, UncheckableFacet, Verdict,
        },
        negate, oracle, parse, DefinitionMap,
    },
    JsonType, JsonTypeSet,
};

/// The schema accepting exactly the values that BOTH `left` and `right` accept (set intersection, `allOf`).
pub(crate) fn intersect(left: Schema, right: Schema, ctx: &CanonicalizationContext) -> Schema {
    // A side that decides the meet on its own hands back the node it already holds. Answering here
    // beats reaching the cache below, whose key comparison walks the other side's whole subtree.
    match (left.kind(), right.kind()) {
        (SchemaKind::False, _) | (_, SchemaKind::True) => return left,
        (SchemaKind::True, _) | (_, SchemaKind::False) => return right,
        _ => {}
    }
    // A node reached from two places is a node the product will reach again, and answering from the
    // first visit stops the whole subtree below it from being walked a second time. A node held
    // nowhere else cannot come back, so remembering it would only cost the room.
    if let Some(remembered) = ctx.recall_intersection(&left, &right) {
        return remembered;
    }
    let key = (left.clone(), right.clone());
    let result = intersect_pair(left, right, ctx);
    ctx.remember_intersection(key.0, key.1, &result);
    result
}

fn intersect_pair(left: Schema, right: Schema, ctx: &CanonicalizationContext) -> Schema {
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
            Schema::falsy()
        }
        // `intersect` hands back the other side before dispatching here.
        (SchemaKind::True, _) | (_, SchemaKind::True) => {
            unreachable!("a `True` side is answered before the pair is dispatched")
        }
        // References stay opaque. Equal references deduplicate; every other interaction remains an
        // exact symbolic conjunction rather than claiming facts about an unresolved target.
        (SchemaKind::Reference(left), SchemaKind::Reference(right)) if left == right => {
            Schema::new(SchemaKind::Reference(left))
        }
        // One side is an `AnyOf` (matches if any branch matches). Push the intersection inside the union:
        // (A or B) and C = (A and C) or (B and C). This happens before opaque ref handling so an `AllOf`
        // never retains a distributable union that would change shape when emitted and parsed again.
        (SchemaKind::AnyOf(branches), other) | (other, SchemaKind::AnyOf(branches)) => {
            distribute(branches, Schema::new(other), ctx)
        }
        (
            left @ (SchemaKind::Not(_)
            | SchemaKind::AllOf(_)
            | SchemaKind::OneOf(_)
            | SchemaKind::Reference(_)),
            right,
        )
        | (
            left,
            right @ (SchemaKind::Not(_)
            | SchemaKind::AllOf(_)
            | SchemaKind::OneOf(_)
            | SchemaKind::Reference(_)),
        ) => opaque_intersection(Schema::new(left), Schema::new(right), ctx),
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
                Schema::falsy()
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
                Schema::falsy()
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
                Schema::falsy()
            }
        }
        // A string leaf constrains string values. A type set keeps it only when the set covers `string`;
        // otherwise the two share no value, so `False`.
        (SchemaKind::MultiType(set), SchemaKind::String(leaf))
        | (SchemaKind::String(leaf), SchemaKind::MultiType(set)) => {
            if SchemaKind::semantic_cover(set).contains(JsonType::String) {
                string_leaf(leaf.into_inner(), ctx)
            } else {
                Schema::falsy()
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
                Schema::falsy()
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
                Schema::falsy()
            }
        }
        // An array leaf constrains array values. A type set keeps it only when the set covers
        // `array`; otherwise the two share no value, so `False`.
        (SchemaKind::MultiType(set), SchemaKind::Array(leaf))
        | (SchemaKind::Array(leaf), SchemaKind::MultiType(set)) => {
            if set.contains(JsonType::Array) {
                array_leaf(leaf.into_inner(), ctx)
            } else {
                Schema::falsy()
            }
        }
        // Two array leaves: keep the arrays both accept - the narrower window, and the distinctness
        // both sides ask for.
        (SchemaKind::Array(first), SchemaKind::Array(second)) => {
            match intersect_array_leaves(first.into_inner(), second.into_inner(), ctx) {
                Some(leaf) => array_leaf(leaf, ctx),
                None => Schema::falsy(),
            }
        }
        // An object leaf constrains object values. A type set keeps it only when the set covers
        // `object`; otherwise the two share no value, so `False`.
        (SchemaKind::MultiType(set), SchemaKind::Object(leaf))
        | (SchemaKind::Object(leaf), SchemaKind::MultiType(set)) => {
            if set.contains(JsonType::Object) {
                object_leaf(leaf.into_inner(), ctx)
            } else {
                Schema::falsy()
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

fn opaque_intersection(left: Schema, right: Schema, ctx: &CanonicalizationContext) -> Schema {
    let mut symbolic = Vec::new();
    let mut structural = Schema::truthy();
    let mut stack = vec![left, right];
    while let Some(schema) = stack.pop() {
        match schema.into_kind() {
            SchemaKind::AllOf(inner) => stack.extend(inner),
            kind @ (SchemaKind::Not(_) | SchemaKind::OneOf(_) | SchemaKind::Reference(_)) => {
                symbolic.push(Schema::new(kind));
            }
            kind @ (SchemaKind::MultiType(_)
            | SchemaKind::TypedGroup { .. }
            | SchemaKind::String(_)
            | SchemaKind::Integer(_)
            | SchemaKind::Number(_)
            | SchemaKind::Array(_)
            | SchemaKind::Object(_)
            | SchemaKind::Const(_)
            | SchemaKind::Enum(_)
            | SchemaKind::AnyOf(_)) => {
                structural = intersect(structural, Schema::new(kind), ctx);
                if matches!(structural.kind(), SchemaKind::False) {
                    return structural;
                }
            }
            // Intersect dispatch consumes both constants before reaching an opaque operand, and an
            // opaque conjunction holds neither, so flattening one never yields them. A definition
            // target that cannot be modeled stays `Raw` in `definitions`, and a reference to it
            // never resolves here, so no combinator ever holds one.
            SchemaKind::True | SchemaKind::False | SchemaKind::Raw(_) => {
                unreachable!("an opaque conjunct is neither a constant nor a whole document")
            }
        }
    }
    debug_assert!(
        !symbolic.is_empty(),
        "opaque intersection retains at least one symbolic branch"
    );
    match structural.into_kind() {
        SchemaKind::AnyOf(branches) => union(
            branches
                .into_iter()
                .map(|branch| {
                    let mut conjuncts = symbolic.clone();
                    conjuncts.push(branch);
                    opaque_conjunction(conjuncts)
                })
                .collect(),
            ctx,
        ),
        SchemaKind::True => opaque_conjunction(symbolic),
        kind @ (SchemaKind::MultiType(_)
        | SchemaKind::TypedGroup { .. }
        | SchemaKind::String(_)
        | SchemaKind::Integer(_)
        | SchemaKind::Number(_)
        | SchemaKind::Array(_)
        | SchemaKind::Object(_)
        | SchemaKind::Const(_)
        | SchemaKind::Enum(_)
        | SchemaKind::Not(_)
        | SchemaKind::AllOf(_)
        | SchemaKind::OneOf(_)
        | SchemaKind::Reference(_)
        | SchemaKind::False
        | SchemaKind::Raw(_)) => {
            symbolic.push(Schema::new(kind));
            opaque_conjunction(symbolic)
        }
    }
}

fn opaque_conjunction(branches: Vec<Schema>) -> Schema {
    for branch in &branches {
        if let SchemaKind::Not(inner) = branch.kind() {
            if branches.iter().any(|candidate| candidate == inner) {
                return Schema::falsy();
            }
        }
    }
    let schema = match AtLeastTwo::new(branches) {
        Ok(branches) => {
            debug_assert!(
                branches.as_slice().iter().all(|branch| !matches!(
                    branch.kind(),
                    SchemaKind::True
                        | SchemaKind::False
                        | SchemaKind::AllOf(_)
                        | SchemaKind::AnyOf(_)
                )),
                "opaque conjunction branches are flattened, non-trivial, and distributable unions are eliminated"
            );
            Schema::new(SchemaKind::AllOf(branches))
        }
        Err(mut lone) => lone.pop().unwrap_or_else(Schema::truthy),
    };
    debug_assert!(
        contains_reference(&schema),
        "opaque intersection is constructed only across a symbolic reference"
    );
    schema
}

/// One representative per branch appearing at least twice, and the branches appearing exactly once.
/// Requires sorted `branches`, so equal ones are adjacent.
fn partition_by_multiplicity(branches: &[Schema]) -> (Vec<Schema>, Vec<Schema>) {
    let mut duplicates = Vec::new();
    let mut singles = Vec::new();
    let mut start = 0;
    while start < branches.len() {
        let mut end = start + 1;
        while end < branches.len() && branches[end] == branches[start] {
            end += 1;
        }
        if end - start >= 2 {
            duplicates.push(branches[start].clone());
        } else {
            singles.push(branches[start].clone());
        }
        start = end;
    }
    (duplicates, singles)
}

/// The schema accepting every value that EXACTLY ONE of the `branches` accepts (`oneOf`), in normal
/// form. `None` when the exclusivity has no exact encoding, keeping the document raw.
///
/// A reference is opaque to intersection and negation, so a branch holding one keeps the
/// exclusivity symbolic instead of expanding it; the rest take [`concrete_one_of`]. `pending`
/// collects the choices a target still being parsed left undecided.
pub(crate) fn one_of(
    mut branches: Vec<Schema>,
    definitions: &DefinitionMap,
    finished: &DefinitionMap,
    pending: &mut Vec<Vec<Schema>>,
    ctx: &CanonicalizationContext,
) -> Option<Schema> {
    if !branches.iter().any(contains_reference) {
        return concrete_one_of(branches, definitions, ctx);
    }
    // A branch accepting nothing can never be the single match.
    branches.retain(|branch| !matches!(branch.kind(), SchemaKind::False));
    branches.sort();

    // A repeated branch contributes 0 or at least 2 matches, never exactly 1, so `oneOf [A, A, B]`
    // is `B and not A`. Dropping the copies without the complement would admit a value in both.
    let (duplicates, singles) = partition_by_multiplicity(&branches);
    if !duplicates.is_empty() {
        if singles.is_empty() {
            return Some(Schema::falsy());
        }
        let mut complements = Vec::with_capacity(duplicates.len());
        for duplicate in &duplicates {
            // All or nothing: dropping a duplicate whose exclusion is never restated is unsound.
            let Some(complement) = negate::negate_in_place(duplicate, definitions, ctx) else {
                complements.clear();
                break;
            };
            complements.push(complement);
        }
        if complements.len() == duplicates.len() {
            // Survivors re-enter from the top, not wrapped in a `OneOf` here: the duplicates may
            // have held the only references, and a reference-free remainder must take the concrete
            // route or it emits a form that canonicalizes to something else.
            let mut result = one_of(singles, definitions, finished, pending, ctx)?;
            for complement in complements {
                result = intersect(result, complement, ctx);
            }
            return Some(result);
        }
    }

    // A lone branch is itself; a one-element `OneOf` would emit as `{"oneOf": [X]}` instead of `X`.
    if branches.len() == 1 {
        return branches.pop();
    }
    debug_assert!(
        branches.windows(2).all(|pair| pair[0] <= pair[1]),
        "oneOf branches are sorted without deduplication"
    );
    // Sharing no JSON type, no two branches hold a value in common, and "exactly one" is then "at
    // least one". The types the targets admit decide that; the branches keep the references they
    // were spelled with. Weighing the bodies themselves would mean intersecting them, which costs
    // as much again as canonicalizing the document they came from.
    // ```text
    // e.g.  oneOf [{"$ref": "#/$defs/count"}, {"type": "array"}]  with  count = {"type": "integer"}
    //       =>  anyOf [{"type": "array"}, {"$ref": "#/$defs/count"}]
    //       oneOf [{"$ref": "#/$defs/plain"}, {"$ref": "#/$defs/tight"}]  => unchanged, both strings
    // ```
    let mut awaits_body = false;
    // One resolution pass feeds both tests: a lookup walks the document's whole definition map, so
    // resolving a branch twice is the expensive half of the check.
    match pointer_targets(&branches, definitions, finished, &mut awaits_body) {
        Some(targets)
            if types_are_disjoint(&targets) || scalar_bodies_are_disjoint(&targets, ctx) =>
        {
            return Some(union(branches, ctx))
        }
        // A body the round has yet to produce leaves the choice for the caller to settle.
        None if awaits_body => pending.push(branches.clone()),
        Some(_) | None => {}
    }
    Some(Schema::new(SchemaKind::OneOf(branches)))
}

/// Whether the choice these branches spell degrades to a union once every body they name is known.
pub(crate) fn choice_folds(
    branches: &[Schema],
    definitions: &DefinitionMap,
    ctx: &CanonicalizationContext,
) -> bool {
    let mut awaits_body = false;
    let known = DefinitionMap::new();
    pointer_targets(branches, definitions, &known, &mut awaits_body).is_some_and(|targets| {
        types_are_disjoint(&targets) || scalar_bodies_are_disjoint(&targets, ctx)
    })
}

/// The body a branch stands for, following a pointer that names another.
fn pointer_target<'a>(
    branch: &'a Schema,
    definitions: &'a DefinitionMap,
    finished: &'a DefinitionMap,
    awaits_body: &mut bool,
) -> Option<&'a Schema> {
    let mut current = branch;
    let mut walked: Vec<&Arc<str>> = Vec::new();
    while let SchemaKind::Reference(uri) = current.kind() {
        // A pointer reached twice on one path names a body that never arrives.
        if walked.contains(&uri) {
            return None;
        }
        // A target still being parsed has no body in this round's map; the previous round's stands
        // in for it, and where neither holds one the caller re-parses to get it.
        let target = definitions
            .get(uri.as_ref())
            .or_else(|| finished.get(uri.as_ref()));
        let Some(target) = target else {
            *awaits_body = true;
            return None;
        };
        walked.push(uri);
        // Every walked pointer named a definition, and no two of them are the same.
        debug_assert!(
            walked.len() <= definitions.len() + finished.len(),
            "more pointers walked than the document defines"
        );
        current = target;
    }
    Some(current)
}

/// Whether the nodes share no value, weighed one against another. Only scalar bodies are weighed:
/// an array or object body costs as much to intersect as the document it came from.
fn scalar_bodies_are_disjoint(targets: &[&Schema], ctx: &CanonicalizationContext) -> bool {
    let scalar = |schema: &Schema| {
        matches!(
            schema.kind(),
            SchemaKind::Const(_)
                | SchemaKind::Enum(_)
                | SchemaKind::MultiType(_)
                | SchemaKind::String(_)
                | SchemaKind::Integer(_)
                | SchemaKind::Number(_)
        )
    };
    if !targets.iter().all(|target| scalar(target)) {
        return false;
    }
    let scalars: Vec<Schema> = targets.iter().map(|target| (*target).clone()).collect();
    pairwise_overlaps(&scalars, ctx).is_empty()
}

/// What each branch stands for, with a pointer replaced by the body it names, or `None` where one
/// leads outside the document or back into itself.
fn pointer_targets<'a>(
    branches: &'a [Schema],
    definitions: &'a DefinitionMap,
    finished: &'a DefinitionMap,
    awaits_body: &mut bool,
) -> Option<Vec<&'a Schema>> {
    let mut targets = Vec::with_capacity(branches.len());
    for branch in branches {
        targets.push(pointer_target(branch, definitions, finished, awaits_body)?);
    }
    Some(targets)
}

/// Whether no two of the nodes admit a value of the same JSON type.
fn types_are_disjoint(targets: &[&Schema]) -> bool {
    let mut covered = JsonTypeSet::empty();
    for target in targets {
        let types = admitted_types(target);
        if !covered.intersect(types).is_empty() {
            return false;
        }
        covered = covered.union(types);
    }
    true
}

/// The JSON types a node can admit, over-approximated: a node holding a reference or a complement
/// stands for every type, which keeps a disjointness claim conservative.
fn admitted_types(schema: &Schema) -> JsonTypeSet {
    match schema.kind() {
        SchemaKind::False => JsonTypeSet::empty(),
        SchemaKind::MultiType(set) => SchemaKind::semantic_cover(*set),
        SchemaKind::TypedGroup { ty, .. } => JsonTypeSet::from(*ty),
        SchemaKind::String(_) => JsonTypeSet::from(JsonType::String),
        SchemaKind::Integer(_) => JsonTypeSet::from(JsonType::Integer),
        SchemaKind::Number(_) => SchemaKind::semantic_cover(JsonTypeSet::from(JsonType::Number)),
        SchemaKind::Array(_) => JsonTypeSet::from(JsonType::Array),
        SchemaKind::Object(_) => JsonTypeSet::from(JsonType::Object),
        SchemaKind::Const(value) => value_types(std::slice::from_ref(value)),
        SchemaKind::Enum(values) => value_types(values.as_slice()),
        SchemaKind::AnyOf(branches) => branches
            .as_slice()
            .iter()
            .fold(JsonTypeSet::empty(), |types, branch| {
                types.union(admitted_types(branch))
            }),
        SchemaKind::OneOf(branches) => {
            branches.iter().fold(JsonTypeSet::empty(), |types, branch| {
                types.union(admitted_types(branch))
            })
        }
        SchemaKind::True
        | SchemaKind::Not(_)
        | SchemaKind::AllOf(_)
        | SchemaKind::Reference(_)
        | SchemaKind::Raw(_) => JsonTypeSet::all(),
    }
}

/// The types the values stand for. Draft 4 matches a whole number by equality, so `1` accepts the
/// float spelling `1.0` its `integer` type rejects, and a numeric value stands for both.
fn value_types(values: &[CanonicalJson]) -> JsonTypeSet {
    values.iter().fold(JsonTypeSet::empty(), |types, value| {
        let ty = value.json_type();
        types.union(match ty {
            JsonType::Integer | JsonType::Number => {
                SchemaKind::semantic_cover(JsonTypeSet::from(JsonType::Number))
            }
            JsonType::Null
            | JsonType::Boolean
            | JsonType::String
            | JsonType::Array
            | JsonType::Object => JsonTypeSet::from(ty),
        })
    })
}

/// [`one_of`] over branches none of which holds a reference: some branch matches and no two-branch
/// overlap does, so only the overlaps need complements — a branch overlapping nothing is never
/// negated. `None` when an overlap's complement is inexpressible.
fn concrete_one_of(
    branches: Vec<Schema>,
    definitions: &DefinitionMap,
    ctx: &CanonicalizationContext,
) -> Option<Schema> {
    let overlaps = pairwise_overlaps(&branches, ctx);
    let mut spelled = branches.clone();
    let mut result = union(branches, ctx);
    for overlap in overlaps {
        result = intersect(
            result,
            negate::negate_in_place(&overlap, definitions, ctx)?,
            ctx,
        );
        // Every shared region removed widens the union again, and the widths multiply, so past the
        // budget the choice keeps the exactly-one spelling: it is exact and far smaller.
        if negate::union_width(&result) > negate::CONJUNCTION_BUDGET {
            spelled.sort();
            return Some(Schema::new(SchemaKind::OneOf(spelled)));
        }
    }
    Some(result)
}

/// Every region two branches share: the values repeating across finite-value branches packed as
/// one value set, and the non-`False` pairwise intersections involving structural branches. Empty
/// exactly when the branches are pairwise disjoint, so `oneOf` degrades to `anyOf`.
///
/// Finite-value branches share a value exactly when a member repeats across them, so one hash set
/// replaces their share of the quadratic sweep; only the remaining branches pay a pairwise
/// `intersect`, plus one `intersect` against each finite-value branch.
fn pairwise_overlaps(branches: &[Schema], ctx: &CanonicalizationContext) -> Vec<Schema> {
    let mut seen: ahash::AHashSet<&CanonicalJson> = ahash::AHashSet::new();
    let mut shared: Vec<CanonicalJson> = Vec::new();
    let mut finite: Vec<&Schema> = Vec::new();
    let mut structural: Vec<&Schema> = Vec::new();
    for branch in branches {
        match branch.kind() {
            SchemaKind::Const(value) => {
                if !seen.insert(value) {
                    shared.push(value.clone());
                }
                finite.push(branch);
            }
            SchemaKind::Enum(values) => {
                for value in values.as_slice() {
                    if !seen.insert(value) {
                        shared.push(value.clone());
                    }
                }
                finite.push(branch);
            }
            SchemaKind::MultiType(_)
            | SchemaKind::TypedGroup { .. }
            | SchemaKind::String(_)
            | SchemaKind::Integer(_)
            | SchemaKind::Number(_)
            | SchemaKind::Array(_)
            | SchemaKind::Object(_)
            | SchemaKind::Not(_)
            | SchemaKind::AllOf(_)
            | SchemaKind::AnyOf(_)
            | SchemaKind::OneOf(_)
            | SchemaKind::Reference(_)
            | SchemaKind::True
            | SchemaKind::False
            | SchemaKind::Raw(_) => structural.push(branch),
        }
    }
    let mut overlaps = Vec::new();
    if !shared.is_empty() {
        overlaps.push(canonicalize_value_set(shared));
    }
    for (index, left) in structural.iter().enumerate() {
        for right in structural[index + 1..].iter().chain(&finite) {
            let intersection = intersect((*left).clone(), (*right).clone(), ctx);
            if !matches!(intersection.kind(), SchemaKind::False) {
                overlaps.push(intersection);
            }
        }
    }
    overlaps
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
    let mut symbolic_branches: Vec<Schema> = Vec::new();

    let mut stack = branches;
    while let Some(branch) = stack.pop() {
        match branch.into_kind() {
            // A branch that accepts everything makes the whole union accept everything.
            SchemaKind::True => return Schema::truthy(),
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
            SchemaKind::Not(schema) => {
                let complement = Schema::new(SchemaKind::Not(schema));
                if !symbolic_branches
                    .iter()
                    .any(|existing| existing == &complement)
                {
                    symbolic_branches.push(complement);
                }
            }
            SchemaKind::AllOf(branches) => {
                let conjunction = Schema::new(SchemaKind::AllOf(branches));
                if !symbolic_branches
                    .iter()
                    .any(|existing| existing == &conjunction)
                {
                    symbolic_branches.push(conjunction);
                }
            }
            SchemaKind::OneOf(branches) => {
                let exclusive = Schema::new(SchemaKind::OneOf(branches));
                if !symbolic_branches
                    .iter()
                    .any(|existing| existing == &exclusive)
                {
                    symbolic_branches.push(exclusive);
                }
            }
            SchemaKind::Reference(uri) => {
                let reference = Schema::new(SchemaKind::Reference(uri));
                if !symbolic_branches
                    .iter()
                    .any(|existing| existing == &reference)
                {
                    symbolic_branches.push(reference);
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
        return Schema::truthy();
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
        if drop_object_branch_covered_by_siblings(&mut objects, ctx) {
            continue;
        }
        if drop_required_covered_by_sibling(&mut objects, ctx) {
            continue;
        }
        if drop_size_bound_covered_by_sibling(&mut objects, ctx) {
            continue;
        }
        if collapse_object_leaves_covering_domain(&mut objects, ctx) {
            continue;
        }
        if widen_size_window_covered_by_siblings(&mut objects, ctx) {
            continue;
        }
        if !widen_entry_covered_by_sibling(&mut objects, ctx) {
            break;
        }
    }
    let mut widened = types;
    integers.retain(|leaf| {
        let spans_domain = leaf.bounds.is_unbounded()
            && leaf.multiple_of.is_empty()
            && leaf.not_multiple_of.is_empty();
        if spans_domain {
            widened = union_type_sets(widened, JsonTypeSet::from(JsonType::Integer));
        }
        !spans_domain
    });
    numbers.retain(|leaf| {
        let spans_domain = leaf.minimum.is_none()
            && leaf.maximum.is_none()
            && leaf.multiple_of.is_empty()
            && leaf.not_multiple_of.is_empty()
            && !leaf.excludes_integers;
        if spans_domain {
            widened = union_type_sets(widened, JsonTypeSet::from(JsonType::Number));
        }
        !spans_domain
    });
    strings.retain(|leaf| {
        let spans_domain = leaf.lengths.is_unbounded()
            && leaf.patterns.is_empty()
            && leaf.excluded_patterns.is_empty()
            && leaf.formats.is_empty()
            && leaf.excluded_formats.is_empty()
            && leaf.content_media_types.is_empty()
            && leaf.content_encodings.is_empty()
            && leaf.excluded.is_empty();
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
            widened,
            members,
            groups,
            strings,
            integers,
            numbers,
            arrays,
            objects,
            symbolic_branches,
            ctx,
        );
    }

    // An integer branch whose values a real interval also accepts adds nothing beside it. A divisor
    // of one over a whole number leaves every integer a multiple, so an interval spanning the window
    // under such a divisor takes it entire.
    // e.g.  anyOf [
    //         {"type": "integer", "minimum": -5},
    //         {"type": "number", "multipleOf": 0.1}
    //       ]  =>  {"type": "number", "multipleOf": 0.1}
    // A divisor the window's integers step past keeps the two apart.
    // e.g.  anyOf [
    //         {"type": "integer", "minimum": -5},
    //         {"type": "number", "multipleOf": 1.5}
    //       ]  =>  unchanged
    if !numbers.is_empty() {
        let intervals = numbers.as_slice();
        integers.retain(|window| {
            !intervals
                .iter()
                .any(|interval| number_leaf_covers_integer_leaf(interval, window))
        });
        // Draft 4 keeps a whole value under an `integer` guard, where `7` does not match `7.0`. The
        // interval matches both, so it still holds everything the guard leaves.
        // e.g.  Draft 4, anyOf [
        //         {"type": "integer", "enum": [1, 2]},
        //         {"type": "number", "multipleOf": 0.5}
        //       ]  =>  {"type": "number", "multipleOf": 0.5}
        groups.retain(|(ty, values)| {
            *ty != JsonType::Integer
                || !values.iter().all(|member| {
                    intervals
                        .iter()
                        .any(|leaf| number_leaf_admits(leaf, member))
                })
        });
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
        let compiled: Vec<(&StringLeaf, StringMatchers)> = strings
            .as_slice()
            .iter()
            .map(|leaf| (leaf, StringMatchers::compile(leaf, ctx)))
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
            symbolic_branches,
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
                    widened,
                    remaining,
                    groups,
                    strings,
                    integers,
                    numbers,
                    arrays,
                    objects,
                    symbolic_branches,
                    ctx,
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
        out.push(array_leaf(leaf, ctx));
    }
    // Each surviving object leaf becomes its own branch.
    for leaf in objects {
        debug_assert!(
            !leaf.spans_domain(),
            "a leaf spanning the object domain joins the type set before assembly"
        );
        out.push(object_leaf(leaf, ctx));
    }
    out.extend(symbolic_branches);
    // The loose value set becomes a branch, unless it collapsed to empty.
    if !matches!(value_set.kind(), SchemaKind::False) {
        out.push(value_set);
    }

    // Shedding a conjunct leaves a plain leaf where a conjunction stood, and leaves are weighed
    // against each other in the pools this pass has already run, so the pass runs again over the
    // shed branches. Every shed lowers the number of conjuncts the branches hold and a pass mints
    // no conjunction of its own, which bounds the recursion.
    // e.g.  anyOf [
    //         {"type": "object", "properties": {"a": false}},
    //         allOf [{"type": "object"}, {"$ref": "#/$defs/integer"}],
    //         {"not": {"$ref": "#/$defs/integer"}}
    //       ]  =>  anyOf [{"type": "object"}, {"not": {"$ref": "#/$defs/integer"}}]
    let held = conjuncts_held(&out);
    if drop_conjuncts_a_complement_branch_covers(&mut out) {
        debug_assert!(
            conjuncts_held(&out) < held,
            "shedding left the branches as they were"
        );
        return union(out, ctx);
    }
    // A direct branch absorbs every stricter conjunction containing it: `A or (A and B) = A`.
    let top_level: ahash::AHashSet<Schema> = out.iter().cloned().collect();
    // A branch beside its own complement leaves no value out: `A or (not A) = true`.
    if out.iter().any(
        |branch| matches!(branch.kind(), SchemaKind::Not(operand) if top_level.contains(operand)),
    ) {
        return Schema::truthy();
    }
    out.retain(|branch| {
        let SchemaKind::AllOf(conjuncts) = branch.kind() else {
            return true;
        };
        !conjuncts
            .as_slice()
            .iter()
            .any(|conjunct| top_level.contains(conjunct))
    });
    drop_covered_conjunctions(&mut out, ctx);
    drop_property_alternatives_covered_by_sibling(&mut out, ctx);

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
            None => Schema::falsy(),
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
                distinctness: Distinctness::Unconstrained,
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
                additional: None,
                required: Vec::new(),
                property_names: None,
                properties: PropertyMap::default(),
                pattern_properties: PropertyMap::default(),
                violations: Vec::new(),
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
                excluded_patterns: Vec::new(),
                formats: Vec::new(),
                excluded_formats: Vec::new(),
                content_media_types: Vec::new(),
                content_encodings: Vec::new(),
                excluded: Vec::new(),
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
                not_multiple_of: ExcludedDivisors::default(),
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
                not_multiple_of: ExcludedDivisors::default(),
                excludes_integers: false,
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
    symbolic_branches: Vec<Schema>,
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
    rest.extend(arrays.into_iter().map(|leaf| array_leaf(leaf, ctx)));
    rest.extend(objects.into_iter().map(|leaf| object_leaf(leaf, ctx)));
    rest.extend(symbolic_branches);
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
                if leaves[first].additional.is_some() || leaves[second].additional.is_some() {
                    continue;
                }
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
        // Merging with unequal violation lists would silently drop one side's constraint: with
        // equal lists the merge distributes, `(A and v) or (B and v) = (A or B) and v`.
        || left.violations != right.violations
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
        additional: None,
        violations: left.violations.clone(),
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

/// Collapse the leaves to the bare object type when together they admit every object: no leaf is
/// redundant on its own, but splitting the unconstrained object by each key any leaf mentions
/// lands every piece inside some leaf.
/// ```text
/// e.g.  anyOf [
///         {"type": "object", "properties": {"a": false}},
///         {"type": "object", "properties": {"b": {"type": "null"}}},
///         {"type": "object", "minProperties": 2}
///       ]  =>  {"type": "object"}    (with `a`: a non-null `b` makes two properties,
///                                     a null or missing `b` fits the second branch)
/// ```
fn collapse_object_leaves_covering_domain(
    leaves: &mut Vec<ObjectLeaf>,
    ctx: &CanonicalizationContext,
) -> bool {
    if leaves.len() < 2 {
        return false;
    }
    let mut keys: Vec<Arc<str>> = leaves
        .iter()
        .flat_map(|leaf| leaf.required.iter().chain(leaf.properties.keys()).cloned())
        .collect();
    keys.sort();
    keys.dedup();
    let piece = ObjectLeaf {
        sizes: LengthBounds::default(),
        required: Vec::new(),
        property_names: None,
        properties: PropertyMap::default(),
        pattern_properties: PropertyMap::default(),
        additional: None,
        // Coverage goes through `oracle::covers` (intersect plus structural equality), so a
        // violation-carrying leaf never falsely covers a violation-free piece.
        violations: Vec::new(),
    };
    let packed = packed_leaves(leaves, ctx);
    if !split_piece_is_covered(piece.clone(), &packed, &keys, ctx) {
        return false;
    }
    leaves.clear();
    leaves.push(piece);
    true
}

/// Pack every leaf into a node once. The coverage walk below tests one node against the same set of
/// leaves at every step of a split, and packing carries a full copy of the property map.
fn packed_leaves(leaves: &[ObjectLeaf], ctx: &CanonicalizationContext) -> Vec<Schema> {
    leaves
        .iter()
        .map(|leaf| object_leaf(leaf.clone(), ctx))
        .collect()
}

/// The packed leaves other than `index`.
fn siblings_of(packed: &[Schema], index: usize) -> Vec<Schema> {
    packed
        .iter()
        .enumerate()
        .filter(|(sibling, _)| *sibling != index)
        .map(|(_, schema)| schema.clone())
        .collect()
}

/// Every key the leaves other than `index` name, sorted and deduplicated.
fn keys_beside(leaves: &[ObjectLeaf], index: usize) -> Vec<Arc<str>> {
    let mut keys: Vec<Arc<str>> = leaves
        .iter()
        .enumerate()
        .filter(|(sibling, _)| *sibling != index)
        .flat_map(|(_, leaf)| leaf.required.iter().chain(leaf.properties.keys()).cloned())
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

/// Whether some leaf admits the whole piece, or both halves of a key-presence split do
/// recursively. The key list shrinks with each split, which bounds the recursion.
fn split_piece_is_covered(
    piece: ObjectLeaf,
    leaves: &[Schema],
    keys: &[Arc<str>],
    ctx: &CanonicalizationContext,
) -> bool {
    debug_assert!(
        keys.windows(2).all(|pair| pair[0] < pair[1]),
        "the split keys are sorted and deduplicated"
    );
    let schema = object_leaf(piece.clone(), ctx);
    if matches!(schema.kind(), SchemaKind::False) {
        return true;
    }
    let mut any_within_reach = false;
    for leaf in leaves {
        if !piece_meets_demands(&schema, leaf) {
            continue;
        }
        any_within_reach = true;
        if oracle::covers(&schema, leaf, ctx) == Verdict::Admits {
            return true;
        }
    }
    // Barring a key leaves the required list alone, so a leaf out of reach here is out of reach for
    // every piece down the chain of missing halves. That chain ends out of keys, uncovered, and -
    // when barring cannot empty a piece - still admitting something, so it answers no, and one no
    // settles the conjunction below.
    if !any_within_reach && barring_keys_keeps_the_piece(&piece, keys) {
        return false;
    }
    let Some((key, rest)) = keys.split_first() else {
        return false;
    };
    let mut holding = piece.clone();
    if let Err(position) = holding.required.binary_search(key) {
        holding.required.insert(position, Arc::clone(key));
    }
    let mut missing = piece;
    missing.properties.insert(Arc::clone(key), Schema::falsy());
    split_piece_is_covered(holding, leaves, rest, ctx)
        && split_piece_is_covered(missing, leaves, rest, ctx)
}

/// Whether the piece demands every key the leaf does, both already packed by [`object_leaf`].
///
/// A leaf failing this cannot admit the piece: the intersection unions the two required lists and
/// packing never touches that list, so a key only the leaf demands survives into the result and
/// tells the two apart. Deciding it reads the required lists alone, where the intersection would
/// merge the property maps - the part that costs, a piece carrying one entry per split key.
fn piece_meets_demands(piece: &Schema, leaf: &Schema) -> bool {
    let (SchemaKind::Object(piece_leaf), SchemaKind::Object(other)) = (piece.kind(), leaf.kind())
    else {
        return true;
    };
    let demanded = &piece_leaf.get().required;
    other
        .get()
        .required
        .iter()
        .all(|key| demanded.binary_search(key).is_ok())
}

/// Whether barring any of these keys leaves the piece saying the same thing about its required
/// list and still admitting something. A key constraint, a shield, a pattern map or a size ceiling
/// read the key set as a whole, so under any of them a barred key reaches further than the entry it
/// adds; and barring a key the piece demands empties it outright.
fn barring_keys_keeps_the_piece(piece: &ObjectLeaf, keys: &[Arc<str>]) -> bool {
    piece.property_names.is_none()
        && piece.additional.is_none()
        && piece.pattern_properties.is_empty()
        && piece.sizes.maximum.is_none()
        && piece
            .required
            .iter()
            .all(|key| keys.binary_search(key).is_err())
}

/// Drop a size bound when the region it excludes - the leaf's other facets on the outer ray - is
/// jointly covered by the siblings, so the wider window admits nothing new.
/// ```text
/// e.g.  anyOf [
///         {"type": "object", "properties": {"a": false}},
///         {"type": "object", "properties": {"b": {"type": "null"}}},
///         {"type": "object", "minProperties": 2, "maxProperties": 2}
///       ]  =>  anyOf [..., {"type": "object", "maxProperties": 2}]
///              (a lone property either is not `a` or leaves `b` missing)
/// ```
fn widen_size_window_covered_by_siblings(
    leaves: &mut [ObjectLeaf],
    ctx: &CanonicalizationContext,
) -> bool {
    let packed = packed_leaves(leaves, ctx);
    for index in 0..leaves.len() {
        let Some(rays) = negate::length_windows(&leaves[index].sizes) else {
            continue;
        };
        if rays.is_empty() {
            continue;
        }
        let siblings = siblings_of(&packed, index);
        let keys = keys_beside(leaves, index);
        for ray in rays {
            let drops_minimum = ray.minimum.is_none();
            let mut piece = leaves[index].clone();
            piece.sizes = ray;
            if split_piece_is_covered(piece, &siblings, &keys, ctx) {
                if drops_minimum {
                    leaves[index].sizes.minimum = None;
                } else {
                    leaves[index].sizes.maximum = None;
                }
                return true;
            }
        }
    }
    false
}

/// Drop a branch its siblings jointly admit: some sibling covers it whole, or splitting it - by
/// the keys the siblings mention, and by a sibling's size window - lands every piece inside one.
/// ```text
/// e.g.  anyOf [
///         {"type": "object", "properties": {"a": {"type": "null"}}},
///         {"type": "object", "minProperties": 2, "properties": {"a": {"type": "null"}}}
///       ]  =>  {"type": "object", "properties": {"a": {"type": "null"}}}
/// e.g.  anyOf [
///         {"type": "object", "required": ["a", "b"]},
///         {"type": "object", "minProperties": 3, "properties": {"a": false}},
///         {"type": "object", "minProperties": 3, "required": ["b"]}
///       ]  =>  the third branch dissolves: with `a` it fits the first, without `a` the second
/// ```
fn drop_object_branch_covered_by_siblings(
    leaves: &mut Vec<ObjectLeaf>,
    ctx: &CanonicalizationContext,
) -> bool {
    let packed = packed_leaves(leaves, ctx);
    for index in 0..leaves.len() {
        let siblings = siblings_of(&packed, index);
        // Siblings sharing no value with the branch cover no part of it, and every piece the walks
        // below cut out is part of it.
        if siblings.iter().all(|sibling| {
            matches!(
                intersect(packed[index].clone(), sibling.clone(), ctx).kind(),
                SchemaKind::False
            )
        }) {
            continue;
        }
        let keys = keys_beside(leaves, index);
        if split_piece_is_covered(leaves[index].clone(), &siblings, &keys, ctx) {
            leaves.remove(index);
            return true;
        }
        // A sibling's size window also splits the branch: the parts inside the window and on the
        // rays outside it partition it, and each part must be covered on its own.
        // e.g.  anyOf [
        //         {"type": "object", "required": ["a"], "properties": {"a": {"type": "string"}}},
        //         {"type": "object", "maxProperties": 1, "required": ["a"]},
        //         {"type": "object", "minProperties": 2, "properties": {"a": {"type": "string"}}}
        //       ]  =>  the first branch dissolves: at one key the entry says nothing beside the
        //              filled slots, above that the third branch holds it
        for divider in 0..leaves.len() {
            if divider == index {
                continue;
            }
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
                split_piece_is_covered(piece, &siblings, &keys, ctx)
            });
            if all_covered {
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
        if leaves[index].additional.is_some() {
            continue;
        }
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
                gained.properties.insert(Arc::clone(&key), Schema::falsy());
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
                            .or_insert_with(Arc::clone(key), || entry.clone());
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
        if leaves[index].additional.is_some() {
            continue;
        }
        let keys: Vec<Arc<str>> = leaves[index].properties.keys().cloned().collect();
        for key in keys {
            for sibling in (0..leaves.len()).filter(|&sibling| sibling != index) {
                if leaves[sibling].additional.is_some() {
                    continue;
                }
                let entry = leaves[index]
                    .properties
                    .get(&key)
                    .expect("the key came from this leaf");
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
        | SchemaKind::Not(_)
        | SchemaKind::AllOf(_)
        | SchemaKind::AnyOf(_)
        | SchemaKind::OneOf(_)
        | SchemaKind::Reference(_)
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
            let matchers = StringMatchers::compile(leaf.get(), ctx);
            let kept = members
                .into_iter()
                // A value set holds no facet, so a facet no checker covers cannot survive beside a
                // member and reads here the way a validator without a checker reads it.
                .filter(|member| {
                    !matches!(
                        string_leaf_admits(
                            leaf.get(),
                            &matchers,
                            member,
                            UncheckableFacet::Skipped,
                        ),
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
        // `other` is a number interval: keep the numeric members it fully admits, and pin a member
        // the leaf admits only outside its integer tokens to the leaf shape carrying that.
        SchemaKind::Number(leaf) => {
            let mut kept = Vec::new();
            let mut partial = Vec::new();
            for member in members {
                match restrict_number_member(leaf.get(), &member, ctx) {
                    MemberRestriction::Full => kept.push(member),
                    MemberRestriction::Empty => {}
                    MemberRestriction::Partial(schema) => partial.push(schema),
                }
            }
            let mut branches = vec![canonicalize_value_set(kept)];
            branches.extend(partial);
            union(branches, ctx)
        }
        // `other` is an array leaf: keep the array members it fully admits, and pin a member an
        // element schema only partially admits to the admitted part of its equality class.
        SchemaKind::Array(leaf) => {
            let mut kept = Vec::new();
            let mut partial = Vec::new();
            for member in members {
                match restrict_array_member(leaf.get(), &member, ctx) {
                    MemberRestriction::Full => kept.push(member),
                    MemberRestriction::Empty => {}
                    MemberRestriction::Partial(schema) => partial.push(schema),
                }
            }
            let mut branches = vec![canonicalize_value_set(kept)];
            branches.extend(partial);
            union(branches, ctx)
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
        | SchemaKind::Not(_)
        | SchemaKind::AllOf(_)
        | SchemaKind::AnyOf(_)
        | SchemaKind::OneOf(_)
        | SchemaKind::Reference(_)
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
    strings: &[(&StringLeaf, StringMatchers)],
    integers: &[IntegerLeaf],
    numbers: &[NumberLeaf],
    arrays: &[ArrayLeaf],
    objects: &[ObjectLeaf],
    member: &CanonicalJson,
    ctx: &CanonicalizationContext,
) -> bool {
    match member.as_value() {
        // Absorbing a member narrows the schema, so only a definite admission absorbs one.
        Value::Array(items) => arrays
            .iter()
            .any(|leaf| matches!(array_leaf_admits(leaf, items, ctx), Verdict::Admits)),
        Value::Object(map) => objects
            .iter()
            .any(|leaf| matches!(object_leaf_admits(leaf, map, ctx), Verdict::Admits)),
        Value::String(_) => strings.iter().any(|(leaf, matchers)| {
            matches!(
                string_leaf_admits(leaf, matchers, member, UncheckableFacet::Undecided),
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
pub(crate) fn string_leaf(mut leaf: StringLeaf, ctx: &CanonicalizationContext) -> Schema {
    if formats_conflict(&leaf) || patterns_conflict(&leaf) {
        return Schema::falsy();
    }
    absorb_empty_exclusion(&mut leaf);
    // No barred-pattern counterpart: a format has a length window to test against, a regex has
    // none, so nothing prunes one that cannot bite.
    prune_excluded_formats(&mut leaf);
    prune_excluded(&mut leaf, ctx);
    let Some(leaf) = NonEmpty::new(leaf) else {
        return Schema::falsy();
    };
    // `maxLength: 0` accepts the empty string and nothing else. A leaf this narrow is spelled as
    // the constant before anything can exclude from it, so exclusions cannot reach here. Nothing
    // prunes a barred pattern against the window, so the collapse gives way to one.
    // e.g.  {"type": "string", "maxLength": 0}  =>  {"const": ""}
    if leaf.get().patterns.is_empty()
        && leaf.get().excluded_patterns.is_empty()
        && leaf.get().formats.is_empty()
        && leaf.get().content_media_types.is_empty()
        && leaf.get().content_encodings.is_empty()
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

/// A complement branch takes every value its own operand rejects, so a sibling conjunction holding
/// that operand says nothing by holding it: `(not A) or (A and B) = (not A) or B`. A conjunction
/// made entirely of covered operands keeps its form, since the union around it is then every value.
/// Reports whether a branch shed anything.
fn drop_conjuncts_a_complement_branch_covers(branches: &mut [Schema]) -> bool {
    let complemented: ahash::AHashSet<Schema> = branches
        .iter()
        .filter_map(|branch| {
            if let SchemaKind::Not(operand) = branch.kind() {
                Some(operand.clone())
            } else {
                None
            }
        })
        .collect();
    if complemented.is_empty() {
        return false;
    }
    let mut shed = false;
    for branch in branches.iter_mut() {
        let SchemaKind::AllOf(conjuncts) = branch.kind() else {
            continue;
        };
        let kept: Vec<Schema> = conjuncts
            .as_slice()
            .iter()
            .filter(|conjunct| !complemented.contains(*conjunct))
            .cloned()
            .collect();
        if kept.is_empty() || kept.len() == conjuncts.as_slice().len() {
            continue;
        }
        *branch = match AtLeastTwo::new(kept) {
            Ok(remaining) => Schema::new(SchemaKind::AllOf(remaining)),
            Err(mut lone) => lone.pop().expect("a non-empty conjunct list"),
        };
        shed = true;
    }
    shed
}

/// How many conjuncts the branches hold between them, counting a branch that is not a conjunction
/// as the single demand it makes.
fn conjuncts_held(branches: &[Schema]) -> usize {
    branches.iter().map(|branch| demands(branch).len()).sum()
}

/// Narrow a property entry spelling several alternatives down to the ones its own branch needs: the
/// values an alternative adds are the branch restricted to it, and a sibling holding all of them
/// makes the alternative say nothing here.
/// ```text
/// e.g.  anyOf [
///         {"type": "object", "properties": {"a": {"$ref": "#/$defs/null"}}},
///         allOf [{"type": "object",
///                 "properties": {"a": {"anyOf": [{"$ref": "#/$defs/integer"},
///                                                {"$ref": "#/$defs/null"}]}}},
///                {"$ref": "#/$defs/integer"}]
///       ]  =>  the second entry keeps only the `integer` alternative
/// ```
fn drop_property_alternatives_covered_by_sibling(
    branches: &mut [Schema],
    ctx: &CanonicalizationContext,
) {
    for index in 0..branches.len() {
        let Some(narrowed) = narrow_branch_entries(branches, index, ctx) else {
            continue;
        };
        branches[index] = narrowed;
    }
}

/// The branch at `index` with every covered alternative dropped, or `None` when it keeps them all.
fn narrow_branch_entries(
    branches: &[Schema],
    index: usize,
    ctx: &CanonicalizationContext,
) -> Option<Schema> {
    let SchemaKind::AllOf(conjuncts) = branches[index].kind() else {
        return None;
    };
    let mut rebuilt = conjuncts.as_slice().to_vec();
    let mut narrowed = false;
    let mut slot = 0;
    while slot < rebuilt.len() {
        let SchemaKind::Object(leaf) = rebuilt[slot].kind() else {
            slot += 1;
            continue;
        };
        let mut replacement = None;
        for (key, entry) in &leaf.get().properties {
            let SchemaKind::AnyOf(alternatives) = entry.kind() else {
                continue;
            };
            let kept: Vec<Schema> = alternatives
                .as_slice()
                .iter()
                .filter(|alternative| {
                    !alternative_is_covered(branches, index, slot, key, alternative, ctx)
                })
                .cloned()
                .collect();
            if kept.len() == alternatives.as_slice().len() {
                continue;
            }
            let mut narrower = leaf.get().clone();
            let merged = union(kept, ctx);
            narrower.properties.insert(Arc::clone(key), merged);
            replacement = Some(narrower);
            break;
        }
        if let Some(narrower) = replacement {
            rebuilt[slot] = object_leaf(narrower, ctx);
            narrowed = true;
        }
        slot += 1;
    }
    if !narrowed {
        return None;
    }
    Some(conjoin(rebuilt, ctx))
}

/// Whether the values one alternative adds - this branch with the entry pinned to it - are all held
/// by a sibling.
fn alternative_is_covered(
    branches: &[Schema],
    index: usize,
    slot: usize,
    key: &Arc<str>,
    alternative: &Schema,
    ctx: &CanonicalizationContext,
) -> bool {
    let mut restricted = demands(&branches[index]).to_vec();
    let SchemaKind::Object(leaf) = restricted[slot].kind() else {
        return false;
    };
    let mut pinned = leaf.get().clone();
    pinned
        .properties
        .insert(Arc::clone(key), alternative.clone());
    restricted[slot] = object_leaf(pinned, ctx);
    let piece = conjoin(restricted, ctx);
    branches
        .iter()
        .enumerate()
        .filter(|(sibling, _)| *sibling != index)
        .any(|(_, sibling)| intersect(piece.clone(), sibling.clone(), ctx) == piece)
}

/// Drop every conjunction a sibling branch already covers: each demand the sibling makes is met by
/// a demand of the conjunction, so the conjunction admits nothing the sibling misses.
/// ```text
/// e.g.  anyOf [
///         allOf [{"type": "object", "required": ["b"], "properties": {"a": false}},
///                {"$ref": "#/$defs/integer"}],
///         {"type": "object", "properties": {"a": false}}
///       ]  =>  {"type": "object", "properties": {"a": false}}
/// e.g.  anyOf [
///         allOf [{"type": "object"}, {"$ref": "#/$defs/integer"}],
///         allOf [{"type": ["object", "string"]}, {"$ref": "#/$defs/integer"}]
///       ]  =>  allOf [{"type": ["object", "string"]}, {"$ref": "#/$defs/integer"}]
/// ```
fn drop_covered_conjunctions(branches: &mut Vec<Schema>, ctx: &CanonicalizationContext) {
    // A branch that is not a conjunction is weighed against its own kind in the leaf pools.
    if !branches
        .iter()
        .any(|branch| matches!(branch.kind(), SchemaKind::AllOf(_)))
    {
        return;
    }
    let mut index = 0;
    while index < branches.len() {
        if conjunction_is_covered(branches, index, ctx) {
            branches.remove(index);
        } else {
            index += 1;
        }
    }
}

/// The values every member admits, built through the algebra so the result stays in normal form.
fn conjoin(members: Vec<Schema>, ctx: &CanonicalizationContext) -> Schema {
    members.into_iter().fold(Schema::truthy(), |held, member| {
        intersect(held, member, ctx)
    })
}

/// The demands a branch makes, which is the branch itself unless it spells several.
fn demands(branch: &Schema) -> &[Schema] {
    if let SchemaKind::AllOf(conjuncts) = branch.kind() {
        conjuncts.as_slice()
    } else {
        std::slice::from_ref(branch)
    }
}

/// Whether the conjunction at `index` has, for every demand of some sibling, a demand of its own
/// that intersecting with it leaves untouched - each of the sibling's demands already met.
fn conjunction_is_covered(
    branches: &[Schema],
    index: usize,
    ctx: &CanonicalizationContext,
) -> bool {
    if !matches!(branches[index].kind(), SchemaKind::AllOf(_)) {
        return false;
    }
    let covered = demands(&branches[index]);
    branches
        .iter()
        .enumerate()
        .filter(|(sibling, _)| *sibling != index)
        .any(|(_, sibling)| {
            demands(sibling).iter().all(|wanted| {
                covered
                    .iter()
                    .any(|held| intersect(held.clone(), wanted.clone(), ctx) == *held)
            })
        })
}

/// The empty string is the only string of its length, so excluding it is the floor above it and
/// both spellings land on one form.
/// ```text
/// e.g.  {"type": "string", "not": {"enum": [""]}}  =>  {"type": "string", "minLength": 1}
/// e.g.  {"type": "string", "not": {"enum": ["a"]}}  =>  unchanged: other lengths hold more strings
/// ```
fn absorb_empty_exclusion(leaf: &mut StringLeaf) {
    if leaf.lengths.minimum.is_some() {
        return;
    }
    let Some(index) = leaf.excluded.iter().position(|value| value.is_empty()) else {
        return;
    };
    leaf.excluded.remove(index);
    leaf.lengths.minimum = Some(BoundCardinality::from(1));
}

/// Drop a barred format no string the leaf admits could match anyway, so one value set keeps one
/// form. A format whose grammar pins a length cannot bite on a window that misses it.
/// e.g.  allOf [
///         {"type": "string", "maxLength": 3},
///         {"not": {"format": "date"}}
///       ]  =>  {"type": "string", "maxLength": 3}
fn prune_excluded_formats(leaf: &mut StringLeaf) {
    if leaf.excluded_formats.is_empty() {
        return;
    }
    let lengths = leaf.lengths.clone();
    leaf.excluded_formats.retain(|format| {
        let Some((minimum, maximum)) = format.length_window() else {
            return true;
        };
        !lengths
            .clone()
            .intersect(LengthBounds {
                minimum: Some(BoundCardinality::from(minimum)),
                maximum: Some(BoundCardinality::from(maximum)),
            })
            .is_empty()
    });
}

/// Drop excluded values the rest of the leaf already rejects, so one value set keeps one form. An
/// undecided verdict keeps the value: dropping one widens the leaf.
fn prune_excluded(leaf: &mut StringLeaf, ctx: &CanonicalizationContext) {
    if leaf.excluded.is_empty() {
        return;
    }
    let matchers = StringMatchers::compile(leaf, ctx);
    let excluded = std::mem::take(&mut leaf.excluded);
    leaf.excluded = excluded
        .into_iter()
        .filter(|value| {
            !matches!(
                string_leaf_admits_text(leaf, &matchers, value, UncheckableFacet::Undecided),
                Verdict::Rejects
            )
        })
        .collect();
}

/// Tighten two integer leaves to the values both admit: the narrower interval and a divisor every
/// value of each must share. `None` when the least common multiple leaves the representable range,
/// which keeps the document unmodeled rather than guessing.
fn intersect_integer_leaves(first: IntegerLeaf, second: IntegerLeaf) -> IntegerLeaf {
    IntegerLeaf {
        bounds: first.bounds.intersect(second.bounds),
        multiple_of: first.multiple_of.intersect(second.multiple_of),
        // Meeting both sets of exclusions is meeting their union.
        not_multiple_of: first.not_multiple_of.intersect(second.not_multiple_of),
    }
}

/// A `Number` node, collapsed to `False` when its interval admits no real value and to the value
/// itself when both ends admit the same one. Unlike `integer`, no draft tells `5` and `5.0` apart on
/// the number domain, so the value needs no type guard.
/// e.g.  {"type": "number", "minimum": 5, "maximum": 5}  =>  {"const": 5}
pub(crate) fn number_leaf(leaf: NumberLeaf, ctx: &CanonicalizationContext) -> Schema {
    // Outside Draft 4 the draft's integers are exactly the multiples of one, so the exclusion
    // respells as a barred divisor and both spellings land on one form.
    let leaf = if leaf.excludes_integers && !matches!(ctx.draft(), Draft::Draft4) {
        NumberLeaf {
            not_multiple_of: leaf
                .not_multiple_of
                .intersect(ExcludedDivisors::one(whole_divisor())),
            excludes_integers: false,
            ..leaf
        }
    } else {
        leaf
    };
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
                    not_multiple_of: leaf.not_multiple_of,
                },
                ctx,
            );
        }
    }
    let Some(leaf) = NonEmpty::new(leaf) else {
        return Schema::falsy();
    };
    if let (Some(min), Some(max)) = (&leaf.get().minimum, &leaf.get().maximum) {
        if min.is_inclusive() && max.is_inclusive() && min.to_number() == max.to_number() {
            let point = min.to_number();
            // A whole point under the exclusion still admits its non-integer tokens, which only
            // the leaf shape can say.
            if leaf.get().excludes_integers && jsonschema_value::types::number_is_integer(&point) {
                return Schema::new(SchemaKind::Number(leaf));
            }
            return if leaf.get().multiple_of.divide(&point)
                && !leaf.get().not_multiple_of.bars(&point)
            {
                Schema::new(SchemaKind::Const(CanonicalJson::from_value(
                    &Value::Number(point),
                )))
            } else {
                Schema::falsy()
            };
        }
    }
    // Paired with the `expect` in `integer_within`, whose leaf always comes from a node built here.
    debug_assert!(
        leaf.get().excludes_integers || integer_bounds_within(leaf.get()).is_some(),
        "a number leaf admitting integers holds ends the integer bounds can spell"
    );
    Schema::new(SchemaKind::Number(leaf))
}

/// Pack an array facet set into a node, collapsing the leaves that say something simpler.
pub(crate) fn array_leaf(mut leaf: ArrayLeaf, ctx: &CanonicalizationContext) -> Schema {
    if !normalize_contains(&mut leaf) {
        return Schema::falsy();
    }
    normalize_items(&mut leaf);
    if !reconcile_contains_window(&mut leaf, ctx) {
        return Schema::falsy();
    }
    if !reconcile_contains_positions(&leaf, ctx) {
        return Schema::falsy();
    }
    match leaf.distinctness {
        Distinctness::Unconstrained => {}
        // Distinct elements cannot outnumber the values they are drawn from, so a finite item
        // domain is a length ceiling.
        // e.g.  {"type": "array", "items": {"type": "boolean"}, "uniqueItems": true}
        //       =>  {"type": "array", "items": {"type": "boolean"}, "uniqueItems": true, "maxItems": 2}
        Distinctness::AllDistinct => {
            // The elements meeting a demand are distinct and all drawn from its own domain, so a
            // demand asking for more matches than that domain holds cannot be met.
            // e.g.  {"type": "array", "contains": {"type": "boolean"}, "minContains": 3, "uniqueItems": true}
            //       =>  false
            if leaf.contains.iter().any(|facet| {
                facet
                    .schema
                    .kind()
                    .finite_domain_size()
                    .is_some_and(|domain| {
                        facet.effective_minimum() > BoundCardinality::from(domain)
                    })
            }) {
                return Schema::falsy();
            }
            if let Some(ceiling) = distinct_length_ceiling(&leaf, ctx) {
                leaf.lengths.maximum = Some(match leaf.lengths.maximum.take() {
                    Some(maximum) => maximum.min(ceiling),
                    None => ceiling,
                });
            }
        }
        // Two elements that coincide are two elements, so the demand floors the length. Spelling
        // that floor is what keeps the demand alone and the demand beside `minItems: 2` together.
        // e.g.  {"type": "array", "allOf": [{"not": {"type": "array", "uniqueItems": true}}]}
        //       =>  {"type": "array", "minItems": 2,
        //            "allOf": [{"not": {"type": "array", "uniqueItems": true}}]}
        Distinctness::SomeRepeated => {
            let floor = BoundCardinality::from(2);
            leaf.lengths.minimum = Some(match leaf.lengths.minimum.take() {
                Some(minimum) => minimum.max(floor),
                None => floor,
            });
        }
    }
    // An array of at most one item holds nothing that can repeat, so a demand for distinct
    // elements says nothing more and a demand for a repeat cannot be met - the latter through the
    // floor above, which leaves such a window empty.
    // e.g.  {"type": "array", "maxItems": 1, "uniqueItems": true}
    //       =>  {"type": "array", "maxItems": 1}
    if leaf
        .lengths
        .maximum
        .as_ref()
        .is_some_and(|max| *max <= BoundCardinality::from(1))
    {
        match leaf.distinctness {
            Distinctness::AllDistinct => leaf.distinctness = Distinctness::Unconstrained,
            Distinctness::SomeRepeated => debug_assert!(
                leaf.lengths.is_empty(),
                "a repeat demand inside a single-item window survived its length floor"
            ),
            Distinctness::Unconstrained => {}
        }
    }
    let Some(leaf) = NonEmpty::new(leaf) else {
        return Schema::falsy();
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
/// so the count they imply must fit under the ceiling, and any item minimum it implies is dropped as
/// redundant.
fn reconcile_contains_window(leaf: &mut ArrayLeaf, ctx: &CanonicalizationContext) -> bool {
    let Some(implied) = implied_length_floor(&leaf.contains, ctx) else {
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
    debug_assert!(
        leaf.lengths
            .minimum
            .as_ref()
            .is_none_or(|min| *min > implied),
        "a length minimum the demands already imply is dropped"
    );
    true
}

/// The shortest array the `contains` demands admit: one element cannot meet two demands sharing no
/// value, so the counts of demands that are pairwise disjoint add up.
/// ```text
/// e.g.  {"type": "array", "contains": {"const": 1}, "allOf": [{"contains": {"const": 2}}]}
///       =>  floor 2
///
///       {"type": "array", "contains": {"type": "integer"}, "allOf": [{"contains": {"const": 1}}]}
///       =>  floor 1
/// ```
fn implied_length_floor(
    demands: &[ContainsFacet],
    ctx: &CanonicalizationContext,
) -> Option<BoundCardinality> {
    // Taking the demands in descending order keeps the widest one, so the floor is never below the
    // largest single count.
    let mut order: Vec<&ContainsFacet> = demands.iter().collect();
    order.sort_by_key(|facet| std::cmp::Reverse(facet.effective_minimum()));
    let mut summed: Vec<&Schema> = Vec::new();
    let mut floor: Option<BoundCardinality> = None;
    for facet in order {
        let minimum = facet.effective_minimum();
        if minimum.is_zero() {
            continue;
        }
        let disjoint = summed.iter().all(|counted| {
            matches!(
                intersect((*counted).clone(), facet.schema.clone(), ctx).kind(),
                SchemaKind::False
            )
        });
        if !disjoint {
            continue;
        }
        floor = match floor {
            // Past the representable range the floor stays where it is, which only understates it.
            Some(current) => current.clone().checked_add(&minimum).or(Some(current)),
            None => Some(minimum),
        };
        summed.push(&facet.schema);
    }
    debug_assert!(
        demands
            .iter()
            .map(ContainsFacet::effective_minimum)
            .max()
            .unwrap_or_default()
            <= floor.clone().unwrap_or_default(),
        "the floor holds at least the largest single demanded count"
    );
    floor
}

/// The longest array `uniqueItems` admits when the tail draws from a finite domain: every element
/// past the prefix comes out of that domain, and a prefix position whose own schema stays inside it
/// competes for the same values instead of contributing one of its own.
/// ```text
/// e.g.  {"prefixItems": [{"const": true}], "items": {"type": "boolean"}, "uniqueItems": true}
///       =>  ceiling 2, not 3
/// ```
fn distinct_length_ceiling(
    leaf: &ArrayLeaf,
    ctx: &CanonicalizationContext,
) -> Option<BoundCardinality> {
    let tail = leaf.items.as_ref()?;
    let domain = tail.kind().finite_domain_size()?;
    let independent = leaf
        .prefix
        .iter()
        .filter(|schema| intersect((*schema).clone(), tail.clone(), ctx) != **schema)
        .count() as u64;
    Some(BoundCardinality::from(domain.saturating_add(independent)))
}

/// Check the `contains` demands against the element schemas: a demand is met only at a position
/// whose own schema shares a value with it, so a demand asking for more matches than there are such
/// positions leaves the leaf empty.
/// ```text
/// e.g.  {"type": "array", "contains": {"type": "integer"}, "items": {"type": "string"}}
///       =>  false
/// ```
fn reconcile_contains_positions(leaf: &ArrayLeaf, ctx: &CanonicalizationContext) -> bool {
    for facet in &leaf.contains {
        let minimum = facet.effective_minimum();
        if minimum.is_zero() {
            continue;
        }
        // Every element past the prefix answers to the tail alone, so once one of those positions
        // can meet the demand, so can any number of them.
        let tail_reachable = leaf
            .lengths
            .maximum
            .as_ref()
            .is_none_or(|max| BoundCardinality::from(leaf.prefix.len() as u64) < *max);
        if tail_reachable {
            let tail = leaf.items.clone().unwrap_or_else(Schema::truthy);
            if !matches!(
                intersect(tail, facet.schema.clone(), ctx).kind(),
                SchemaKind::False
            ) {
                continue;
            }
        }
        let matching = leaf
            .prefix
            .iter()
            .filter(|schema| {
                !matches!(
                    intersect((*schema).clone(), facet.schema.clone(), ctx).kind(),
                    SchemaKind::False
                )
            })
            .count();
        if BoundCardinality::from(matching as u64) < minimum {
            return false;
        }
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
        leaf.prefix.truncate(keep);
    }
    // The tail governs the elements past the prefix, which an array capped at the prefix length
    // has none of.
    // e.g.  {"type": "array", "maxItems": 2, "prefixItems": [A, B], "items": C}
    //       =>  {"type": "array", "maxItems": 2, "prefixItems": [A, B]}
    if unreachable_tail(leaf) {
        leaf.items = None;
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
    debug_assert!(
        !unreachable_tail(leaf),
        "a tail beyond the length ceiling survived normalization"
    );
}

/// Whether the length ceiling leaves no element for the tail to govern.
fn unreachable_tail(leaf: &ArrayLeaf) -> bool {
    leaf.items.is_some()
        && leaf
            .lengths
            .maximum
            .as_ref()
            .is_some_and(|max| *max <= BoundCardinality::from(leaf.prefix.len() as u64))
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

/// Keep the arrays both leaves accept: the narrower window, the distinctness both demand, and
/// elements both leaves admit at every index. `None` when one side demands distinct elements and
/// the other a repeat, which no array does at once.
fn intersect_array_leaves(
    first: ArrayLeaf,
    second: ArrayLeaf,
    ctx: &CanonicalizationContext,
) -> Option<ArrayLeaf> {
    let distinctness = match (first.distinctness, second.distinctness) {
        (Distinctness::Unconstrained, other) | (other, Distinctness::Unconstrained) => other,
        (Distinctness::AllDistinct, Distinctness::AllDistinct) => Distinctness::AllDistinct,
        (Distinctness::SomeRepeated, Distinctness::SomeRepeated) => Distinctness::SomeRepeated,
        (Distinctness::AllDistinct, Distinctness::SomeRepeated)
        | (Distinctness::SomeRepeated, Distinctness::AllDistinct) => return None,
    };
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
    Some(ArrayLeaf {
        lengths: first.lengths.intersect(second.lengths),
        distinctness,
        prefix,
        items,
        contains,
    })
}

/// The schema a leaf places on the element at `index`: its prefix schema there, or the tail once
/// the prefix runs out.
fn element_schema(leaf: &ArrayLeaf, index: usize) -> Option<&Schema> {
    leaf.prefix.get(index).or(leaf.items.as_ref())
}

/// [`element_schema`] with an unconstrained element spelled out.
fn element_constraint(leaf: &ArrayLeaf, index: usize) -> Schema {
    element_schema(leaf, index)
        .cloned()
        .unwrap_or_else(Schema::truthy)
}

/// Whether any two elements are the same value. Members are normalized, so `1` and `1.0` compare
/// equal here just as they do at validation.
fn has_duplicate_elements(elements: &[Value]) -> bool {
    elements
        .iter()
        .enumerate()
        .any(|(index, element)| elements[..index].contains(element))
}

/// Whether the elements sit on the side of coincidence the leaf demands.
fn meets_distinctness(leaf: &ArrayLeaf, elements: &[Value]) -> bool {
    match leaf.distinctness {
        Distinctness::Unconstrained => true,
        Distinctness::AllDistinct => !has_duplicate_elements(elements),
        Distinctness::SomeRepeated => has_duplicate_elements(elements),
    }
}

/// Whether `items` has a length in the window, every element the item schema admits, and the
/// distinctness the leaf asks for.
fn array_leaf_admits(leaf: &ArrayLeaf, items: &[Value], ctx: &CanonicalizationContext) -> Verdict {
    if !leaf
        .lengths
        .contains(&BoundCardinality::from(items.len() as u64))
    {
        return Verdict::Rejects;
    }
    if !meets_distinctness(leaf, items) {
        return Verdict::Rejects;
    }
    contains_verdict(&leaf.contains, items, UncheckableFacet::Undecided, ctx).and(Verdict::all(
        items
            .iter()
            .enumerate()
            .map(|(index, element)| match element_schema(leaf, index) {
                Some(schema) => admits_value(schema, element, UncheckableFacet::Undecided, ctx),
                None => Verdict::Admits,
            }),
    ))
}

/// How the `contains` demands read `elements`. An undecided element leaves the matching count an
/// interval: `definite` counts sure matches, `possible` also the undecided ones. A window missed
/// at both readings rejects; one met only at the right reading stays undecided.
fn contains_verdict(
    facets: &[ContainsFacet],
    elements: &[Value],
    uncheckable: UncheckableFacet,
    ctx: &CanonicalizationContext,
) -> Verdict {
    let mut verdict = Verdict::Admits;
    for facet in facets {
        let mut definite: u64 = 0;
        let mut possible: u64 = 0;
        for element in elements {
            match admits_value(&facet.schema, element, uncheckable, ctx) {
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
            verdict = Verdict::Unknown;
        }
    }
    verdict
}

/// How a leaf restricts a candidate member: kept whole, emptied, or pinned to the part of its
/// equality class the nested schemas admit.
enum MemberRestriction {
    Full,
    Empty,
    Partial(Schema),
}

/// Restrict `member` to the arrays the leaf admits. `Partial` arises when a nested constraint admits
/// only part of the member's equality class or when a symbolic `contains` demand is undecidable.
// e.g.  Draft 4, allOf [
//         {"enum": [[1]]},
//         {"items": {"type": "integer"}}
//       ]  =>  {"type": "array", "items": [{"type": "integer", "enum": [1]}],
//              "minItems": 1, "maxItems": 1}
fn restrict_array_member(
    leaf: &ArrayLeaf,
    member: &CanonicalJson,
    ctx: &CanonicalizationContext,
) -> MemberRestriction {
    let Value::Array(elements) = member.as_value() else {
        return MemberRestriction::Empty;
    };
    if !leaf
        .lengths
        .contains(&BoundCardinality::from(elements.len() as u64))
    {
        return MemberRestriction::Empty;
    }
    if !meets_distinctness(leaf, elements) {
        return MemberRestriction::Empty;
    }
    // Counting the elements of a finite member leaves the demand undecided only across a symbolic
    // reference, which must survive on the member. A facet no checker covers already counted, both
    // toward the floor and toward the ceiling.
    let (mut full, contains) =
        match contains_verdict(&leaf.contains, elements, UncheckableFacet::Skipped, ctx) {
            Verdict::Rejects => return MemberRestriction::Empty,
            Verdict::Unknown => (false, leaf.contains.clone()),
            Verdict::Admits => (true, Vec::new()),
        };
    debug_assert!(
        contains.is_empty()
            || leaf
                .contains
                .iter()
                .any(|facet| contains_reference(&facet.schema)),
        "only reference-bearing contains facets survive an undecidable finite member"
    );
    let mut restricted = Vec::with_capacity(elements.len());
    for (index, element) in elements.iter().enumerate() {
        let pin = Schema::new(SchemaKind::Const(CanonicalJson::from_value(element)));
        let entry = match element_schema(leaf, index) {
            None => pin,
            Some(schema) => {
                let entry = intersect(schema.clone(), pin.clone(), ctx);
                if matches!(entry.kind(), SchemaKind::False) {
                    return MemberRestriction::Empty;
                }
                if entry != pin {
                    full = false;
                }
                entry
            }
        };
        restricted.push(entry);
    }
    if full {
        debug_assert!(
            contains.is_empty(),
            "a fully admitted array has no unresolved contains demand"
        );
        return MemberRestriction::Full;
    }
    let length = BoundCardinality::from(elements.len() as u64);
    MemberRestriction::Partial(array_leaf(
        ArrayLeaf {
            lengths: LengthBounds {
                minimum: Some(length.clone()),
                maximum: Some(length),
            },
            // Element pinning preserves elementwise equality, so the member's own coincidences
            // carry over and the pinned tuple needs no distinctness demand of its own.
            distinctness: Distinctness::Unconstrained,
            prefix: restricted,
            items: None,
            contains,
        },
        ctx,
    ))
}

/// Pack an object facet set into a node, collapsing the leaves that say something simpler.
pub(crate) fn object_leaf(mut leaf: ObjectLeaf, ctx: &CanonicalizationContext) -> Schema {
    normalize_additional(&mut leaf, ctx);
    normalize_property_names(&mut leaf, ctx);
    // A demand no key can break admits nothing; negation never builds one, so reaching here is a
    // constructor bug upstream.
    debug_assert!(
        !leaf.violations.iter().any(|violation| match violation {
            ObjectViolation::NameFails(violated) => matches!(violated.kind(), SchemaKind::True)
                || matches!(violated.kind(), SchemaKind::MultiType(set) if set.contains(JsonType::String)),
            ObjectViolation::UndeclaredValueFails { additional, .. } => {
                matches!(additional.kind(), SchemaKind::True)
            }
        }),
        "a demand no key can break survived construction"
    );
    // Every key must satisfy the constraint, yet a demand needs one that breaks it.
    if let Some(names) = &leaf.property_names {
        for violation in &leaf.violations {
            let ObjectViolation::NameFails(violated) = violation else {
                continue;
            };
            if violated == names {
                return Schema::falsy();
            }
            if let Some(values) = names.kind().finite_values() {
                if values.iter().all(|value| {
                    matches!(value.as_value(), Value::String(key)
                    if matches!(admits_key(violated, key, ctx), Verdict::Admits))
                }) {
                    return Schema::falsy();
                }
            }
        }
    }
    // Every undeclared key's value must satisfy the shield, yet a demand needs one that breaks it.
    if let Some(shield) = &leaf.additional {
        for violation in &leaf.violations {
            if let ObjectViolation::UndeclaredValueFails {
                names,
                patterns,
                additional,
            } = violation
            {
                if additional == shield
                    && leaf.properties.keys().eq(names.iter())
                    && leaf.pattern_properties.keys().eq(patterns.iter())
                {
                    return Schema::falsy();
                }
            }
        }
    }
    // A required key already breaks the schema a `NameFails` demand names, so the key alone
    // supplies the "some key breaks it" the demand needs; keeping the demand spells the same
    // value set twice.
    // e.g.  allOf [{"not": {"type": "object", "propertyNames": {"maxLength": 2}}},
    //              {"type": "object", "required": ["abc"]}]
    //       =>  {"type": "object", "required": ["abc"]}
    let required = &leaf.required;
    leaf.violations.retain(|violation| {
        let ObjectViolation::NameFails(violated) = violation else {
            return true;
        };
        !required
            .iter()
            .any(|key| matches!(admits_key(violated, key, ctx), Verdict::Rejects))
    });
    // A surviving demand needs a key none of the required ones can be, so it needs a key beyond
    // them; the size ceiling then has to leave room for one, or no object can carry the violation.
    // e.g.  {"type": "object", "maxProperties": 1, "minProperties": 1, "required": ["a"],
    //        "properties": {"a": {"type": "string"}},
    //        "not": {"type": "object", "propertyNames": {"enum": ["a", "b"]}}}
    //       =>  {"not": {}}
    if leaf
        .effective_sizes()
        .maximum
        .as_ref()
        .is_some_and(|max| *max <= leaf.required_count())
        && leaf.violations.iter().any(|violation| match violation {
            ObjectViolation::NameFails(violated) => required
                .iter()
                .all(|key| matches!(admits_key(violated, key, ctx), Verdict::Admits)),
            ObjectViolation::UndeclaredValueFails {
                names, patterns, ..
            } => required.iter().all(|key| {
                names.contains(key)
                    || patterns
                        .iter()
                        .any(|pattern| matches_key(pattern, key, ctx))
            }),
        })
    {
        return Schema::falsy();
    }
    expand_additional_over_admitted_keys(&mut leaf);
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
        return Schema::falsy();
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
            return Schema::falsy();
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
        return Schema::falsy();
    };
    // A ceiling of zero present keys accepts the empty object and nothing else, whether spelled as
    // `maxProperties: 0` or as a finite key set whose every key is forbidden; a required key would
    // have emptied the leaf above, and a demand would have collapsed against the slot check, which
    // reads the same ceiling and passes vacuously without required keys.
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
        debug_assert!(
            leaf.get().violations.is_empty(),
            "a demand survived a zero ceiling past the slot check"
        );
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

/// Fold the degenerate shields away: one admitting everything says nothing, and one admitting
/// nothing closes the map, which the key constraint spells.
fn normalize_additional(leaf: &mut ObjectLeaf, ctx: &CanonicalizationContext) {
    let Some(shield) = leaf.additional.take() else {
        return;
    };
    if matches!(shield.kind(), SchemaKind::True) {
        return;
    }
    if matches!(shield.kind(), SchemaKind::False) {
        let allowed = union(
            leaf.properties
                .keys()
                .map(|key| {
                    Schema::new(SchemaKind::Const(CanonicalJson::from_value(
                        &Value::String(key.to_string()),
                    )))
                })
                .collect(),
            ctx,
        );
        leaf.property_names = Some(match leaf.property_names.take() {
            Some(names) => intersect(names, allowed, ctx),
            None => allowed,
        });
        return;
    }
    leaf.additional = Some(shield);
}

/// A finite key constraint leaves no room for unnamed keys beyond its members, so the shield
/// becomes their entries and goes; the two spellings would otherwise name one value set twice.
/// e.g.  {"type": "object", "propertyNames": {"const": "a"}, "additionalProperties": {"type": "integer"}}
///       =>  {"type": "object", "propertyNames": {"const": "a"}, "properties": {"a": {"type": "integer"}}}
fn expand_additional_over_admitted_keys(leaf: &mut ObjectLeaf) {
    if leaf.additional.is_none() {
        return;
    }
    let Some(keys) = admitted_keys(leaf) else {
        return;
    };
    let shield = leaf
        .additional
        .take()
        .expect("the early return proved a shield present");
    for key in keys {
        leaf.properties.or_insert_with(key, || shield.clone());
    }
}

/// Drop the property schemas that say nothing: one accepting every value, and one whose key the
/// key constraint rejects, since that key can never be present to be checked.
fn normalize_properties(leaf: &mut ObjectLeaf, ctx: &CanonicalizationContext) {
    let names = leaf.property_names.clone();
    let shielded = leaf.additional.is_some();
    // A finite key constraint decides every key by membership, and the property map hands the keys
    // over in the order the set is sorted in, so one walk over it settles the whole map.
    let mut admitted = names
        .as_ref()
        .and_then(|names| names.kind().finite_values())
        .map(AscendingMembership::new);
    leaf.properties.retain(|key, schema| {
        // Dropping the entry loses what it says about the key, so only a key the constraint
        // definitely rejects lets the entry go. Under a shield an unconstrained entry still
        // exempts its key, so it stays.
        let named = match (&mut admitted, &names) {
            (Some(admitted), _) => admitted.holds(key),
            (None, Some(names)) => !matches!(admits_key(names, key, ctx), Verdict::Rejects),
            (None, None) => true,
        };
        named && (shielded || !matches!(schema.kind(), SchemaKind::True))
    });
}

/// Fold the pattern map into the facets able to hold what it says: an entry saying nothing goes
/// unless it exempts matching keys from the additional-properties shield; a pattern matching a
/// named key moves onto that key's schema.
fn normalize_pattern_properties(leaf: &mut ObjectLeaf, ctx: &CanonicalizationContext) {
    let shielded = leaf.additional.is_some();
    leaf.pattern_properties
        .retain(|_, schema| shielded || !matches!(schema.kind(), SchemaKind::True));
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
    properties: &mut PropertyMap,
    patterns: &PropertyMap,
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
        .or_else(|| governing_shield(leaf, key, ctx))
        .cloned()
        .unwrap_or_else(Schema::truthy);
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
        SchemaKind::AllOf(branches) => branches
            .as_slice()
            .iter()
            .any(|branch| is_string_domain(branch.kind())),
        // A typed group exists only under Draft 4, which has no `propertyNames`; grouping it here
        // keeps the answer conservative, and narrowing is the identity on any string-domain schema.
        SchemaKind::True
        | SchemaKind::TypedGroup { .. }
        | SchemaKind::Integer(_)
        | SchemaKind::Number(_)
        | SchemaKind::Array(_)
        | SchemaKind::Object(_)
        | SchemaKind::Not(_)
        | SchemaKind::OneOf(_)
        | SchemaKind::Reference(_)
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
        // A key constraint survives on an object leaf, so an undecided facet needs no reading of
        // its own here: the leaf that keeps it hands it to the validator.
        SchemaKind::String(leaf) => {
            let matchers = StringMatchers::compile(leaf.get(), ctx);
            string_leaf_admits_text(leaf.get(), &matchers, key, UncheckableFacet::Undecided)
        }
        SchemaKind::AnyOf(branches) => Verdict::any(
            branches
                .as_slice()
                .iter()
                .map(|branch| admits_key(branch, key, ctx)),
        ),
        SchemaKind::AllOf(branches) => Verdict::all(
            branches
                .as_slice()
                .iter()
                .map(|branch| admits_key(branch, key, ctx)),
        ),
        SchemaKind::Not(_) | SchemaKind::OneOf(_) | SchemaKind::Reference(_) => Verdict::Unknown,
        // An opaque conjunct keeps the narrowing intersection from folding into a string leaf, so
        // the type set it introduced stays a branch of its own.
        SchemaKind::MultiType(set) => Verdict::from_bool(set.contains(JsonType::String)),
        // Normalization stores the rest of a key constraint as a string value set, a string leaf,
        // or a union of those: everything else was narrowed or folded away.
        SchemaKind::TypedGroup { .. }
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
pub(crate) fn admits_value(
    schema: &Schema,
    value: &Value,
    uncheckable: UncheckableFacet,
    ctx: &CanonicalizationContext,
) -> Verdict {
    if contains_reference(schema) {
        return Verdict::Unknown;
    }
    let member = Schema::new(SchemaKind::Const(CanonicalJson::from_value(value)));
    // Non-`False` is not enough: under Draft 4 the intersection can pin a nested whole number to
    // its integer spelling (a typed group), a strict subset of the member's equality class - the
    // member `1` also matches `1.0`, which an integer-typed property schema rejects.
    if intersect(schema.clone(), member.clone(), ctx) != member {
        return Verdict::Rejects;
    }
    // Intersection reads a facet no checker covers the way a validator without one does, so its
    // "yes" is definite only when the schema carries none.
    if matches!(uncheckable, UncheckableFacet::Undecided) && has_uncheckable_string_facet(schema) {
        return Verdict::Unknown;
    }
    Verdict::Admits
}

pub(crate) fn contains_reference(schema: &Schema) -> bool {
    match schema.kind() {
        SchemaKind::Reference(_) => true,
        SchemaKind::Not(inner) | SchemaKind::TypedGroup { body: inner, .. } => {
            contains_reference(inner)
        }
        SchemaKind::AllOf(branches) | SchemaKind::AnyOf(branches) => {
            for branch in branches.as_slice() {
                if contains_reference(branch) {
                    return true;
                }
            }
            false
        }
        SchemaKind::OneOf(branches) => {
            for branch in branches {
                if contains_reference(branch) {
                    return true;
                }
            }
            false
        }
        SchemaKind::Array(leaf) => {
            let leaf = leaf.get();
            for schema in &leaf.prefix {
                if contains_reference(schema) {
                    return true;
                }
            }
            if let Some(schema) = &leaf.items {
                if contains_reference(schema) {
                    return true;
                }
            }
            for facet in &leaf.contains {
                if contains_reference(&facet.schema) {
                    return true;
                }
            }
            false
        }
        SchemaKind::Object(leaf) => {
            let leaf = leaf.get();
            if let Some(schema) = &leaf.property_names {
                if contains_reference(schema) {
                    return true;
                }
            }
            for schema in leaf.properties.values() {
                if contains_reference(schema) {
                    return true;
                }
            }
            for schema in leaf.pattern_properties.values() {
                if contains_reference(schema) {
                    return true;
                }
            }
            if let Some(schema) = &leaf.additional {
                if contains_reference(schema) {
                    return true;
                }
            }
            leaf.violations.iter().any(|violation| match violation {
                ObjectViolation::NameFails(schema) => contains_reference(schema),
                ObjectViolation::UndeclaredValueFails { additional, .. } => {
                    contains_reference(additional)
                }
            })
        }
        SchemaKind::MultiType(_)
        | SchemaKind::String(_)
        | SchemaKind::Integer(_)
        | SchemaKind::Number(_)
        | SchemaKind::Const(_)
        | SchemaKind::Enum(_)
        | SchemaKind::True
        | SchemaKind::False
        | SchemaKind::Raw(_) => false,
    }
}

/// Whether `schema` demands or bars a format, media type, or encoding this draft has no checker for.
fn has_uncheckable_string_facet(schema: &Schema) -> bool {
    match schema.kind() {
        SchemaKind::String(leaf) => {
            leaf.get()
                .formats
                .iter()
                .chain(leaf.get().excluded_formats.iter())
                .any(|format| format.is_valid("").is_none())
                || leaf
                    .get()
                    .content_media_types
                    .iter()
                    .any(|media_type| !is_known_content_media_type(media_type))
                || leaf
                    .get()
                    .content_encodings
                    .iter()
                    .any(|encoding| !is_known_content_encoding(encoding))
        }
        SchemaKind::AnyOf(branches) => branches.as_slice().iter().any(has_uncheckable_string_facet),
        // A conjunction and a complement both carry a reference, and the only caller declines a
        // schema holding one before asking about its facets.
        SchemaKind::AllOf(_) | SchemaKind::OneOf(_) | SchemaKind::Not(_) => {
            unreachable!("a symbolic branch never reaches the facet scan")
        }
        SchemaKind::Object(leaf) => {
            leaf.get()
                .property_names
                .iter()
                .chain(leaf.get().properties.values())
                .chain(leaf.get().pattern_properties.values())
                .chain(leaf.get().additional.iter())
                .any(has_uncheckable_string_facet)
                || leaf
                    .get()
                    .violations
                    .iter()
                    .any(|violation| match violation {
                        ObjectViolation::NameFails(schema) => has_uncheckable_string_facet(schema),
                        ObjectViolation::UndeclaredValueFails { additional, .. } => {
                            has_uncheckable_string_facet(additional)
                        }
                    })
        }
        SchemaKind::Array(leaf) => leaf
            .get()
            .prefix
            .iter()
            .chain(leaf.get().items.iter())
            .chain(leaf.get().contains.iter().map(|facet| &facet.schema))
            .any(has_uncheckable_string_facet),

        // A typed group's body is a value set, which carries no format or content check.
        SchemaKind::TypedGroup { .. }
        | SchemaKind::MultiType(_)
        | SchemaKind::Integer(_)
        | SchemaKind::Number(_)
        | SchemaKind::Const(_)
        | SchemaKind::Enum(_)
        | SchemaKind::Reference(_)
        | SchemaKind::True
        | SchemaKind::False
        | SchemaKind::Raw(_) => false,
    }
}

fn is_known_content_media_type(media_type: &str) -> bool {
    crate::content_media_type::DEFAULT_CONTENT_MEDIA_TYPE_CHECKS.contains_key(media_type)
}

fn is_known_content_encoding(encoding: &str) -> bool {
    crate::content_encoding::DEFAULT_CONTENT_ENCODING_CHECKS_AND_CONVERTERS.contains_key(encoding)
}

/// Keep the objects both leaves accept: the narrower window, and every key either demands.
fn intersect_object_leaves(
    first: ObjectLeaf,
    second: ObjectLeaf,
    ctx: &CanonicalizationContext,
) -> ObjectLeaf {
    let properties = intersect_property_entries(&first, &second, ctx);
    let pattern_properties = intersect_pattern_entries(&first, &second, ctx);
    if !spells_shielded_meet(&first, &second, &properties, ctx) {
        ctx.record_unspellable_meet();
    }
    let mut required = first.required;
    required.extend(second.required);
    required.sort();
    required.dedup();
    let property_names = match (first.property_names, second.property_names) {
        (Some(left), Some(right)) => Some(intersect(left, right, ctx)),
        (names, None) | (None, names) => names,
    };
    let additional = match (first.additional, second.additional) {
        (Some(left), Some(right)) => Some(intersect(left, right, ctx)),
        (shield, None) | (None, shield) => shield,
    };
    let mut violations = first.violations;
    violations.extend(second.violations);
    violations.sort();
    violations.dedup();
    ObjectLeaf {
        sizes: first.sizes.intersect(second.sizes),
        required,
        property_names,
        properties,
        pattern_properties,
        additional,
        violations,
    }
}

/// What one leaf's `additionalProperties` demands of `key`, which is nothing unless that leaf
/// leaves the key to the shield - naming it or matching it with a pattern takes it away.
fn governing_shield<'leaf>(
    leaf: &'leaf ObjectLeaf,
    key: &str,
    ctx: &CanonicalizationContext,
) -> Option<&'leaf Schema> {
    let shield = leaf.additional.as_ref()?;
    if leaf.properties.contains_key(key) {
        return None;
    }
    (!leaf
        .pattern_properties
        .keys()
        .any(|pattern| matches_key(pattern, key, ctx)))
    .then_some(shield)
}

/// The entry for every key either leaf names, meeting what both sides demand of it: the stored
/// entry where the side names the key, and the side's shield where it leaves the key to one.
fn intersect_property_entries(
    first: &ObjectLeaf,
    second: &ObjectLeaf,
    ctx: &CanonicalizationContext,
) -> PropertyMap {
    let mut keys: Vec<&Arc<str>> = first
        .properties
        .keys()
        .chain(second.properties.keys())
        .collect();
    keys.sort();
    keys.dedup();
    let mut entries = PropertyMap::default();
    for key in keys {
        let entry = [
            first.properties.get(key),
            governing_shield(first, key, ctx),
            second.properties.get(key),
            governing_shield(second, key, ctx),
        ]
        .into_iter()
        .flatten()
        .cloned()
        .reduce(|held, applicable| intersect(held, applicable, ctx))
        .expect("a key of the union is named by one of the two property maps");
        entries.insert(Arc::clone(key), entry);
    }
    entries
}

/// The pattern entries of both leaves, met where they share a pattern. A side carrying no pattern
/// of its own sends every key the other side's patterns match to its shield, so that shield meets
/// each of those entries.
fn intersect_pattern_entries(
    first: &ObjectLeaf,
    second: &ObjectLeaf,
    ctx: &CanonicalizationContext,
) -> PropertyMap {
    let mut entries = first.pattern_properties.clone();
    for (pattern, schema) in &second.pattern_properties {
        let entry = match entries.remove(pattern) {
            Some(existing) => intersect(existing, schema.clone(), ctx),
            None => schema.clone(),
        };
        entries.insert(Arc::clone(pattern), entry);
    }
    let shield = match (
        first.pattern_properties.is_empty(),
        second.pattern_properties.is_empty(),
    ) {
        (true, false) => first.additional.as_ref(),
        (false, true) => second.additional.as_ref(),
        (true, true) | (false, false) => None,
    };
    if let Some(shield) = shield {
        for entry in entries.values_mut() {
            *entry = intersect(entry.clone(), shield.clone(), ctx);
        }
    }
    entries
}

/// Whether the merged entries say exactly what both leaves demand of every key.
///
/// Two pattern maps beside a shield would need to know which keys the patterns share, since a key
/// only one map matches answers to the other map's shield and a key both match answers to neither.
/// A key the shield's own side names is outside that shield, so an entry the shield already
/// admits keeps the pattern it was met into faithful and anything narrower does not.
fn spells_shielded_meet(
    first: &ObjectLeaf,
    second: &ObjectLeaf,
    properties: &PropertyMap,
    ctx: &CanonicalizationContext,
) -> bool {
    if !first.pattern_properties.is_empty() && !second.pattern_properties.is_empty() {
        // Maps naming the same patterns match the same keys, so no key reaches one map's shield
        // without reaching the other's, and the entries carry both sides for every key either
        // matches. Shields on neither side leave nothing to place in the first place.
        let same_keys = first
            .pattern_properties
            .keys()
            .eq(second.pattern_properties.keys());
        return same_keys || (first.additional.is_none() && second.additional.is_none());
    }
    shield_spares_named_keys(first, second, properties, ctx)
        && shield_spares_named_keys(second, first, properties, ctx)
}

/// Whether `shielded`'s shield, met into every pattern entry `patterned` carries, leaves the keys
/// `shielded` names as they were.
fn shield_spares_named_keys(
    shielded: &ObjectLeaf,
    patterned: &ObjectLeaf,
    properties: &PropertyMap,
    ctx: &CanonicalizationContext,
) -> bool {
    let Some(shield) = &shielded.additional else {
        return true;
    };
    if patterned.pattern_properties.is_empty() {
        return true;
    }
    shielded.properties.keys().all(|key| {
        !patterned
            .pattern_properties
            .keys()
            .any(|pattern| matches_key(pattern, key, ctx))
            || properties
                .get(key)
                .is_some_and(|entry| oracle::covers(entry, shield, ctx) == Verdict::Admits)
    })
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
    let mut restricted_property_names = None;
    if let Some(names) = &leaf.property_names {
        for key in map.keys() {
            match admits_key(names, key, ctx) {
                Verdict::Admits => {}
                Verdict::Rejects => return MemberRestriction::Empty,
                Verdict::Unknown => restricted_property_names = Some(names.clone()),
            }
        }
    }
    let mut restricted_violations = Vec::new();
    for violation in &leaf.violations {
        match violation {
            ObjectViolation::NameFails(violated) => {
                let mut satisfied = Verdict::Rejects;
                for key in map.keys() {
                    match admits_key(violated, key, ctx) {
                        Verdict::Rejects => {
                            satisfied = Verdict::Admits;
                            break;
                        }
                        Verdict::Unknown => satisfied = Verdict::Unknown,
                        Verdict::Admits => {}
                    }
                }
                match satisfied {
                    Verdict::Admits => {}
                    Verdict::Rejects => return MemberRestriction::Empty,
                    Verdict::Unknown => {
                        restricted_violations.push(ObjectViolation::NameFails(violated.clone()));
                    }
                }
            }
            ObjectViolation::UndeclaredValueFails {
                names,
                patterns,
                additional,
            } => {
                let mut satisfied = Verdict::Rejects;
                for (key, value) in map {
                    if names.iter().any(|name| name.as_ref() == key.as_str())
                        || patterns
                            .iter()
                            .any(|pattern| matches_key(pattern, key, ctx))
                    {
                        continue;
                    }
                    match admits_value(additional, value, UncheckableFacet::Undecided, ctx) {
                        Verdict::Rejects => {
                            satisfied = Verdict::Admits;
                            break;
                        }
                        Verdict::Unknown => satisfied = Verdict::Unknown,
                        Verdict::Admits => {}
                    }
                }
                match satisfied {
                    Verdict::Admits => {}
                    Verdict::Rejects => return MemberRestriction::Empty,
                    Verdict::Unknown => {
                        restricted_violations.push(ObjectViolation::UndeclaredValueFails {
                            names: names.clone(),
                            patterns: patterns.clone(),
                            additional: additional.clone(),
                        });
                    }
                }
            }
        }
    }
    let mut full = restricted_property_names.is_none() && restricted_violations.is_empty();
    let mut restricted = PropertyMap::default();
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
            property_names: restricted_property_names,
            properties: restricted,
            pattern_properties: PropertyMap::default(),
            additional: None,
            violations: restricted_violations,
        },
        ctx,
    ))
}

/// Whether `map` carries every required key, every key admitted by the key constraint, and a
/// property count in the window.
fn object_leaf_admits(
    leaf: &ObjectLeaf,
    map: &serde_json::Map<String, Value>,
    ctx: &CanonicalizationContext,
) -> Verdict {
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
    let violations = Verdict::all(leaf.violations.iter().map(|violation| match violation {
        ObjectViolation::NameFails(violated) => {
            let mut satisfied = Verdict::Rejects;
            for key in map.keys() {
                match admits_key(violated, key, ctx) {
                    Verdict::Rejects => return Verdict::Admits,
                    Verdict::Unknown => satisfied = Verdict::Unknown,
                    Verdict::Admits => {}
                }
            }
            satisfied
        }
        ObjectViolation::UndeclaredValueFails {
            names,
            patterns,
            additional,
        } => {
            let mut satisfied = Verdict::Rejects;
            for (key, value) in map {
                if names.iter().any(|name| name.as_ref() == key.as_str())
                    || patterns
                        .iter()
                        .any(|pattern| matches_key(pattern, key, ctx))
                {
                    continue;
                }
                match admits_value(additional, value, UncheckableFacet::Undecided, ctx) {
                    Verdict::Rejects => return Verdict::Admits,
                    Verdict::Unknown => satisfied = Verdict::Unknown,
                    Verdict::Admits => {}
                }
            }
            satisfied
        }
    }));
    if violations == Verdict::Rejects {
        return Verdict::Rejects;
    }
    let values = Verdict::all(map.iter().map(|(key, value)| {
        let named = match (
            leaf.properties.get(key.as_str()),
            governing_shield(leaf, key, ctx),
        ) {
            (Some(schema), _) => admits_value(schema, value, UncheckableFacet::Undecided, ctx),
            (None, Some(shield)) => admits_value(shield, value, UncheckableFacet::Undecided, ctx),
            (None, None) => Verdict::Admits,
        };
        if named == Verdict::Rejects {
            return Verdict::Rejects;
        }
        named.and(Verdict::all(leaf.pattern_properties.iter().map(
            |(pattern, schema)| {
                if matches_key(pattern, key, ctx) {
                    admits_value(schema, value, UncheckableFacet::Undecided, ctx)
                } else {
                    Verdict::Admits
                }
            },
        )))
    }));
    keys.and(values).and(violations)
}

/// The number leaf admitting exactly the values both admit.
fn intersect_number_leaves(first: NumberLeaf, second: NumberLeaf) -> NumberLeaf {
    NumberLeaf {
        minimum: tightest(first.minimum, second.minimum, Side::Lower),
        maximum: tightest(first.maximum, second.maximum, Side::Upper),
        // Meeting both sets of divisors is meeting their union, and likewise the exclusions.
        multiple_of: first.multiple_of.intersect(second.multiple_of),
        not_multiple_of: first.not_multiple_of.intersect(second.not_multiple_of),
        excludes_integers: first.excludes_integers || second.excludes_integers,
    }
}

/// The divisor every whole-valued number is a multiple of.
fn whole_divisor() -> BoundRational {
    BoundRational::new(&serde_json::Number::from(1)).expect("one is a representable divisor")
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
        not_multiple_of: leaf.not_multiple_of,
        excludes_integers: leaf.excludes_integers,
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
    if leaf.excludes_integers {
        return Schema::falsy();
    }
    let bounds = integer_bounds_within(leaf)
        .expect("a number leaf admitting integers holds ends the integer bounds can spell");
    integer_leaf(
        IntegerLeaf {
            bounds,
            multiple_of: leaf.multiple_of.clone(),
            not_multiple_of: leaf.not_multiple_of.clone(),
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

/// Whether the interval admits every integer the window does. Dropping the window narrows the union,
/// so a divisor the exact arithmetic cannot compare leaves the two apart.
fn number_leaf_covers_integer_leaf(interval: &NumberLeaf, window: &IntegerLeaf) -> bool {
    // An interval barring the draft's integers holds none of them, whatever the window spans.
    if interval.excludes_integers {
        return false;
    }
    // Ends past the representable range leave no integer bounds to compare against.
    let Some(reach) = integer_bounds_within(interval) else {
        return false;
    };
    reach.covers(&window.bounds)
        // The divisors every integer already meets leave no work; the window's own must imply the
        // rest.
        && interval
            .multiple_of
            .clone()
            .over_integers()
            .divide_all(&window.multiple_of)
        && interval
            .not_multiple_of
            .bars_no_more_than(&window.not_multiple_of)
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
        && !leaf.not_multiple_of.bars(number)
        && !(leaf.excludes_integers && jsonschema_value::types::number_is_integer(number))
}

/// Restrict `member` to the numbers the leaf admits. `Partial` arises under Draft 4, where a
/// whole member barred as an integer keeps its float tokens and only the leaf shape carries them.
fn restrict_number_member(
    leaf: &NumberLeaf,
    member: &CanonicalJson,
    ctx: &CanonicalizationContext,
) -> MemberRestriction {
    if number_leaf_admits(leaf, member) {
        return MemberRestriction::Full;
    }
    let Value::Number(number) = member.as_value() else {
        return MemberRestriction::Empty;
    };
    if !(leaf.excludes_integers && jsonschema_value::types::number_is_integer(number)) {
        return MemberRestriction::Empty;
    }
    // The member's whole point, narrowed by every facet of the leaf — a bound the point misses
    // empties the window rather than being replaced by it.
    let point = BoundNumber::new(number, true);
    let window = intersect_number_leaves(
        NumberLeaf {
            minimum: Some(point.clone()),
            maximum: Some(point),
            multiple_of: Divisors::default(),
            not_multiple_of: ExcludedDivisors::default(),
            excludes_integers: false,
        },
        leaf.clone(),
    );
    debug_assert!(
        matches!(ctx.draft(), Draft::Draft4),
        "the integer exclusion survives normalization only under Draft 4"
    );
    let restricted = number_leaf(window, ctx);
    if matches!(restricted.kind(), SchemaKind::False) {
        MemberRestriction::Empty
    } else {
        MemberRestriction::Partial(restricted)
    }
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
    if leaf.bounds.minimum.is_none()
        && leaf.bounds.maximum.is_none()
        && leaf.multiple_of.is_empty()
        && leaf.not_multiple_of.is_empty()
    {
        return type_set_schema(JsonTypeSet::from(JsonType::Integer));
    }
    let Some(leaf) = snap_to_multiples(leaf).and_then(NonEmpty::new) else {
        return Schema::falsy();
    };
    if let (Some(min), Some(max)) = (&leaf.get().bounds.minimum, &leaf.get().bounds.maximum) {
        if min == max {
            let point = min.to_number();
            // Only a divisor snapping could not pull onto the progression is left to check here.
            if !leaf.get().multiple_of.divide(&point) || leaf.get().not_multiple_of.bars(&point) {
                return Schema::falsy();
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
        not_multiple_of: leaf.not_multiple_of,
    })
}

/// Whether `member` is an integer value within `bounds`.
fn integer_leaf_admits(leaf: &IntegerLeaf, member: &CanonicalJson) -> bool {
    let Value::Number(number) = member.as_value() else {
        return false;
    };
    if leaf.not_multiple_of.bars(number) {
        return false;
    }
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
    let mut excluded_patterns = first.excluded_patterns;
    excluded_patterns.extend(second.excluded_patterns);
    excluded_patterns.sort();
    excluded_patterns.dedup();
    let mut formats = first.formats;
    formats.extend(second.formats);
    formats.sort();
    formats.dedup();
    let mut excluded_formats = first.excluded_formats;
    excluded_formats.extend(second.excluded_formats);
    excluded_formats.sort();
    excluded_formats.dedup();
    let mut content_media_types = first.content_media_types;
    content_media_types.extend(second.content_media_types);
    content_media_types.sort();
    content_media_types.dedup();
    let mut content_encodings = first.content_encodings;
    content_encodings.extend(second.content_encodings);
    content_encodings.sort();
    content_encodings.dedup();
    let mut excluded = first.excluded;
    excluded.extend(second.excluded);
    excluded.sort();
    excluded.dedup();
    StringLeaf {
        lengths: first.lengths.intersect(second.lengths),
        patterns,
        excluded_patterns,
        formats,
        excluded_formats,
        content_media_types,
        content_encodings,
        excluded,
    }
}

/// Whether the leaf's formats and length window leave no string. A format whose grammar pins a
/// length narrows the window; two such formats of different lengths admit nothing.
/// e.g.  allOf [
///         {"type": "string", "format": "date"},
///         {"type": "string", "format": "uuid"}
///       ]  =>  false
fn formats_conflict(leaf: &StringLeaf) -> bool {
    if leaf
        .excluded_formats
        .iter()
        .any(|format| leaf.formats.contains(format))
    {
        return true;
    }
    let mut window = leaf.lengths.clone();
    for format in &leaf.formats {
        let Some((minimum, maximum)) = format.length_window() else {
            continue;
        };
        window = window.intersect(LengthBounds {
            minimum: Some(BoundCardinality::from(minimum)),
            maximum: Some(BoundCardinality::from(maximum)),
        });
    }
    window.is_empty()
}

/// Whether the leaf both demands and bars one pattern, which no string can satisfy. Syntactic, so
/// `^a` against `^a.*` is not caught - see [`StringLeaf::excluded_patterns`].
/// e.g.  allOf [
///         {"type": "string", "pattern": "^a"},
///         {"not": {"pattern": "^a"}}
///       ]  =>  false
fn patterns_conflict(leaf: &StringLeaf) -> bool {
    leaf.excluded_patterns
        .iter()
        .any(|pattern| leaf.patterns.contains(pattern))
}

/// A leaf's compiled patterns, built once so a scan over many members compiles nothing per member.
struct StringMatchers {
    required: Vec<Arc<CompiledMatcher>>,
    barred: Vec<Arc<CompiledMatcher>>,
}

impl StringMatchers {
    fn compile(leaf: &StringLeaf, ctx: &CanonicalizationContext) -> Self {
        let compile_all = |patterns: &[Arc<str>]| {
            patterns
                .iter()
                .map(|pattern| {
                    ctx.compile_regex(pattern)
                        .expect("pattern validated during parsing")
                })
                .collect()
        };
        Self {
            required: compile_all(&leaf.patterns),
            barred: compile_all(&leaf.excluded_patterns),
        }
    }
}

/// Whether the string `member` falls within the leaf's length window and matches every required
/// pattern and no barred one.
fn string_leaf_admits(
    leaf: &StringLeaf,
    matchers: &StringMatchers,
    member: &CanonicalJson,
    uncheckable: UncheckableFacet,
) -> Verdict {
    let Value::String(text) = member.as_value() else {
        return Verdict::Rejects;
    };
    string_leaf_admits_text(leaf, matchers, text, uncheckable)
}

/// Whether `text` falls within the leaf's length window, matches every required pattern and no
/// barred one, and meets every format, media type, and encoding.
fn string_leaf_admits_text(
    leaf: &StringLeaf,
    matchers: &StringMatchers,
    text: &str,
    uncheckable: UncheckableFacet,
) -> Verdict {
    let length = BoundCardinality::from(bytecount::num_chars(text.as_bytes()) as u64);
    if !leaf.lengths.contains(&length)
        || !matchers.required.iter().all(|regex| regex.is_match(text))
        || matchers.barred.iter().any(|regex| regex.is_match(text))
        || leaf.excluded.iter().any(|value| value.as_ref() == text)
    {
        return Verdict::Rejects;
    }
    // A checker that is not there admits every string it was asked about and so meets a demand and
    // breaks a bar, which is why the two resolve to opposite verdicts.
    let demanded = |checked: Option<bool>| match (checked, uncheckable) {
        (Some(admitted), _) => Verdict::from_bool(admitted),
        (None, UncheckableFacet::Skipped) => Verdict::Admits,
        (None, UncheckableFacet::Undecided) => Verdict::Unknown,
    };
    let barred = |checked: Option<bool>| match (checked, uncheckable) {
        (Some(admitted), _) => Verdict::from_bool(!admitted),
        (None, UncheckableFacet::Skipped) => Verdict::Rejects,
        (None, UncheckableFacet::Undecided) => Verdict::Unknown,
    };
    Verdict::all(
        leaf.formats
            .iter()
            .map(|format| demanded(format.is_valid(text)))
            .chain(
                leaf.excluded_formats
                    .iter()
                    .map(|format| barred(format.is_valid(text))),
            )
            .chain(leaf.content_media_types.iter().map(|media_type| {
                demanded(
                    crate::content_media_type::DEFAULT_CONTENT_MEDIA_TYPE_CHECKS
                        .get(media_type.as_ref())
                        .map(|check| check(text)),
                )
            }))
            .chain(leaf.content_encodings.iter().map(|encoding| {
                demanded(
                    crate::content_encoding::DEFAULT_CONTENT_ENCODING_CHECKS_AND_CONVERTERS
                        .get(encoding.as_ref())
                        .map(|(check, _)| check(text)),
                )
            })),
    )
}
