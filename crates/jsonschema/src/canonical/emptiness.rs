//! Deciding which `$ref` targets denote the empty set, and which constrain nothing at all.
//!
//! `$ref` stays symbolic, so what sits behind one is invisible to leaf normalization. Settling a
//! target lets `parse` substitute `False` or `True` and the existing pipeline fold.

use std::{collections::BTreeSet, sync::Arc};

use ahash::{AHashMap, AHashSet};

use referencing::Resolver;
use serde_json::Value;

use crate::canonical::{
    context::CanonicalizationContext,
    ir::{BoundCardinality, ObjectViolation, Schema, SchemaKind},
    parse::{self, Assumptions, ParseOutput},
    schema::DefinitionMap,
    CanonicalizationError, ROOT_DEFINITION_KEY,
};

/// Where a reference sits relative to the instance being validated.
///
/// A purely [`Position::InPlace`] cycle is ill-founded - the validator accepts every value against
/// it - so it must never be assumed empty.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Position {
    /// Evaluated against the same instance: `allOf`, `anyOf`, `oneOf`, `not`, a bare `$ref`.
    InPlace,
    /// Evaluated against a proper sub-value: a property value, an array element, a property name.
    Consuming,
}

/// Every reference reachable from `schema`, paired with the position it occupies.
///
/// `position` is `schema`'s own; once a walk enters a leaf's children it stays
/// [`Position::Consuming`].
pub(crate) fn collect_classified_references<'a>(
    schema: &'a Schema,
    position: Position,
    out: &mut Vec<(&'a Arc<str>, Position)>,
) {
    match schema.kind() {
        SchemaKind::Reference(uri) => out.push((uri, position)),
        SchemaKind::Not(inner) | SchemaKind::TypedGroup { body: inner, .. } => {
            collect_classified_references(inner, position, out);
        }
        SchemaKind::AllOf(branches) | SchemaKind::AnyOf(branches) => {
            for branch in branches.as_slice() {
                collect_classified_references(branch, position, out);
            }
        }
        SchemaKind::OneOf(branches) => {
            for branch in branches {
                collect_classified_references(branch, position, out);
            }
        }
        SchemaKind::Array(leaf) => {
            let leaf = leaf.get();
            for schema in &leaf.prefix {
                collect_classified_references(schema, Position::Consuming, out);
            }
            if let Some(schema) = &leaf.items {
                collect_classified_references(schema, Position::Consuming, out);
            }
            for facet in &leaf.contains {
                collect_classified_references(&facet.schema, Position::Consuming, out);
            }
        }
        SchemaKind::Object(leaf) => {
            let leaf = leaf.get();
            // A property name is smaller than the object holding it, so it consumes structure.
            if let Some(schema) = &leaf.property_names {
                collect_classified_references(schema, Position::Consuming, out);
            }
            for schema in leaf.properties.values() {
                collect_classified_references(schema, Position::Consuming, out);
            }
            for schema in leaf.pattern_properties.values() {
                collect_classified_references(schema, Position::Consuming, out);
            }
            if let Some(schema) = &leaf.additional {
                collect_classified_references(schema, Position::Consuming, out);
            }
            for violation in &leaf.violations {
                match violation {
                    ObjectViolation::NameFails(schema) => {
                        collect_classified_references(schema, Position::Consuming, out);
                    }
                    ObjectViolation::UndeclaredValueFails { additional, .. } => {
                        collect_classified_references(additional, Position::Consuming, out);
                    }
                }
            }
        }
        SchemaKind::MultiType(_)
        | SchemaKind::String(_)
        | SchemaKind::Integer(_)
        | SchemaKind::Number(_)
        | SchemaKind::Const(_)
        | SchemaKind::Enum(_)
        | SchemaKind::True
        | SchemaKind::False
        | SchemaKind::Raw(_) => {}
    }
}

/// The definition keys `node` reaches, walked over the map itself.
///
/// `document_root` is followed where the walk meets a pointer at the document, which is how a node
/// below that root reaches what the root names; a walk starting at the root itself passes `None`.
pub(crate) fn reachable_definition_keys(
    node: &Schema,
    document_root: Option<&Schema>,
    definitions: &DefinitionMap,
) -> AHashSet<Arc<str>> {
    let mut pending = Vec::new();
    collect_definition_references(node, &mut pending);
    let mut reachable = AHashSet::new();
    let mut followed_document = false;
    while let Some(uri) = pending.pop() {
        if uri == ROOT_DEFINITION_KEY {
            // Following it once is enough - the root's own pointers join the same worklist.
            if let Some(root) = document_root {
                if !std::mem::replace(&mut followed_document, true) {
                    collect_definition_references(root, &mut pending);
                }
            }
            continue;
        }
        let Some((uri, schema)) = definitions.get_key_value(uri) else {
            continue;
        };
        if reachable.insert(Arc::clone(uri)) {
            collect_definition_references(schema, &mut pending);
        }
    }
    reachable
}

/// Every pointer `schema` spells, position discarded.
///
/// Derived from the classifying walker rather than repeated: the two must agree on which fields
/// hold a schema, and a field missed here leaks a `$ref` to a definition nothing kept.
fn collect_definition_references<'a>(schema: &'a Schema, references: &mut Vec<&'a str>) {
    let mut found = Vec::new();
    collect_classified_references(schema, Position::InPlace, &mut found);
    references.extend(found.into_iter().map(|(uri, _)| uri.as_ref()));
}

/// The reference graph: each definition key mapped to the targets it reaches and how.
pub(crate) type ReferenceEdges = AHashMap<Arc<str>, Vec<(Arc<str>, Position)>>;

/// Build the reference graph over the root and every definition.
///
/// The root is the node [`ROOT_DEFINITION_KEY`]: a self-reference is never keyed.
pub(crate) fn reference_edges(root: &Schema, definitions: &DefinitionMap) -> ReferenceEdges {
    let mut edges = ReferenceEdges::default();
    let mut record = |key: Arc<str>, body: &Schema| {
        let mut found = Vec::new();
        collect_classified_references(body, Position::InPlace, &mut found);
        // `Arc::clone`, not `Arc::from`: the IR already holds each target as an `Arc<str>`.
        let mut targets: Vec<(Arc<str>, Position)> = found
            .into_iter()
            .map(|(uri, position)| (Arc::clone(uri), position))
            .collect();
        // Over the whole pair: a body reaching one target both in place and under a consuming
        // keyword must keep both edges, or `guarded_components` qualifies an ill-founded cycle.
        targets.sort();
        targets.dedup();
        edges.insert(key, targets);
    };
    record(Arc::from(ROOT_DEFINITION_KEY), root);
    for (uri, body) in definitions {
        record(Arc::clone(uri), body);
    }
    edges
}

/// Every definition key on a reference cycle. Following one leads back to where it started and
/// never finishes, so callers leave these unresolved.
///
/// Built on the same [`strongly_connected`] pass the folding proofs use: a depth-first walk would
/// finish with a key before another on the same cycle had been reached, missing keys that are on
/// one - and which it missed would depend on the order it visited them in.
pub(crate) fn cyclic_definition_keys(
    definitions: &DefinitionMap,
    within: &BTreeSet<Arc<str>>,
) -> BTreeSet<Arc<str>> {
    // A target nothing reaches is on no cycle anything enters, so building the graph over what the
    // operands reach drops whole `$defs` maps while finding the same keys.
    if within.is_empty() {
        return BTreeSet::new();
    }
    let reachable: DefinitionMap = definitions
        .iter()
        .filter(|(uri, _)| within.contains(*uri))
        .map(|(uri, body)| (Arc::clone(uri), body.clone()))
        .collect();
    // `#` is not a key of the map, so nothing resolves through it: the graph covers the definitions
    // alone, against a root that references nothing.
    let edges = reference_edges(&Schema::truthy(), &reachable);
    strongly_connected(&edges)
        .into_iter()
        .filter(|component| is_cyclic(component, &edges))
        .flatten()
        .filter(|key| key.as_ref() != ROOT_DEFINITION_KEY)
        .collect()
}

/// Every definition key, ordered so a body comes after every body it reads. `None` where the map
/// holds a cycle, which admits no such order.
pub(crate) fn settling_order(definitions: &DefinitionMap) -> Option<Vec<Arc<str>>> {
    // `#` is not a key of the map, so nothing resolves through it: the graph covers the definitions
    // alone, against a root that references nothing.
    let edges = reference_edges(&Schema::truthy(), definitions);
    let mut order = Vec::with_capacity(definitions.len());
    // Tarjan hands a component back once everything it reaches is out, so targets come first.
    for component in strongly_connected(&edges) {
        if is_cyclic(&component, &edges) {
            return None;
        }
        // Tarjan walks targets too, so a component can name one the map does not hold - and `#`,
        // which is the root rather than an entry.
        order.extend(
            component
                .into_iter()
                .filter(|key| definitions.contains_key(key)),
        );
    }
    Some(order)
}

/// Every member of a cyclic component whose internal cycles all pass through a
/// [`Position::Consuming`] edge. The rest keep their references symbolic.
///
/// Flat, not grouped: one hypothesis covers every member at once, so the grouping the SCC pass
/// produces has no consumer.
pub(crate) fn guarded_members(edges: &ReferenceEdges) -> AHashSet<Arc<str>> {
    strongly_connected(edges)
        .into_iter()
        .filter(|component| is_cyclic(component, edges))
        .filter_map(|component| {
            let internal = restrict(&component, edges, Edges::All);
            let unguarded = in_place_cycle_members(&component, &internal);
            let guarded: Vec<Arc<str>> = component
                .into_iter()
                .filter(|key| !unguarded.contains(key))
                .collect();
            // Filtering can leave nothing to prove, which ordinary folding already handles.
            let remaining = restrict(&guarded, &internal, Edges::All);
            strongly_connected(&remaining)
                .iter()
                .any(|inner| is_cyclic(inner, &remaining))
                .then_some(guarded)
        })
        .flatten()
        .collect()
}

/// Members lying on a cycle made only of in-place edges, which no assumption may include.
///
/// Excluding just these keeps a well-founded member provable when an ill-founded one shares its
/// component; dropping them can only destroy cycles among the rest.
fn in_place_cycle_members(component: &[Arc<str>], internal: &ReferenceEdges) -> AHashSet<Arc<str>> {
    let restricted = restrict(component, internal, Edges::InPlaceOnly);
    strongly_connected(&restricted)
        .into_iter()
        .filter(|inner| is_cyclic(inner, &restricted))
        .flatten()
        .collect()
}

/// Definition keys on a cycle whose every edge is in place.
pub(crate) fn in_place_cycle_keys(
    root: &Schema,
    definitions: &DefinitionMap,
) -> AHashSet<Arc<str>> {
    let edges = reference_edges(root, definitions);
    strongly_connected(&edges)
        .into_iter()
        .filter(|component| is_cyclic(component, &edges))
        .flat_map(|component| {
            let internal = restrict(&component, &edges, Edges::All);
            in_place_cycle_members(&component, &internal)
        })
        .collect()
}

/// Which edges a restricted subgraph keeps.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Edges {
    All,
    InPlaceOnly,
}

/// `component`'s own subgraph, dropping edges to non-members and, for [`Edges::InPlaceOnly`],
/// consuming ones. Filtering here is what lets the walk itself stay filter-free.
fn restrict(component: &[Arc<str>], edges: &ReferenceEdges, keep: Edges) -> ReferenceEdges {
    let members: AHashSet<&Arc<str>> = component.iter().collect();
    component
        .iter()
        .map(|key| {
            let targets = edges
                .get(key)
                .map(|targets| {
                    targets
                        .iter()
                        .filter(|(target, position)| {
                            members.contains(target)
                                && (keep == Edges::All || *position == Position::InPlace)
                        })
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            (Arc::clone(key), targets)
        })
        .collect()
}

/// Whether `component` contains a cycle.
fn is_cyclic(component: &[Arc<str>], edges: &ReferenceEdges) -> bool {
    match component {
        [only] => edges
            .get(only)
            .is_some_and(|targets| targets.iter().any(|(target, _)| target == only)),
        _ => component.len() > 1,
    }
}

/// Tarjan's strongly connected components, iterative so a deep graph cannot overflow the stack.
///
/// Components come out in reverse topological order of the condensation.
fn strongly_connected(edges: &ReferenceEdges) -> Vec<Vec<Arc<str>>> {
    let mut index_of: AHashMap<Arc<str>, usize> = AHashMap::default();
    let mut low_of: AHashMap<Arc<str>, usize> = AHashMap::default();
    let mut on_stack: AHashSet<Arc<str>> = AHashSet::default();
    let mut stack: Vec<Arc<str>> = Vec::new();
    let mut components = Vec::new();
    let mut next_index = 0;

    let mut roots: Vec<&Arc<str>> = edges.keys().collect();
    roots.sort();

    for root in roots {
        if index_of.contains_key(root) {
            continue;
        }
        // Each frame is a node plus how many of its targets have been visited.
        let mut frames: Vec<(Arc<str>, usize)> = vec![(Arc::clone(root), 0)];
        index_of.insert(Arc::clone(root), next_index);
        low_of.insert(Arc::clone(root), next_index);
        next_index += 1;
        stack.push(Arc::clone(root));
        on_stack.insert(Arc::clone(root));

        while let Some((node, cursor)) = frames.pop() {
            let targets: &[(Arc<str>, Position)] =
                edges.get(&node).map_or(&[], |targets| targets.as_slice());
            let next = targets
                .iter()
                .enumerate()
                .skip(cursor)
                .find(|(_, (target, _))| edges.contains_key(target));
            if let Some((offset, (target, _))) = next {
                frames.push((Arc::clone(&node), offset + 1));
                if let Some(target_index) = index_of.get(target).copied() {
                    if on_stack.contains(target) {
                        let low = low_of[&node].min(target_index);
                        low_of.insert(Arc::clone(&node), low);
                    }
                } else {
                    index_of.insert(Arc::clone(target), next_index);
                    low_of.insert(Arc::clone(target), next_index);
                    next_index += 1;
                    stack.push(Arc::clone(target));
                    on_stack.insert(Arc::clone(target));
                    frames.push((Arc::clone(target), 0));
                }
                continue;
            }

            // Every target visited: close the node and propagate its low-link to the caller.
            if low_of[&node] == index_of[&node] {
                let mut component = Vec::new();
                while let Some(member) = stack.pop() {
                    on_stack.remove(&member);
                    let done = member == node;
                    component.push(member);
                    if done {
                        break;
                    }
                }
                components.push(component);
            }
            if let Some((parent, _)) = frames.last() {
                let low = low_of[parent].min(low_of[&node]);
                let parent = Arc::clone(parent);
                low_of.insert(parent, low);
            }
        }
    }
    components
}

/// `value`'s canonical form with every settled target folded away.
///
/// The whole pass behind one signature: applying a proof folds definitions it did not name, which
/// can expose a cycle the previous round could not see, so this repeats until nothing new is
/// settled. That makes the result a fixed point of the pass rather than a document whose own
/// re-canonicalization folds further.
pub(crate) fn fold_definitions<'a>(
    mut parsed: ParseOutput,
    value: &'a Value,
    ctx: &CanonicalizationContext,
    resolver: &Resolver<'a>,
) -> Result<ParseOutput, CanonicalizationError> {
    // Terminates because both sets only grow, into the document's finite set of keys.
    let mut assumptions = Assumptions::default();
    loop {
        let mut grew = false;
        let edges = reference_edges(&parsed.root, &parsed.definitions);
        // Proving a target empty can leave a cycle with nothing on it, and folding one away can
        // leave a target provably empty, so neither ordering settles - only running both to a
        // fixed point does.
        // An inversion cannot appear over a round that already resolved a target as `true`: that
        // only ever replaces a reference with `true`, and neither `allOf` nor `anyOf` - the only
        // operators a qualifying body is built from - can turn one into an inversion.
        debug_assert!(
            !inverts_an_operand(&parsed) || assumptions.admits_all.is_empty(),
            "resolving a target as `true` never puts an inversion over a later round"
        );
        for uri in unconstrained_members(&parsed, &edges) {
            grew |= assumptions.admits_all.insert(uri);
        }
        for uri in resolve_empty_definitions(&parsed, &edges, value, ctx, resolver, &assumptions)? {
            grew |= assumptions.empty.insert(uri);
        }
        if !grew {
            return Ok(parsed);
        }
        // Degrading to `Raw` leaves the proofs unapplied: an under-claim, and safe.
        let Some(refolded) = parse::parse_with(value, ctx, resolver, &assumptions)? else {
            return Ok(parsed);
        };
        parsed = refolded;
    }
}

/// Keys whose body names nothing but other such keys.
///
/// Closed under the reference edges and free of assertions, so every walk out of a member stays
/// inside the set forever without meeting one.
fn unconstrained_members(parsed: &ParseOutput, edges: &ReferenceEdges) -> AHashSet<Arc<str>> {
    // `reference_to_definition` is the only producer of `Reference`, so without one no body
    // qualifies. `not` and `oneOf` invert their operand, so a reference resolved to `true` under
    // one can reject a value the validator admits - and a member is named from anywhere in the
    // document, not only from the bodies this walk keeps. Both gates read the current IR, so a
    // later round still folds what an earlier one declined.
    if !parsed.has_references || inverts_an_operand(parsed) {
        return AHashSet::default();
    }
    let mut members: AHashSet<Arc<str>> = edges.keys().cloned().collect();
    // Who names each key, so dropping one revisits only the bodies that named it rather than
    // re-testing every member - a plain alias chain drops one key per sweep otherwise.
    let mut named_by: AHashMap<Arc<str>, Vec<Arc<str>>> = AHashMap::default();
    for (key, targets) in edges {
        for (target, _) in targets {
            named_by
                .entry(Arc::clone(target))
                .or_default()
                .push(Arc::clone(key));
        }
    }
    // Terminates because a key is enqueued once at the start and once per removal, and removals
    // are bounded by the member count.
    let mut queue: Vec<Arc<str>> = members.iter().map(Arc::clone).collect();
    while let Some(key) = queue.pop() {
        if !members.contains(&key) {
            continue;
        }
        if body_of(parsed, &key).is_some_and(|body| names_only(body, &members)) {
            continue;
        }
        members.remove(&key);
        if let Some(dependents) = named_by.get(&key) {
            queue.extend(dependents.iter().map(Arc::clone));
        }
    }
    members
}

/// Whether anything anywhere in the document inverts its operand.
fn inverts_an_operand(parsed: &ParseOutput) -> bool {
    inverts(&parsed.root) || parsed.definitions.values().any(inverts)
}

fn inverts(schema: &Schema) -> bool {
    match schema.kind() {
        SchemaKind::Not(_) | SchemaKind::OneOf(_) => true,
        SchemaKind::TypedGroup { body, .. } => inverts(body),
        SchemaKind::AllOf(branches) | SchemaKind::AnyOf(branches) => {
            branches.as_slice().iter().any(inverts)
        }
        SchemaKind::Array(leaf) => {
            let leaf = leaf.get();
            leaf.prefix.iter().any(inverts)
                || leaf.items.as_ref().is_some_and(inverts)
                || leaf.contains.iter().any(|facet| inverts(&facet.schema))
        }
        SchemaKind::Object(leaf) => {
            let leaf = leaf.get();
            // `NameFails(S)` is `not(for every key, S(key))`: anti-monotone in `S` regardless of
            // what `S` holds, so any violation demand at all inverts, not only one whose own body
            // does.
            leaf.property_names.as_ref().is_some_and(inverts)
                || leaf.properties.values().any(inverts)
                || leaf.pattern_properties.values().any(inverts)
                || leaf.additional.as_ref().is_some_and(inverts)
                || !leaf.violations.is_empty()
        }
        SchemaKind::Reference(_)
        | SchemaKind::MultiType(_)
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

/// Whether `schema` is built solely from references into `members`.
fn names_only(schema: &Schema, members: &AHashSet<Arc<str>>) -> bool {
    match schema.kind() {
        SchemaKind::Reference(uri) => members.contains(uri.as_ref()),
        SchemaKind::AllOf(branches) | SchemaKind::AnyOf(branches) => branches
            .as_slice()
            .iter()
            .all(|branch| names_only(branch, members)),
        // `not` and `oneOf` invert their operand, so a cycle through one has no fixed point at all.
        SchemaKind::Not(_)
        | SchemaKind::OneOf(_)
        | SchemaKind::TypedGroup { .. }
        | SchemaKind::Array(_)
        | SchemaKind::Object(_)
        | SchemaKind::MultiType(_)
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

/// Definitions proven to denote the empty set, given `parsed` is `value`'s current canonical form.
///
/// Assume every guarded member empty at once, re-parse under that hypothesis, drop the members that
/// did not come back `false`, repeat. The design spec proves the survivors.
///
/// One parse per round, not one per member: the hypothesis is the same for all of them.
///
/// `proven` holds earlier rounds' results. `parsed` reflects them, `value` does not, so every
/// hypothesis carries `proven` beside the assumptions under test.
fn resolve_empty_definitions<'a>(
    parsed: &ParseOutput,
    edges: &ReferenceEdges,
    value: &'a Value,
    ctx: &CanonicalizationContext,
    resolver: &Resolver<'a>,
    proven: &Assumptions,
) -> Result<AHashSet<Arc<str>>, CanonicalizationError> {
    // `resolve_reference` is the only producer of `Reference`, so without one there is no graph,
    // no cycle, and nothing to prove.
    if !parsed.has_references {
        return Ok(AHashSet::default());
    }
    let mut assumed = guarded_members(edges);
    // A target resolved as `true` cannot also be resolved as `false`: `reference_to_definition`
    // reads one set before the other, so an overlap would silently pick a winner.
    assumed.retain(|key| !proven.admits_all.contains(key));
    let mut assumed = plausible_assumptions(parsed, assumed);
    // Terminates because a continuing round drops at least one assumption.
    while !assumed.is_empty() {
        let hypothesis = Assumptions {
            empty: proven.empty.union(&assumed).cloned().collect(),
            admits_all: proven.admits_all.clone(),
            finished: proven.finished.clone(),
        };
        // A hypothesis that keeps the document `Raw` proves nothing; giving up is an under-claim.
        let Some(hypothetical) = parse::parse_hypothesis(value, ctx, resolver, &hypothesis)? else {
            return Ok(AHashSet::default());
        };
        let survivors: AHashSet<Arc<str>> = assumed
            .iter()
            .filter(|key| folds_to_false(key, &hypothetical))
            .cloned()
            .collect();
        if survivors.len() == assumed.len() {
            return Ok(assumed);
        }
        assumed = plausible_assumptions(parsed, survivors);
    }
    Ok(assumed)
}

/// Whether substituting `False` for `assumed` could make `schema` itself `False`.
///
/// A necessary condition checked on the built IR, so cycles recursing only through optional
/// positions - the common shape, and every cycle in FHIR - never pay for a hypothesis parse.
/// Saying yes only costs one; the parse still decides.
fn may_fold(schema: &Schema, assumed: &AHashSet<Arc<str>>) -> bool {
    match schema.kind() {
        SchemaKind::False => true,
        SchemaKind::Reference(uri) => assumed.contains(uri.as_ref()),
        SchemaKind::TypedGroup { body, .. } => may_fold(body, assumed),
        // One failing conjunct is enough; a disjunction needs every branch to fail.
        SchemaKind::AllOf(branches) => branches.as_slice().iter().any(|b| may_fold(b, assumed)),
        SchemaKind::AnyOf(branches) => branches.as_slice().iter().all(|b| may_fold(b, assumed)),
        SchemaKind::OneOf(branches) => branches.iter().all(|b| may_fold(b, assumed)),
        SchemaKind::Array(leaf) => {
            let leaf = leaf.get();
            // An index is demanded when the minimum length reaches past it.
            let demands = |index: usize| {
                leaf.lengths.minimum.as_ref().is_some_and(|minimum| {
                    u64::try_from(index)
                        .ok()
                        .and_then(|index| index.checked_add(1))
                        .is_some_and(|reach| *minimum >= BoundCardinality::from(reach))
                })
            };
            let demanded_prefix = leaf
                .prefix
                .iter()
                .enumerate()
                .any(|(index, schema)| demands(index) && may_fold(schema, assumed));
            let demanded_items = leaf
                .items
                .as_ref()
                .is_some_and(|schema| demands(leaf.prefix.len()) && may_fold(schema, assumed));
            let demanded_contains = leaf.contains.iter().any(|facet| {
                !facet.effective_minimum().is_zero() && may_fold(&facet.schema, assumed)
            });
            demanded_prefix || demanded_items || demanded_contains
        }
        SchemaKind::Object(leaf) => {
            let leaf = leaf.get();
            // An object owing at least one key, none of which can be named, admits nothing.
            let demands_a_key = !leaf.required.is_empty()
                || leaf
                    .sizes
                    .minimum
                    .as_ref()
                    .is_some_and(|minimum| !minimum.is_zero());
            let demanded_names = demands_a_key
                && (leaf
                    .property_names
                    .as_ref()
                    .is_some_and(|schema| may_fold(schema, assumed))
                    // A violation records a rule the object must break, never a key it must carry.
                    || leaf.violations.iter().any(|violation| match violation {
                        ObjectViolation::NameFails(_)
                        | ObjectViolation::UndeclaredValueFails { .. } => false,
                    }));
            // A key must come from the catch-all when nothing names one, so `minProperties` alone
            // demands it. Deciding which pattern claims a key needs the pattern engine, so any of
            // them counts - and left tight deliberately: widening this to named properties would
            // put every `required` object with an optional recursive property back on the slow path.
            let demanded_catch_all = demands_a_key
                && leaf.required.is_empty()
                && leaf.properties.is_empty()
                && (leaf
                    .pattern_properties
                    .values()
                    .any(|schema| may_fold(schema, assumed))
                    || leaf
                        .additional
                        .as_ref()
                        .is_some_and(|schema| may_fold(schema, assumed)));
            // A required key is demanded; an optional one admitting nothing just forbids its key.
            let demanded_required =
                leaf.required
                    .iter()
                    .any(|key| match leaf.properties.get(key) {
                        Some(schema) => may_fold(schema, assumed),
                        None => {
                            leaf.pattern_properties
                                .values()
                                .any(|schema| may_fold(schema, assumed))
                                || leaf
                                    .additional
                                    .as_ref()
                                    .is_some_and(|schema| may_fold(schema, assumed))
                        }
                    });
            demanded_names || demanded_catch_all || demanded_required
        }
        SchemaKind::Not(_)
        | SchemaKind::MultiType(_)
        | SchemaKind::String(_)
        | SchemaKind::Integer(_)
        | SchemaKind::Number(_)
        | SchemaKind::Const(_)
        | SchemaKind::Enum(_)
        | SchemaKind::True
        | SchemaKind::Raw(_) => false,
    }
}

/// `assumed` grown with every definition that could fold once the assumptions hold.
///
/// A cycle can collapse through a definition outside it, so filtering against the bare assumptions
/// would miss those.
fn foldable_closure(parsed: &ParseOutput, assumed: &AHashSet<Arc<str>>) -> AHashSet<Arc<str>> {
    let mut candidates = assumed.clone();
    loop {
        let mut grew = false;
        for (key, body) in &parsed.definitions {
            if !candidates.contains(key) && may_fold(body, &candidates) {
                candidates.insert(Arc::clone(key));
                grew = true;
            }
        }
        if !grew {
            return candidates;
        }
    }
}

/// Shrink `assumed` to the members that could still fold, until the set stops changing.
fn plausible_assumptions(
    parsed: &ParseOutput,
    mut assumed: AHashSet<Arc<str>>,
) -> AHashSet<Arc<str>> {
    // The closure below would sweep every definition to produce a subset of the empty set.
    if assumed.is_empty() {
        return assumed;
    }
    loop {
        let candidates = foldable_closure(parsed, &assumed);
        let kept: AHashSet<Arc<str>> = assumed
            .iter()
            .filter(|key| body_of(parsed, key).is_some_and(|body| may_fold(body, &candidates)))
            .cloned()
            .collect();
        if kept.len() == assumed.len() {
            return kept;
        }
        assumed = kept;
    }
}

/// Whether `key`'s body came back `false` from a parse carrying the round's hypothesis.
fn folds_to_false(key: &Arc<str>, hypothetical: &ParseOutput) -> bool {
    body_of(hypothetical, key).is_some_and(|body| matches!(body.kind(), SchemaKind::False))
}

/// The body a graph node names: [`ROOT_DEFINITION_KEY`] is the parse root, everything else a
/// definition.
fn body_of<'a>(parsed: &'a ParseOutput, key: &str) -> Option<&'a Schema> {
    if key == ROOT_DEFINITION_KEY {
        Some(&parsed.root)
    } else {
        parsed.definitions.get(key)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};
    use test_case::test_case;

    use super::*;

    /// The parse output for `schema`, which is what the pass actually consumes.
    fn parsed_of(schema: &Value) -> ParseOutput {
        let registry = referencing::Registry::new()
            .add(
                "json-schema:///",
                referencing::Draft::Draft202012.create_resource_ref(schema),
            )
            .expect("the fixture is a resource")
            .draft(referencing::Draft::Draft202012)
            .prepare()
            .expect("the registry prepares");
        let resolver = registry
            .resolver(referencing::uri::from_str("json-schema:///").expect("a valid base URI"));
        let ctx = CanonicalizationContext::new(
            referencing::Draft::Draft202012,
            crate::options::PatternEngineOptions::default(),
            false,
        );
        parse::parse(schema, &ctx, &resolver)
            .expect("the fixture parses")
            .expect("the fixture is modeled")
    }

    /// The position of `schema`'s single `#` self-reference.
    fn self_reference_position(schema: &Value) -> Position {
        let parsed = parsed_of(schema);
        let mut found = Vec::new();
        collect_classified_references(&parsed.root, Position::InPlace, &mut found);
        let positions: Vec<Position> = found
            .into_iter()
            .filter(|(uri, _)| uri.as_ref() == "#")
            .map(|(_, position)| position)
            .collect();
        assert_eq!(
            positions.len(),
            1,
            "the fixture holds exactly one self-reference, found {positions:?}"
        );
        positions[0]
    }

    #[test_case(&json!({"properties": {"a": {"$ref": "#"}}}), Position::Consuming ; "properties")]
    #[test_case(&json!({"patternProperties": {"^a": {"$ref": "#"}}}), Position::Consuming ; "pattern_properties")]
    #[test_case(&json!({"type": "object", "additionalProperties": {"$ref": "#"}}), Position::Consuming ; "additional_properties")]
    #[test_case(&json!({"type": "object", "propertyNames": {"$ref": "#"}}), Position::Consuming ; "property_names")]
    #[test_case(&json!({"type": "array", "items": {"$ref": "#"}}), Position::Consuming ; "items")]
    #[test_case(&json!({"type": "array", "prefixItems": [{"$ref": "#"}]}), Position::Consuming ; "prefix_items")]
    #[test_case(&json!({"contains": {"$ref": "#"}}), Position::Consuming ; "contains")]
    #[test_case(&json!({"allOf": [{"$ref": "#"}, {"type": "integer"}]}), Position::InPlace ; "all_of")]
    #[test_case(&json!({"anyOf": [{"$ref": "#"}, {"type": "integer"}]}), Position::InPlace ; "any_of")]
    #[test_case(&json!({"not": {"$ref": "#"}}), Position::InPlace ; "not")]
    fn classifies_reference_position(schema: &Value, expected: Position) {
        assert_eq!(self_reference_position(schema), expected);
    }

    #[test]
    fn nesting_below_a_consuming_position_stays_consuming() {
        let schema = json!({"properties": {"a": {"anyOf": [{"$ref": "#"}, {"type": "null"}]}}});
        assert_eq!(self_reference_position(&schema), Position::Consuming);
    }

    /// The graph the pass sees, built from the parse rather than the emitted form - so a fixture
    /// does not have to be one the pass leaves alone.
    fn edges_of(schema: &Value) -> ReferenceEdges {
        let parsed = parsed_of(schema);
        reference_edges(&parsed.root, &parsed.definitions)
    }

    fn guarded(schema: &Value) -> Vec<String> {
        let mut names: Vec<String> = guarded_members(&edges_of(schema))
            .iter()
            .map(ToString::to_string)
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_consuming_self_cycle_qualifies() {
        let schema = json!({"type": "object", "properties": {"a": {"$ref": "#"}}});
        assert_eq!(guarded(&schema), vec!["#".to_string()]);
    }

    /// The cycle the pass actually decides. Reachable only because the graph is built from the
    /// parse: the emitted form of this document is `false`, with no graph left to inspect.
    #[test]
    fn a_required_consuming_self_cycle_qualifies() {
        let schema =
            json!({"type": "object", "required": ["a"], "properties": {"a": {"$ref": "#"}}});
        assert_eq!(guarded(&schema), vec!["#".to_string()]);
    }

    #[test]
    fn an_in_place_self_cycle_is_excluded() {
        let schema = json!({"$ref": "#/$defs/a", "$defs": {"a": {"allOf": [{"$ref": "#/$defs/a"}, {"type": "integer"}]}}});
        assert!(guarded(&schema).is_empty());
    }

    /// Every cycle here passes through the consuming edge.
    #[test]
    fn a_mixed_cycle_qualifies() {
        let schema = json!({
            "$ref": "#/$defs/a",
            "$defs": {
                "a": {"allOf": [{"$ref": "#/$defs/b"}, {"type": "object"}]},
                "b": {"type": "object", "properties": {"x": {"$ref": "#/$defs/a"}}}
            }
        });
        assert_eq!(
            guarded(&schema),
            vec!["#/$defs/a".to_string(), "#/$defs/b".to_string()]
        );
    }

    /// A member keeping a consuming cycle of its own stays guarded when an ill-founded member
    /// shares its component; one whose only cycle ran through the excluded member does not.
    #[test]
    fn only_the_ill_founded_member_is_excluded() {
        let schema = json!({
            "$ref": "#/$defs/a",
            "$defs": {
                "a": {"type": "object", "properties": {
                    "x": {"$ref": "#/$defs/a"},
                    "y": {"$ref": "#/$defs/b"}
                }},
                "b": {"anyOf": [{"$ref": "#/$defs/b"}, {"$ref": "#/$defs/a"}]}
            }
        });
        assert_eq!(guarded(&schema), vec!["#/$defs/a".to_string()]);
    }

    #[test]
    fn an_acyclic_document_has_no_members() {
        let schema = json!({"$ref": "#/$defs/a", "$defs": {"a": {"type": "integer"}}});
        assert!(guarded(&schema).is_empty());
    }
}
