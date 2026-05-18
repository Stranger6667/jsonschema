//! Recursion soundness checks for the schema graph.
//!
//! [`check_unguarded_recursion`] rejects raw-JSON cycles through only composition keywords (no fixed point);
//! [`check_infinite_recursion`] rejects IR cycles with no finite instance (every value forced to nest, no base case).

use std::sync::Arc;

use ahash::{AHashMap, AHashSet};
use referencing::{Draft, Resolver};
use serde_json::Value;

use crate::{
    canonical::{
        context::compile_with,
        document::{canonical_registry_builder, exceeds_depth_limit, root_base_uri},
        error::CanonicalizationError,
        ir::{
            ArrayLeaf, ObjectConstraint, ObjectLeaf, ObjectRequirement, OneOf, PropertyNameMatcher,
            Schema, SharedSchema,
        },
        parse::{build_def_pointer, ValueIdentity},
    },
    options::PatternEngineOptions,
};

// A cycle traversing only these never consumes any of the recursive value, so the recursion is ill-founded.
const UNGUARDED_KEYWORDS: &[&str] = &["allOf", "anyOf", "oneOf", "not"];

pub(super) fn check_unguarded_recursion(
    value: &Value,
    draft: Draft,
    resolver: Option<&Resolver<'_>>,
) -> Result<(), CanonicalizationError> {
    let cycle = if let Some(resolver) = resolver {
        unguarded_cycle(value, draft, resolver)
    } else {
        // Register the document locally so same-document refs resolve. External refs fail to prepare (the default
        // retriever never fetches) and the check is skipped; production callers always supply a resolver.
        let base_uri = root_base_uri(value, draft, None);
        let resource = draft.create_resource_ref(value);
        let Ok(registry) = canonical_registry_builder(None, &base_uri, resource, draft)
            .and_then(referencing::RegistryBuilder::prepare)
        else {
            return Ok(());
        };
        unguarded_cycle(value, draft, &registry.resolver(base_uri))
    };
    cycle.map_or(Ok(()), |node| {
        Err(CanonicalizationError::UnguardedRecursion(node))
    })
}

#[derive(Default)]
struct ReferenceGraph {
    edges: AHashMap<ValueIdentity, Vec<GraphEdge>>,
    names: AHashMap<ValueIdentity, String>,
}

/// An edge retained for cycle detection. `find_cycle` reads only the target identity and its display
/// name, so the graph stores nothing more.
struct GraphEdge {
    target: ValueIdentity,
    target_name: String,
}

/// A composition `$ref` edge plus everything needed to recurse into its target. Transient: consumed
/// during the walk and never stored in the graph.
struct ReferenceEdge<'r> {
    target: ValueIdentity,
    target_name: String,
    target_schema: &'r Value,
    target_resolver: Resolver<'r>,
    target_draft: Draft,
}

/// A pending node visit for the iterative walk. Owns its resolver (cloning the parent's when a nested node
/// reuses it) so a queued item outlives the frame that produced it.
struct WorkItem<'r> {
    name: String,
    schema: &'r Value,
    resolver: Resolver<'r>,
    draft: Draft,
    include_direct_ref: bool,
}

fn unguarded_cycle<'r>(value: &'r Value, draft: Draft, resolver: &Resolver<'r>) -> Option<String> {
    let mut graph = ReferenceGraph::default();
    let mut visited = AHashSet::new();
    let mut unguarded_ref_source = AHashSet::new();
    let root_resolver = resource_scope(value, resolver, draft).unwrap_or_else(|| resolver.clone());
    // Explicit work stack: recursing one native frame per `$ref` edge overflows on a long (valid) chain.
    // `visited` bounds the walk to one body per node.
    let mut stack = vec![WorkItem {
        name: String::new(),
        schema: value,
        resolver: root_resolver,
        draft,
        include_direct_ref: true,
    }];
    while let Some(item) = stack.pop() {
        visit_node(
            item,
            &mut graph,
            &mut visited,
            &mut unguarded_ref_source,
            &mut stack,
        );
    }
    find_cycle(&graph)
}

/// Record `item`'s node and queue its outgoing `$ref` edges and nested referenceable nodes. `item.resolver`
/// must already be scoped to `item.schema`'s resource.
fn visit_node<'r>(
    item: WorkItem<'r>,
    graph: &mut ReferenceGraph,
    visited: &mut AHashSet<ValueIdentity>,
    unguarded_ref_source: &mut AHashSet<ValueIdentity>,
    stack: &mut Vec<WorkItem<'r>>,
) {
    let WorkItem {
        name,
        schema,
        resolver,
        draft,
        include_direct_ref,
    } = item;
    let node = ValueIdentity::of(schema);
    graph.names.entry(node).or_insert(name);
    if !visited.insert(node) {
        // A guarded first visit suppressed this node's own `$ref`; on a later unguarded visit add that
        // now-unguarded edge (the rest of the walk already ran).
        if include_direct_ref && unguarded_ref_source.insert(node) {
            if let Some(reference) = schema
                .as_object()
                .and_then(|map| map.get("$ref"))
                .and_then(Value::as_str)
            {
                if let Some(edge) = direct_ref_edge(reference, &resolver) {
                    graph.edges.entry(node).or_default().push(GraphEdge {
                        target: edge.target,
                        target_name: edge.target_name.clone(),
                    });
                    queue_edge(edge, stack);
                }
            }
        }
        return;
    }
    // Record this node so a later unguarded visit does not re-add the direct `$ref` edge collected below.
    if include_direct_ref {
        unguarded_ref_source.insert(node);
    }
    let mut edges = Vec::new();
    collect_composition_refs(schema, &resolver, draft, &mut edges, include_direct_ref);
    graph.edges.insert(
        node,
        edges
            .iter()
            .map(|edge| GraphEdge {
                target: edge.target,
                target_name: edge.target_name.clone(),
            })
            .collect(),
    );
    for edge in edges {
        queue_edge(edge, stack);
    }
    queue_nested_referenceable_nodes(schema, &resolver, draft, stack);
}

/// Queue an edge target as an unguarded visit. A target past the depth cap is raw-preserved by parse, so its
/// cycles are moot; skip it.
fn queue_edge<'r>(edge: ReferenceEdge<'r>, stack: &mut Vec<WorkItem<'r>>) {
    if exceeds_depth_limit(edge.target_schema) {
        return;
    }
    stack.push(WorkItem {
        name: edge.target_name,
        schema: edge.target_schema,
        resolver: edge.target_resolver,
        draft: edge.target_draft,
        include_direct_ref: true,
    });
}

/// Queue every referenceable node nested in `schema`: `$defs`/`definitions` entries as unguarded roots, then
/// the guarded subresources. `resolver` must already be scoped to `schema`'s resource.
fn queue_nested_referenceable_nodes<'r>(
    schema: &'r Value,
    resolver: &Resolver<'r>,
    draft: Draft,
    stack: &mut Vec<WorkItem<'r>>,
) {
    let Value::Object(map) = schema else {
        return;
    };
    // `$defs`/`definitions` entries are unguarded roots (a bare `$ref` there can sit on an ill-founded cycle), so visit
    // with `include_direct_ref = true`. Record them to skip the redundant guarded visit `subresources_of` re-yields.
    let mut definition_nodes = AHashSet::new();
    for (registry, prefix) in [("$defs", "#/$defs/"), ("definitions", "#/definitions/")] {
        if let Some(Value::Object(definitions)) = map.get(registry) {
            for (name, child) in definitions {
                definition_nodes.insert(ValueIdentity::of(child));
                let child_resolver =
                    resource_scope(child, resolver, draft).unwrap_or_else(|| resolver.clone());
                stack.push(WorkItem {
                    name: definition_node_name(resolver, prefix, name),
                    schema: child,
                    resolver: child_resolver,
                    draft,
                    include_direct_ref: true,
                });
            }
        }
    }
    for child in draft.subresources_of(schema) {
        if definition_nodes.contains(&ValueIdentity::of(child)) {
            continue;
        }
        let child_resolver =
            resource_scope(child, resolver, draft).unwrap_or_else(|| resolver.clone());
        stack.push(WorkItem {
            name: child_resolver.base_uri().as_str().to_owned(),
            schema: child,
            resolver: child_resolver,
            draft,
            include_direct_ref: false,
        });
    }
}

/// Reject recursive schemas with no finite instance: recursion in a required position with no base case. Ordinary
/// finite contradictions fold into `Schema::False` downstream, so this runs only when a real cycle exists.
pub(super) fn check_infinite_recursion(
    root: &SharedSchema,
    definitions: &AHashMap<Arc<str>, SharedSchema>,
    cyclic: &AHashSet<Arc<str>>,
    pattern_options: PatternEngineOptions,
) -> Result<(), CanonicalizationError> {
    let mut prover = ProductivityProver {
        definitions,
        productive: AHashSet::new(),
        pattern_options,
    };
    prover.saturate();
    if prover.is_productive(root.as_schema()) {
        Ok(())
    } else {
        let node = cyclic
            .iter()
            .min()
            .map_or_else(String::new, ToString::to_string);
        Err(CanonicalizationError::InfiniteRecursion(node))
    }
}

struct ProductivityProver<'a> {
    definitions: &'a AHashMap<Arc<str>, SharedSchema>,
    /// Definition keys proven to admit a finite instance.
    productive: AHashSet<Arc<str>>,
    pattern_options: PatternEngineOptions,
}

impl ProductivityProver<'_> {
    /// Least fixpoint: a definition is productive once its body admits a finite instance, treating recursive leaves
    /// as productive only after their target is proven productive.
    fn saturate(&mut self) {
        loop {
            let newly: Vec<Arc<str>> = self
                .definitions
                .iter()
                .filter(|(key, body)| {
                    !self.productive.contains(*key) && self.is_productive(body.as_schema())
                })
                .map(|(key, _)| Arc::clone(key))
                .collect();
            if newly.is_empty() {
                break;
            }
            self.productive.extend(newly);
        }
    }

    /// Whether a finite instance of `schema` exists, given the keys already proven productive. Conservative:
    /// anything not provably blocking (external refs, `not`, raw/dynamic, leaves) is productive, never rejecting satisfiable schemas.
    fn is_productive(&self, schema: &Schema) -> bool {
        match schema {
            Schema::False => false,
            Schema::Recursive(name) => self.productive.contains(name),
            Schema::Reference(uri) => match self.definitions.get(uri.as_str()) {
                Some(_) => self.productive.contains(uri.as_str()),
                None => true,
            },
            Schema::AllOf(members) => members
                .iter()
                .all(|member| self.is_productive(member.as_schema())),
            Schema::AnyOf(members) | Schema::OneOf(OneOf(members)) => members
                .iter()
                .any(|member| self.is_productive(member.as_schema())),
            // A `TypedGroup` is `ty AND body`, productive only when the body is. A `TypeGuard` is
            // `NOT ty OR body`, satisfied by any value outside the guarded type, so it is productive
            // regardless of body and falls to the conservative `_` arm.
            Schema::TypedGroup { body, .. } => self.is_productive(body.as_schema()),
            Schema::IfThenElse(conditional) => {
                let arm_ok = |arm: &Option<SharedSchema>| {
                    arm.as_ref()
                        .is_none_or(|schema| self.is_productive(schema.as_schema()))
                };
                arm_ok(&conditional.then_branch) || arm_ok(&conditional.else_branch)
            }
            Schema::Object(object) => self.is_object_productive(object),
            Schema::Array(array) => self.is_array_productive(array),
            _ => true,
        }
    }

    fn is_object_productive(&self, object: &ObjectLeaf) -> bool {
        if !self.min_properties_productive(object) {
            return false;
        }
        object
            .requirements
            .iter()
            .all(|requirement| match requirement {
                ObjectRequirement::RequiredProperty(name) => {
                    self.required_property_productive(name, object)
                }
                _ => true,
            })
    }

    fn min_properties_productive(&self, object: &ObjectLeaf) -> bool {
        let minimum = object
            .requirements
            .iter()
            .find_map(|requirement| match requirement {
                ObjectRequirement::MinProperties(minimum) if !minimum.is_zero() => {
                    Some(minimum.to_usize().unwrap_or(usize::MAX))
                }
                _ => None,
            });
        let Some(minimum) = minimum else {
            return true;
        };
        if self.has_universal_non_productive_pattern(object) {
            return false;
        }
        let Some(additional) = object.constraints.iter().find(|constraint| {
            matches!(
                constraint.matcher,
                PropertyNameMatcher::AdditionalProperties
            )
        }) else {
            return true;
        };
        if self.is_productive(additional.schema.as_schema()) {
            return true;
        }
        let mut productive_named_properties = 0usize;
        for constraint in &object.constraints {
            if let PropertyNameMatcher::NamedProperty(name) = &constraint.matcher {
                if self.required_property_productive(name, object) {
                    productive_named_properties += 1;
                }
            }
        }
        if productive_named_properties >= minimum {
            return true;
        }
        object.constraints.iter().any(|constraint| {
            matches!(constraint.matcher, PropertyNameMatcher::PatternProperty(_))
                && self.is_productive(constraint.schema.as_schema())
        })
    }

    fn has_universal_non_productive_pattern(&self, object: &ObjectLeaf) -> bool {
        object.constraints.iter().any(|constraint| {
            let PropertyNameMatcher::PatternProperty(pattern) = &constraint.matcher else {
                return false;
            };
            pattern_matches_every_name(pattern)
                && !self.is_productive(constraint.schema.as_schema())
        })
    }

    /// Whether a required property `name` can hold a value with a finite instance.
    ///
    /// Its value must satisfy every applicable subschema, so one non-productive applicable subschema sinks the object.
    /// Conservative: a constraint applies only when provably so, so an uncompilable pattern never forces a rejection.
    fn required_property_productive(&self, name: &str, object: &ObjectLeaf) -> bool {
        let mut matched_named_or_pattern = false;
        // Set when a pattern fails to compile: we can't prove `name` matches no pattern, so `additionalProperties`
        // can't be shown to apply.
        let mut pattern_match_uncertain = false;
        let mut additional: Option<&ObjectConstraint> = None;
        for constraint in &object.constraints {
            match &constraint.matcher {
                PropertyNameMatcher::NamedProperty(other) if other.as_ref() == name => {
                    matched_named_or_pattern = true;
                    if !self.is_productive(constraint.schema.as_schema()) {
                        return false;
                    }
                }
                PropertyNameMatcher::NamedProperty(_) => {}
                PropertyNameMatcher::PatternProperty(pattern) => {
                    match compile_with(self.pattern_options, pattern) {
                        Some(matcher) if matcher.is_match(name) => {
                            matched_named_or_pattern = true;
                            if !self.is_productive(constraint.schema.as_schema()) {
                                return false;
                            }
                        }
                        // Compiles but does not match `name`: this subschema does not apply.
                        Some(_) => {}
                        // Uncompilable pattern: can't prove it doesn't match, so don't let `additionalProperties`
                        // apply.
                        None => pattern_match_uncertain = true,
                    }
                }
                PropertyNameMatcher::AdditionalProperties => additional = Some(constraint),
            }
        }
        // `additionalProperties` applies to `name` only when no `properties` entry and no `patternProperties`
        // matched it.
        if !matched_named_or_pattern && !pattern_match_uncertain {
            if let Some(constraint) = additional {
                if !self.is_productive(constraint.schema.as_schema()) {
                    return false;
                }
            }
        }
        true
    }

    fn is_array_productive(&self, array: &ArrayLeaf) -> bool {
        // A required `contains` element must have a finite instance.
        for clause in &array.contains {
            if !clause.min_contains.is_zero() && !self.is_productive(clause.schema.as_schema()) {
                return false;
            }
        }
        if array.length.minimum.is_zero() {
            return true;
        }
        // Forced positions: each `prefix` entry up to `minimum`, then `tail` for the rest.
        let minimum = array.length.minimum.to_usize().unwrap_or(usize::MAX);
        let forced = minimum.min(array.prefix.len());
        if !array.prefix[..forced]
            .iter()
            .all(|position| self.is_productive(position.as_schema()))
        {
            return false;
        }
        minimum <= array.prefix.len() || self.is_productive(array.tail.as_schema())
    }
}

/// Whether a `patternProperties` regex matches every possible property name. Conservative: matches the
/// universal spellings seen in practice after peeling a whole-pattern group (`(.*)`, `(?:.*)`). Treating an
/// unrecognized universal as non-universal is sound - it only misses this recursion-detection shortcut.
fn pattern_matches_every_name(pattern: &str) -> bool {
    let mut pattern = pattern;
    while let Some(inner) = strip_full_span_group(pattern) {
        pattern = inner;
    }
    matches!(
        pattern,
        "" | ".*" | ".*?" | "[\\s\\S]*" | "[\\S\\s]*" | "^.*$" | "^.*" | ".*$" | "^" | "$"
    )
}

/// The body of a group spanning the whole pattern (`(X)`, `(?:X)`, `(?<n>X)`), else `None`. The group must
/// open at the first byte and close only at the last, so `(a)(b)` is not one span.
fn strip_full_span_group(pattern: &str) -> Option<&str> {
    let body_start = if let Some(rest) = pattern.strip_prefix("(?:") {
        pattern.len() - rest.len()
    } else if pattern.starts_with("(?<")
        || pattern.starts_with("(?P<")
        || pattern.starts_with("(?'")
    {
        pattern.find(['>', '\''])? + 1
    } else if pattern.starts_with('(') && !pattern.starts_with("(?") {
        1
    } else {
        return None;
    };
    if !balanced_group_spans_all(pattern) {
        return None;
    }
    pattern.get(body_start..pattern.len() - 1)
}

/// Whether the leading `(` closes only at the final byte: depth stays `>= 1` from the opening paren until
/// the last byte, honoring `\(`/`\)` escapes and treating `[...]` character classes as opaque.
fn balanced_group_spans_all(pattern: &str) -> bool {
    let bytes = pattern.as_bytes();
    if bytes.last() != Some(&b')') {
        return false;
    }
    let mut depth = 0i32;
    let mut escaped = false;
    let mut in_class = false;
    for (index, &byte) in bytes.iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        match byte {
            b'\\' => escaped = true,
            b'[' if !in_class => in_class = true,
            b']' if in_class => in_class = false,
            b'(' if !in_class => depth += 1,
            b')' if !in_class => {
                depth -= 1;
                // Closing before the final byte means the leading group already ended.
                if depth == 0 && index != bytes.len() - 1 {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

/// Collect `$ref` edges reachable through composition keywords only. A ref under any other keyword is guarded (the
/// recursion consumes part of the value) so cannot sit on an ill-founded cycle. `resolver` must be scoped to `value`.
fn collect_composition_refs<'r>(
    value: &Value,
    resolver: &Resolver<'r>,
    draft: Draft,
    out: &mut Vec<ReferenceEdge<'r>>,
    include_direct_ref: bool,
) {
    let Value::Object(map) = value else {
        return;
    };
    if let Some(reference) = map.get("$ref").and_then(Value::as_str) {
        if include_direct_ref {
            if let Some(edge) = direct_ref_edge(reference, resolver) {
                out.push(edge);
            }
        }
        // Draft 4/6/7 treat `$ref` as exclusive - sibling keywords are ignored (mirrors the parser). Newer
        // drafts evaluate `$ref` alongside its composition siblings, so keep collecting their edges too.
        if ref_is_exclusive(draft) {
            return;
        }
    }
    for keyword in UNGUARDED_KEYWORDS {
        match map.get(*keyword) {
            Some(Value::Array(branches)) => {
                for branch in branches {
                    collect_branch_refs(branch, resolver, draft, out);
                }
            }
            Some(branch) => collect_branch_refs(branch, resolver, draft, out),
            None => {}
        }
    }
    // `if`/`then`/`else` apply to the same instance; `then`/`else` are inert without `if`.
    if draft.is_known_keyword("if") && map.contains_key("if") {
        for keyword in ["if", "then", "else"] {
            if let Some(branch) = map.get(keyword) {
                collect_branch_refs(branch, resolver, draft, out);
            }
        }
    }
    // Schema-form dependents apply to the same instance too; array-form `dependencies` entries are
    // required property names, not schemas.
    for keyword in ["dependentSchemas", "dependencies"] {
        if !draft.is_known_keyword(keyword) {
            continue;
        }
        if let Some(Value::Object(entries)) = map.get(keyword) {
            for entry in entries.values() {
                if !matches!(entry, Value::Array(_)) {
                    collect_branch_refs(entry, resolver, draft, out);
                }
            }
        }
    }
}

/// Whether `$ref` suppresses its sibling keywords, matching the parser's draft split.
fn ref_is_exclusive(draft: Draft) -> bool {
    matches!(draft, Draft::Draft4 | Draft::Draft6 | Draft::Draft7)
}

/// The unguarded edge for a node's own `$ref`, or `None` when it does not resolve. `resolver` must be scoped
/// to the node.
fn direct_ref_edge<'r>(reference: &str, resolver: &Resolver<'r>) -> Option<ReferenceEdge<'r>> {
    let resolved = resolver.lookup(reference).ok()?;
    let (target, target_resolver, target_draft) = resolved.into_inner();
    Some(ReferenceEdge {
        target: ValueIdentity::of(target),
        target_name: resolved_reference_name(reference, &target_resolver),
        target_schema: target,
        target_resolver,
        target_draft,
    })
}

fn collect_branch_refs<'r>(
    branch: &Value,
    resolver: &Resolver<'r>,
    draft: Draft,
    out: &mut Vec<ReferenceEdge<'r>>,
) {
    match resource_scope(branch, resolver, draft) {
        Some(scoped) => collect_composition_refs(branch, &scoped, draft, out, true),
        None => collect_composition_refs(branch, resolver, draft, out, true),
    }
}

/// `Some` when `schema` declares a non-fragment `$id` (Draft 4: `id`) opening a new resource scope.
fn resource_scope<'r>(
    schema: &Value,
    resolver: &Resolver<'r>,
    draft: Draft,
) -> Option<Resolver<'r>> {
    let resource = draft.create_resource_ref(schema);
    resource.id().filter(|id| !id.starts_with('#'))?;
    resolver.in_subresource(resource).ok()
}

fn definition_node_name(resolver: &Resolver<'_>, prefix: &str, name: &str) -> String {
    let mut node = resolver.base_uri().as_str().to_owned();
    node.push_str(&build_def_pointer(prefix, name));
    node
}

fn resolved_reference_name(reference: &str, target_resolver: &Resolver<'_>) -> String {
    match reference.split_once('#') {
        Some((_, fragment)) => format!("{}#{fragment}", target_resolver.base_uri().as_str()),
        None => target_resolver.base_uri().as_str().to_owned(),
    }
}

/// Returns the name of the first node on any cycle. Edges traverse composition keywords only, so every cycle in
/// the graph is unguarded.
fn find_cycle(graph: &ReferenceGraph) -> Option<String> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Mark {
        Unvisited,
        InStack,
        Done,
    }
    let mut state: AHashMap<ValueIdentity, Mark> = graph
        .edges
        .keys()
        .copied()
        .map(|key| (key, Mark::Unvisited))
        .collect();
    // Sort so the reported cycle node is deterministic.
    let mut nodes: Vec<ValueIdentity> = graph.edges.keys().copied().collect();
    nodes.sort_unstable_by(|left, right| graph.names.get(left).cmp(&graph.names.get(right)));
    for start in nodes {
        if state.get(&start).copied() != Some(Mark::Unvisited) {
            continue;
        }
        let mut stack: Vec<(ValueIdentity, usize)> = vec![(start, 0)];
        state.insert(start, Mark::InStack);
        while let Some(&(node, index)) = stack.last() {
            let outgoing = graph.edges.get(&node).map_or(&[][..], Vec::as_slice);
            if let Some(edge) = outgoing.get(index) {
                stack.last_mut().expect("non-empty by while-let").1 += 1;
                match state.get(&edge.target).copied() {
                    Some(Mark::InStack) => return Some(edge.target_name.clone()),
                    Some(Mark::Unvisited) => {
                        state.insert(edge.target, Mark::InStack);
                        stack.push((edge.target, 0));
                    }
                    Some(Mark::Done) | None => {}
                }
            } else {
                state.insert(node, Mark::Done);
                stack.pop();
            }
        }
    }
    None
}
