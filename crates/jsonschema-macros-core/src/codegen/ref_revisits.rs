//! The `$ref` cycles that one validation can take through the same instance more than once.
//!
//! A cycle reached from two keywords that apply to one instance validates that instance once per
//! path to it, so the work doubles at every level of the cycle. Only those cycles keep the
//! results of their calls for the run.
//!
//! A cycle that comes back to the instance it holds would never end; the runtime stops a target
//! re-entered on one instance, wherever a `$ref` in a cycle passed that instance to it first.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    ops::Range,
    sync::Arc,
};

use referencing::{Draft, Uri};
use serde_json::{Map, Value};

use super::{
    draft::DraftExt,
    emit::ValueEmitter,
    keywords::one_of::{branch_discriminators, DiscriminatorLiteral},
    refs::{resolve_lone_top_level_ref, resolve_ref},
    CompileContext,
};

/// The `$ref` targets of the schema's cycles that one validation reaches more than once on one
/// instance.
pub(crate) struct RefCycles {
    /// Targets in cycles that one validation can take through the same instance more than once.
    pub(crate) revisited: HashSet<String>,
    /// Targets in cycles that come back to the instance they hold.
    pub(crate) same_instance: HashSet<String>,
    /// The schema has `$recursiveRef` or `$dynamicRef`, so a target's result can depend on the
    /// dynamic scope it runs in.
    pub(crate) through_scope: bool,
}

pub(crate) fn ref_cycles<'cfg, E: ValueEmitter>(
    ctx: &mut CompileContext<'cfg, E>,
    schema: &'cfg Value,
) -> RefCycles {
    let base_uri = ctx.current_base_uri.clone();
    let draft = ctx.draft;
    let graph = Graph::build(ctx, schema);
    let revisited = graph.revisited(ctx);
    ctx.current_base_uri = base_uri;
    ctx.draft = draft;
    RefCycles {
        revisited,
        same_instance: graph.same_instance_cycles(),
        through_scope: graph.dynamic,
    }
}

/// Where a subschema applies, relative to the instance of the schema that holds it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Step {
    Same,
    Property(usize),
    AnyProperty,
    Item,
}

impl Step {
    /// Whether both can lead to the same instance.
    fn overlaps(self, other: Self) -> bool {
        match (self, other) {
            (Step::Property(first), Step::Property(second)) => first == second,
            (Step::Same, Step::Same)
            | (Step::Property(_) | Step::AnyProperty, Step::AnyProperty)
            | (Step::AnyProperty, Step::Property(_))
            | (Step::Item, Step::Item) => true,
            _ => false,
        }
    }
}

/// The keyword position a subschema holds in its schema.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Same,
    Then,
    Else,
    Property(usize),
    PatternProperty,
    AdditionalProperty,
    UnevaluatedProperty,
    PrefixItem(usize),
    RestItem,
    Contains,
    UnevaluatedItem,
}

impl Slot {
    fn step(self) -> Step {
        match self {
            Slot::Same | Slot::Then | Slot::Else => Step::Same,
            Slot::Property(name) => Step::Property(name),
            Slot::PatternProperty | Slot::AdditionalProperty | Slot::UnevaluatedProperty => {
                Step::AnyProperty
            }
            Slot::PrefixItem(_) | Slot::RestItem | Slot::Contains | Slot::UnevaluatedItem => {
                Step::Item
            }
        }
    }

    /// Whether two positions in one schema can apply their subschemas to the same instance. A
    /// failing subschema ends the schema's validation before its `unevaluated*` keyword runs.
    fn overlaps(self, other: Self) -> bool {
        match (self, other) {
            (Slot::Then, Slot::Else) | (Slot::Else, Slot::Then) => false,
            (Slot::Same | Slot::Then | Slot::Else, _)
            | (_, Slot::Same | Slot::Then | Slot::Else)
            | (Slot::Property(_), Slot::PatternProperty)
            | (Slot::PatternProperty, Slot::Property(_) | Slot::PatternProperty)
            | (
                Slot::Contains,
                Slot::PrefixItem(_) | Slot::RestItem | Slot::UnevaluatedItem | Slot::Contains,
            )
            | (Slot::PrefixItem(_) | Slot::RestItem | Slot::UnevaluatedItem, Slot::Contains) => {
                true
            }
            _ => false,
        }
    }
}

const NULL: u8 = 1;
const BOOLEAN: u8 = 1 << 1;
const OBJECT: u8 = 1 << 2;
const ARRAY: u8 = 1 << 3;
const NUMBER: u8 = 1 << 4;
const STRING: u8 = 1 << 5;
const ANY_TYPE: u8 = NULL | BOOLEAN | OBJECT | ARRAY | NUMBER | STRING;

/// The instance types a schema can accept. Its `type` check runs before its subschemas, so a
/// schema never descends into an instance of another type.
fn accepted_types(schema: &Value) -> u8 {
    let type_bit = |name: &str| match name {
        "null" => NULL,
        "boolean" => BOOLEAN,
        "object" => OBJECT,
        "array" => ARRAY,
        "number" | "integer" => NUMBER,
        "string" => STRING,
        _ => ANY_TYPE,
    };
    match schema {
        Value::Bool(false) => 0,
        Value::Object(object) => match object.get("type") {
            Some(Value::String(name)) => type_bit(name),
            Some(Value::Array(names)) => names
                .iter()
                .map(|name| name.as_str().map_or(ANY_TYPE, type_bit))
                .fold(0, |types, bit| types | bit),
            _ => ANY_TYPE,
        },
        _ => ANY_TYPE,
    }
}

/// A subschema, or a `$ref` target, a schema applies.
#[derive(Clone, Copy)]
enum Applied {
    Node(usize),
    Target(usize),
}

struct Child<'cfg> {
    slot: Slot,
    applied: Applied,
    schema: &'cfg Value,
    branch: Option<Branch>,
}

/// An `anyOf` or `oneOf` branch.
struct Branch {
    group: usize,
    position: usize,
}

/// The branches of one `anyOf` or `oneOf`.
struct Group<'cfg> {
    branches: &'cfg [Value],
    one_of: bool,
    env: usize,
}

/// What a schema's references resolve against.
struct Env {
    base_uri: Arc<Uri<String>>,
    draft: Draft,
}

struct Node<'cfg> {
    children: Vec<Child<'cfg>>,
    env: usize,
}

/// One target per distinct `$ref` location, plus the root schema at index 0.
struct Graph<'cfg> {
    nodes: Vec<Node<'cfg>>,
    envs: Vec<Env>,
    env: usize,
    groups: Vec<Group<'cfg>>,
    locations: Vec<String>,
    /// The nodes of each target's schema, its root first.
    spans: Vec<Range<usize>>,
    targets: HashMap<String, usize>,
    /// The target of each reference, by the base it resolves against.
    resolved: HashMap<String, HashMap<&'cfg str, Option<usize>>>,
    names: HashMap<&'cfg str, usize>,
    dynamic: bool,
}

impl<'cfg> Graph<'cfg> {
    fn build<E: ValueEmitter>(ctx: &mut CompileContext<'cfg, E>, schema: &'cfg Value) -> Self {
        let mut graph = Graph {
            nodes: Vec::new(),
            envs: Vec::new(),
            env: 0,
            groups: Vec::new(),
            locations: vec![String::new()],
            spans: Vec::new(),
            targets: HashMap::new(),
            resolved: HashMap::new(),
            names: HashMap::new(),
            dynamic: false,
        };
        graph.spans.push(0..0);
        let mut pending = vec![Pending {
            target: 0,
            schema,
            base_uri: ctx.current_base_uri.clone(),
            draft: ctx.draft,
        }];
        while let Some(next) = pending.pop() {
            ctx.current_base_uri = next.base_uri;
            ctx.draft = next.draft;
            graph.enter(ctx);
            let start = graph.node(ctx, next.schema, &mut pending);
            graph.spans[next.target] = start..graph.nodes.len();
        }
        graph
    }

    /// Makes the context's base and draft the ones new nodes resolve against.
    fn enter<E: ValueEmitter>(&mut self, ctx: &CompileContext<'cfg, E>) {
        self.env = self.envs.len();
        self.envs.push(Env {
            base_uri: ctx.current_base_uri.clone(),
            draft: ctx.draft,
        });
    }

    fn node<E: ValueEmitter>(
        &mut self,
        ctx: &mut CompileContext<'cfg, E>,
        schema: &'cfg Value,
        pending: &mut Vec<Pending<'cfg>>,
    ) -> usize {
        let index = self.nodes.len();
        self.nodes.push(Node {
            children: Vec::new(),
            env: self.env,
        });
        let Value::Object(object) = schema else {
            return index;
        };
        let ignored_siblings = (!ctx.draft.supports_adjacent_validation())
            .then(|| object.get("$ref"))
            .flatten();
        let children = if let Some(reference) = ignored_siblings {
            self.reference(ctx, reference, pending)
                .into_iter()
                .collect()
        } else {
            let id_key = if ctx.draft == Draft::Draft4 {
                "id"
            } else {
                "$id"
            };
            let resource = object
                .get(id_key)
                .and_then(Value::as_str)
                .and_then(|id| {
                    ctx.config
                        .registry
                        .resolver((*ctx.current_base_uri).clone())
                        .lookup(id)
                        .ok()
                })
                .map(|resolved| resolved.resolver().base_uri().clone());
            match resource {
                Some(base_uri) => {
                    let outer = (
                        std::mem::replace(&mut ctx.current_base_uri, base_uri),
                        self.env,
                    );
                    self.enter(ctx);
                    self.nodes[index].env = self.env;
                    let children = self.children(ctx, object, pending);
                    (ctx.current_base_uri, self.env) = outer;
                    children
                }
                None => self.children(ctx, object, pending),
            }
        };
        self.nodes[index].children = children;
        index
    }

    fn reference<E: ValueEmitter>(
        &mut self,
        ctx: &mut CompileContext<'cfg, E>,
        schema: &'cfg Value,
        pending: &mut Vec<Pending<'cfg>>,
    ) -> Option<Child<'cfg>> {
        let reference = schema.as_str()?;
        let cached = self
            .resolved
            .get(ctx.current_base_uri.as_str())
            .and_then(|references| references.get(reference));
        let target = if let Some(&target) = cached {
            target
        } else {
            let target = self.resolve(ctx, reference, pending);
            self.resolved
                .entry(ctx.current_base_uri.to_string())
                .or_default()
                .insert(reference, target);
            target
        }?;
        Some(Child {
            slot: Slot::Same,
            applied: Applied::Target(target),
            schema,
            branch: None,
        })
    }

    fn resolve<E: ValueEmitter>(
        &mut self,
        ctx: &mut CompileContext<'cfg, E>,
        reference: &str,
        pending: &mut Vec<Pending<'cfg>>,
    ) -> Option<usize> {
        let resolved = resolve_ref(ctx, reference).ok()?;
        if let Some(&target) = self.targets.get(&resolved.location) {
            return Some(target);
        }
        let target = self.spans.len();
        self.spans.push(0..0);
        self.locations.push(resolved.location.clone());
        self.targets.insert(resolved.location, target);
        let draft = match resolved.schema.get("$schema") {
            Some(dialect) => ctx.draft.detect(&serde_json::json!({ "$schema": dialect })),
            None => ctx.draft,
        };
        pending.push(Pending {
            target,
            schema: resolved.schema,
            base_uri: resolved.base_uri,
            draft,
        });
        Some(target)
    }

    fn children<E: ValueEmitter>(
        &mut self,
        ctx: &mut CompileContext<'cfg, E>,
        object: &'cfg Map<String, Value>,
        pending: &mut Vec<Pending<'cfg>>,
    ) -> Vec<Child<'cfg>> {
        let conditional = ctx.draft.supports_if_then_else_keyword() && object.contains_key("if");
        let mut children = Vec::new();
        for (keyword, value) in object {
            match keyword.as_str() {
                "$recursiveRef" | "$dynamicRef" => {
                    self.dynamic = true;
                    // Without an anchor in its dynamic scope, it applies the schema it names.
                    children.extend(self.reference(ctx, value, pending));
                }
                "$ref" => children.extend(self.reference(ctx, value, pending)),
                "allOf" | "anyOf" | "oneOf" => {
                    let Value::Array(branches) = value else {
                        continue;
                    };
                    let group = (keyword != "allOf").then(|| {
                        self.groups.push(Group {
                            branches,
                            one_of: keyword == "oneOf",
                            env: self.env,
                        });
                        self.groups.len() - 1
                    });
                    for (position, branch) in branches.iter().enumerate() {
                        let mut child = self.child(ctx, Slot::Same, branch, pending);
                        child.branch = group.map(|group| Branch { group, position });
                        children.push(child);
                    }
                }
                "not" => children.push(self.child(ctx, Slot::Same, value, pending)),
                "if" if conditional => children.push(self.child(ctx, Slot::Same, value, pending)),
                "then" if conditional => children.push(self.child(ctx, Slot::Then, value, pending)),
                "else" if conditional => children.push(self.child(ctx, Slot::Else, value, pending)),
                "dependencies" | "dependentSchemas" => {
                    let Value::Object(dependencies) = value else {
                        continue;
                    };
                    for schema in dependencies.values().filter(|value| !value.is_array()) {
                        children.push(self.child(ctx, Slot::Same, schema, pending));
                    }
                }
                "properties" => {
                    let Value::Object(properties) = value else {
                        continue;
                    };
                    for (name, schema) in properties {
                        let next = self.names.len();
                        let name = *self.names.entry(name.as_str()).or_insert(next);
                        children.push(self.child(ctx, Slot::Property(name), schema, pending));
                    }
                }
                "patternProperties" => {
                    let Value::Object(patterns) = value else {
                        continue;
                    };
                    for schema in patterns.values() {
                        children.push(self.child(ctx, Slot::PatternProperty, schema, pending));
                    }
                }
                "additionalProperties" => {
                    children.push(self.child(ctx, Slot::AdditionalProperty, value, pending));
                }
                "unevaluatedProperties" => {
                    children.push(self.child(ctx, Slot::UnevaluatedProperty, value, pending));
                }
                "additionalItems" => children.push(self.child(ctx, Slot::RestItem, value, pending)),
                "contains" => children.push(self.child(ctx, Slot::Contains, value, pending)),
                "unevaluatedItems" => {
                    children.push(self.child(ctx, Slot::UnevaluatedItem, value, pending));
                }
                "prefixItems" | "items" => match value {
                    Value::Array(items) => {
                        for (position, schema) in items.iter().enumerate() {
                            children.push(self.child(
                                ctx,
                                Slot::PrefixItem(position),
                                schema,
                                pending,
                            ));
                        }
                    }
                    schema if keyword == "items" => {
                        children.push(self.child(ctx, Slot::RestItem, schema, pending));
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        children
    }

    fn child<E: ValueEmitter>(
        &mut self,
        ctx: &mut CompileContext<'cfg, E>,
        slot: Slot,
        schema: &'cfg Value,
        pending: &mut Vec<Pending<'cfg>>,
    ) -> Child<'cfg> {
        Child {
            slot,
            applied: Applied::Node(self.node(ctx, schema, pending)),
            schema,
            branch: None,
        }
    }

    fn revisited<E: ValueEmitter>(&self, ctx: &mut CompileContext<'cfg, E>) -> HashSet<String> {
        let edges: Vec<Vec<usize>> = (0..self.spans.len())
            .map(|target| self.targets_applied(target).collect())
            .collect();
        let components = components(&edges);
        let mut component_of = vec![0; self.spans.len()];
        for (component, members) in components.iter().enumerate() {
            for &member in members {
                component_of[member] = component;
            }
        }
        let cyclic: Vec<bool> = components
            .iter()
            .map(|members| members.len() > 1 || edges[members[0]].contains(&members[0]))
            .collect();
        if !cyclic.contains(&true) {
            return HashSet::new();
        }
        // Whether each target, then each node, leads into a cycle; dependencies come first.
        let mut leads = vec![false; self.spans.len()];
        for (component, members) in components.iter().enumerate() {
            let value = cyclic[component]
                || members
                    .iter()
                    .flat_map(|&member| &edges[member])
                    .any(|&applied| leads[applied]);
            for &member in members {
                leads[member] = value;
            }
        }
        let mut node_leads = vec![false; self.nodes.len()];
        for span in &self.spans {
            for node in span.clone().rev() {
                node_leads[node] =
                    self.nodes[node]
                        .children
                        .iter()
                        .any(|child| match child.applied {
                            Applied::Node(nested) => node_leads[nested],
                            Applied::Target(target) => leads[target],
                        });
            }
        }
        let candidates = self.candidates(ctx, &leads, &node_leads);
        if candidates.is_empty() {
            return HashSet::new();
        }
        let mut reaches = Reaches {
            graph: self,
            component_of,
            cyclic,
            target: vec![Vec::new(); self.spans.len()],
            node: vec![Vec::new(); self.nodes.len()],
        };
        let mut dependents = vec![Vec::new(); self.spans.len()];
        for (target, applied) in edges.iter().enumerate() {
            for &applied in applied {
                dependents[applied].push(target);
            }
        }
        // Components come dependencies first, so each converges on its own, re-walking only the
        // members that apply a target whose reach grew.
        let mut queued = vec![false; self.spans.len()];
        for (component, members) in components.iter().enumerate() {
            let mut queue: VecDeque<usize> = members.iter().copied().collect();
            for &member in members {
                queued[member] = true;
            }
            while let Some(member) = queue.pop_front() {
                queued[member] = false;
                reaches.walk(member);
                let root = self.spans[member].start;
                if reaches.node[root] != reaches.target[member] {
                    reaches.target[member].clone_from(&reaches.node[root]);
                    for &dependent in &dependents[member] {
                        if reaches.component_of[dependent] == component && !queued[dependent] {
                            queued[dependent] = true;
                            queue.push_back(dependent);
                        }
                    }
                }
            }
        }
        let mut target_of = vec![0; self.nodes.len()];
        for (target, span) in self.spans.iter().enumerate() {
            for node in span.clone() {
                target_of[node] = target;
            }
        }
        let revisited: HashSet<usize> = candidates
            .iter()
            .filter_map(|candidate| {
                let component = reaches.component_of[target_of[candidate.node]];
                let children = &self.nodes[candidate.node].children;
                reaches
                    .overlap_into(
                        &children[candidate.first],
                        &children[candidate.second],
                        component,
                    )
                    .then_some(component)
            })
            .collect();
        (1..self.spans.len())
            .filter(|&target| revisited.contains(&reaches.component_of[target]))
            .map(|target| self.locations[target].clone())
            .collect()
    }

    /// The pairs of subschemas of one schema that both lead into a cycle and can both run on the
    /// same instance.
    fn candidates<E: ValueEmitter>(
        &self,
        ctx: &mut CompileContext<'cfg, E>,
        leads: &[bool],
        node_leads: &[bool],
    ) -> Vec<Candidate> {
        let mut candidates = Vec::new();
        let mut discriminators = HashMap::new();
        let mut accepted = Vec::new();
        let mut leading = Vec::new();
        for (index, node) in self.nodes.iter().enumerate() {
            let children = &node.children;
            leading.clear();
            leading.extend(
                children
                    .iter()
                    .enumerate()
                    .filter(|(_, child)| match child.applied {
                        Applied::Node(nested) => node_leads[nested],
                        Applied::Target(target) => leads[target],
                    })
                    .map(|(position, _)| position),
            );
            if leading.len() < 2 {
                continue;
            }
            accepted.clear();
            accepted.resize(children.len(), None);
            for (offset, &first) in leading.iter().enumerate() {
                for &second in &leading[offset + 1..] {
                    if children[first].slot.overlaps(children[second].slot)
                        && !self.discriminated(
                            ctx,
                            &children[first],
                            &children[second],
                            &mut discriminators,
                        )
                        && self.accepted(ctx, node, &mut accepted, first)
                            & self.accepted(ctx, node, &mut accepted, second)
                            != 0
                    {
                        candidates.push(Candidate {
                            node: index,
                            first,
                            second,
                        });
                    }
                }
            }
        }
        candidates
    }

    /// Whether two `oneOf` branches run for disjoint discriminator values.
    fn discriminated<E: ValueEmitter>(
        &self,
        ctx: &mut CompileContext<'cfg, E>,
        one: &Child<'cfg>,
        other: &Child<'cfg>,
        discriminators: &mut HashMap<usize, Vec<Option<Vec<DiscriminatorLiteral>>>>,
    ) -> bool {
        let (Some(one), Some(other)) = (&one.branch, &other.branch) else {
            return false;
        };
        let group = &self.groups[one.group];
        if one.group != other.group || !group.one_of {
            return false;
        }
        let values = discriminators.entry(one.group).or_insert_with(|| {
            self.envs[group.env].apply(ctx);
            branch_discriminators(ctx, group.branches)
        });
        match (&values[one.position], &values[other.position]) {
            (Some(one), Some(other)) => one.iter().all(|value| !other.contains(value)),
            _ => false,
        }
    }

    /// The types of the holding instance from which a child can descend further, computed once.
    fn accepted<E: ValueEmitter>(
        &self,
        ctx: &mut CompileContext<'cfg, E>,
        node: &Node<'cfg>,
        accepted: &mut [Option<u8>],
        position: usize,
    ) -> u8 {
        if let Some(types) = accepted[position] {
            return types;
        }
        let child = &node.children[position];
        let types = match (child.slot.step(), child.applied) {
            (Step::Same, Applied::Node(_)) => {
                self.envs[node.env].apply(ctx);
                accepted_types(resolve_lone_top_level_ref(ctx, child.schema))
            }
            (Step::Same, Applied::Target(_)) => ANY_TYPE,
            (Step::Property(_) | Step::AnyProperty, _) => OBJECT,
            (Step::Item, _) => ARRAY,
        };
        accepted[position] = Some(types);
        types
    }

    /// The targets a target's schema applies directly.
    fn targets_applied(&self, target: usize) -> impl Iterator<Item = usize> + '_ {
        self.nodes[self.spans[target].clone()]
            .iter()
            .flat_map(|node| &node.children)
            .filter_map(|child| match child.applied {
                Applied::Target(target) => Some(target),
                Applied::Node(_) => None,
            })
    }

    /// The targets on cycles of subschemas that apply to the instance of the schema holding them.
    fn same_instance_cycles(&self) -> HashSet<String> {
        let edges: Vec<Vec<usize>> = (0..self.spans.len())
            .map(|target| self.targets_applied_in_place(target))
            .collect();
        components(&edges)
            .into_iter()
            .filter(|members| members.len() > 1 || edges[members[0]].contains(&members[0]))
            .flatten()
            .map(|target| self.locations[target].clone())
            .collect()
    }

    /// The targets a target's schema applies to its own instance.
    fn targets_applied_in_place(&self, target: usize) -> Vec<usize> {
        let mut applied = Vec::new();
        let mut nodes = vec![self.spans[target].start];
        while let Some(node) = nodes.pop() {
            for child in &self.nodes[node].children {
                if child.slot.step() != Step::Same {
                    continue;
                }
                match child.applied {
                    Applied::Node(nested) => nodes.push(nested),
                    Applied::Target(target) => applied.push(target),
                }
            }
        }
        applied
    }
}

/// Strongly connected targets, each component after every component it reaches.
fn components(edges: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let mut tarjan = Tarjan {
        edges,
        index: vec![None; edges.len()],
        low: vec![0; edges.len()],
        on_stack: vec![false; edges.len()],
        stack: Vec::new(),
        next: 0,
        components: Vec::new(),
    };
    for target in 0..edges.len() {
        if tarjan.index[target].is_none() {
            tarjan.visit(target);
        }
    }
    tarjan.components
}

/// A target whose schema is not walked yet, with the environment it is walked in.
struct Pending<'cfg> {
    target: usize,
    schema: &'cfg Value,
    base_uri: Arc<Uri<String>>,
    draft: Draft,
}

struct Tarjan<'a> {
    edges: &'a [Vec<usize>],
    index: Vec<Option<usize>>,
    low: Vec<usize>,
    on_stack: Vec<bool>,
    stack: Vec<usize>,
    next: usize,
    components: Vec<Vec<usize>>,
}

impl Tarjan<'_> {
    fn visit(&mut self, target: usize) {
        self.index[target] = Some(self.next);
        self.low[target] = self.next;
        self.next += 1;
        self.stack.push(target);
        self.on_stack[target] = true;
        for &next in &self.edges[target] {
            match self.index[next] {
                None => {
                    self.visit(next);
                    self.low[target] = self.low[target].min(self.low[next]);
                }
                Some(index) if self.on_stack[next] => {
                    self.low[target] = self.low[target].min(index);
                }
                Some(_) => {}
            }
        }
        if Some(self.low[target]) == self.index[target] {
            let mut component = Vec::new();
            loop {
                let member = self.stack.pop().expect("the target is on the stack");
                self.on_stack[member] = false;
                component.push(member);
                if member == target {
                    break;
                }
            }
            self.components.push(component);
        }
    }
}

/// The cycles each schema leads to, and the first step it takes to get there.
struct Reaches<'a, 'cfg> {
    graph: &'a Graph<'cfg>,
    component_of: Vec<usize>,
    cyclic: Vec<bool>,
    target: Vec<Vec<(usize, Step)>>,
    node: Vec<Vec<(usize, Step)>>,
}

impl Reaches<'_, '_> {
    /// Recomputes the reach of each node of a target's schema, innermost first.
    fn walk(&mut self, target: usize) {
        for node in self.graph.spans[target].clone().rev() {
            let children = &self.graph.nodes[node].children;
            let size = children
                .iter()
                .map(|child| {
                    let (inner, own) = self.inner(child);
                    inner.len() + usize::from(own.is_some())
                })
                .sum();
            if size == 0 {
                self.node[node] = Vec::new();
                continue;
            }
            let mut reach = Vec::with_capacity(size);
            for child in children {
                self.extend(child, &mut reach);
            }
            // Each child adds a sorted run, which a stable sort merges.
            if !reach.is_sorted() {
                reach.sort();
            }
            reach.dedup();
            self.node[node] = reach;
        }
    }

    /// Where the subschema itself leads, and the cycle of the target it is, if any.
    fn inner(&self, child: &Child<'_>) -> (&[(usize, Step)], Option<usize>) {
        match child.applied {
            Applied::Node(node) => (&self.node[node], None),
            Applied::Target(target) => {
                let component = self.component_of[target];
                (
                    &self.target[target],
                    self.cyclic[component].then_some(component),
                )
            }
        }
    }

    /// Adds where a child leads, relative to the instance of the schema that holds it.
    fn extend(&self, child: &Child<'_>, reach: &mut Vec<(usize, Step)>) {
        let (inner, own) = self.inner(child);
        match child.slot.step() {
            Step::Same => reach.extend_from_slice(inner),
            step => {
                let mut previous = None;
                for &(component, _) in inner {
                    if previous != Some(component) {
                        reach.push((component, step));
                        previous = Some(component);
                    }
                }
            }
        }
        if let Some(component) = own {
            reach.push((component, child.slot.step()));
        }
    }

    /// Whether two subschemas of one schema both lead into `component` through the same instance.
    fn overlap_into(&self, first: &Child<'_>, second: &Child<'_>, component: usize) -> bool {
        match (
            self.steps_into(first, component),
            self.steps_into(second, component),
        ) {
            (Some(first), Some(second)) => first.overlaps(&second),
            _ => false,
        }
    }

    /// The first steps a child takes on its way into `component`, if it gets there.
    fn steps_into<'s>(&'s self, child: &Child<'_>, component: usize) -> Option<Steps<'s>> {
        let (inner, own) = self.inner(child);
        let start = inner.partition_point(|&(reached, _)| reached < component);
        let end = start + inner[start..].partition_point(|&(reached, _)| reached == component);
        let step = child.slot.step();
        let steps = match step {
            // The subschema applies to the same instance, so its own first steps are the child's.
            Step::Same => Steps {
                inner: &inner[start..end],
                own: (own == Some(component)).then_some(step),
            },
            step => Steps {
                inner: &[],
                own: (start < end || own == Some(component)).then_some(step),
            },
        };
        (!steps.inner.is_empty() || steps.own.is_some()).then_some(steps)
    }
}

/// Two subschemas of one schema that can both run on the same instance.
struct Candidate {
    node: usize,
    first: usize,
    second: usize,
}

/// The first steps a subschema takes into a cycle.
struct Steps<'a> {
    /// Entries of the cycle, when the subschema applies to the same instance.
    inner: &'a [(usize, Step)],
    own: Option<Step>,
}

impl Steps<'_> {
    fn iter(&self) -> impl Iterator<Item = Step> + '_ {
        self.inner.iter().map(|&(_, step)| step).chain(self.own)
    }

    fn overlaps(&self, other: &Steps<'_>) -> bool {
        self.iter()
            .any(|step| other.iter().any(|other| step.overlaps(other)))
    }
}

impl Env {
    fn apply<E: ValueEmitter>(&self, ctx: &mut CompileContext<'_, E>) {
        ctx.current_base_uri = self.base_uri.clone();
        ctx.draft = self.draft;
    }
}
