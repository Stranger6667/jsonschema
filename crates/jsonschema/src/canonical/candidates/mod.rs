//! Instances worth trying against a node, for the questions a form alone cannot answer.
use std::{cell::Cell, ops::ControlFlow};

use serde_json::Value;

use crate::{
    canonical::{
        context::CanonicalizationContext,
        ir::{BoundCardinality, Schema, SchemaKind},
    },
    JsonType,
};

mod array;
mod number;
mod object;
mod string;

/// How far a candidate walks into a node before giving up.
pub(crate) const DEPTH: u32 = 6;

/// The longest instance worth building; past it the answer is left undecided instead of
/// allocating what a keyword asks for.
const CANDIDATE_LENGTH: u64 = 64;

/// Nodes one walk may visit. Depth and width bound a candidate on their own, but a node wide at
/// every level multiplies them, so the walk also spends from one count and gives up where it runs
/// out.
pub(crate) const NODES: u32 = 4_096;

/// The length a leaf demands, or `None` where no instance that long is worth building.
fn demanded_length(minimum: Option<&BoundCardinality>) -> Option<usize> {
    let Some(minimum) = minimum else {
        return Some(0);
    };
    minimum
        .to_usize()
        .filter(|length| *length as u64 <= CANDIDATE_LENGTH)
}

/// Where a candidate goes: the consumer breaks once it has what it asked for, and no later
/// candidate is built.
pub(crate) type Visit<'v> = dyn FnMut(Value) -> ControlFlow<()> + 'v;

/// Offer each instance worth trying against `node` to `visit` until it breaks. A candidate proves
/// nothing on its own - `algebra::accepts_value` decides - so the sequence only has to be short.
pub(crate) fn search<'a>(
    node: &'a Schema,
    target: &dyn Fn(&str) -> Option<&'a Schema>,
    depth: u32,
    budget: &Cell<u32>,
    ctx: &CanonicalizationContext,
    visit: &mut Visit<'_>,
) -> ControlFlow<()> {
    if depth == 0 || budget.get() == 0 {
        return ControlFlow::Continue(());
    }
    budget.set(budget.get() - 1);
    let walk = Walk {
        target,
        depth,
        budget,
        ctx,
    };
    match node.kind() {
        // A pointer accepts what its target does, so the instances worth trying are the target's.
        SchemaKind::Reference(uri) => match target(uri) {
            Some(named) => walk.each(named, visit),
            None => ControlFlow::Continue(()),
        },
        SchemaKind::True => visit(Value::Null),
        SchemaKind::Const(value) => visit(value.as_value().clone()),
        SchemaKind::Enum(values) => values
            .as_slice()
            .iter()
            .try_for_each(|value| visit(value.as_value().clone())),
        SchemaKind::MultiType(set) => set.iter().try_for_each(|ty| visit(shortest_instance(ty))),
        SchemaKind::TypedGroup { body, .. } => walk.each(body, visit),
        SchemaKind::String(leaf) => string::string(leaf.get()).into_iter().try_for_each(visit),
        SchemaKind::Integer(leaf) => number::integer(leaf.get(), visit),
        SchemaKind::Number(leaf) => number::number(leaf.get(), visit),
        SchemaKind::Array(leaf) => array::array(leaf.get(), &walk, visit),
        SchemaKind::Object(leaf) => object::object(leaf.get(), &walk, visit),
        // A branch's values are worth trying against the whole: `accepts_value` decides.
        SchemaKind::AllOf(branches) | SchemaKind::AnyOf(branches) => branches
            .as_slice()
            .iter()
            .try_for_each(|branch| walk.each(branch, visit)),
        SchemaKind::OneOf(branches) => branches
            .iter()
            .try_for_each(|branch| walk.each(branch, visit)),
        SchemaKind::False | SchemaKind::Not(_) | SchemaKind::Raw(_) => ControlFlow::Continue(()),
    }
}

/// One step of a candidate walk: what a kind needs to build instances of the schemas below it.
pub(super) struct Walk<'a, 'c> {
    target: &'c dyn Fn(&str) -> Option<&'a Schema>,
    depth: u32,
    budget: &'c Cell<u32>,
    ctx: &'c CanonicalizationContext,
}

impl<'a> Walk<'a, '_> {
    /// Offer the instances worth trying against `schema`, one level down, to `visit`.
    pub(super) fn each(&self, schema: &'a Schema, visit: &mut Visit<'_>) -> ControlFlow<()> {
        search(
            schema,
            self.target,
            self.depth - 1,
            self.budget,
            self.ctx,
            visit,
        )
    }

    /// Offer the instances worth trying against `schema`, a schema built during the walk, one level
    /// down, to `visit`.
    pub(super) fn each_owned(&self, schema: &Schema, visit: &mut Visit<'_>) -> ControlFlow<()> {
        let target = |uri: &str| -> Option<&Schema> { (self.target)(uri) };
        search(
            schema,
            &target,
            self.depth - 1,
            self.budget,
            self.ctx,
            visit,
        )
    }

    /// The first instance worth trying against `schema`, one level down. Only that one is built, so
    /// a wide schema leaves the walk's node count to the schemas after it.
    pub(super) fn first(&self, schema: &'a Schema) -> Option<Value> {
        let mut found = None;
        let _ = self.each(schema, &mut keep_first(&mut found));
        found
    }

    /// The first instance worth trying against `schema`, a schema built during the walk.
    pub(super) fn first_owned(&self, schema: &Schema) -> Option<Value> {
        let mut found = None;
        let _ = self.each_owned(schema, &mut keep_first(&mut found));
        found
    }

    pub(super) fn ctx(&self) -> &CanonicalizationContext {
        self.ctx
    }
}

/// Whether a check the walk makes to build a candidate holds. The walk only suggests values, so what
/// such a check approximates must not mark the run it serves; an approximated answer is `false`.
pub(super) fn holds(ctx: &CanonicalizationContext, check: impl FnOnce() -> bool) -> bool {
    let (answer, inexact) = ctx.probe(check);
    answer && !inexact
}

/// A visitor keeping the first candidate it is offered.
fn keep_first(found: &mut Option<Value>) -> impl FnMut(Value) -> ControlFlow<()> + '_ {
    |value| {
        *found = Some(value);
        ControlFlow::Break(())
    }
}

/// A visitor keeping the first candidate `accept` takes, giving up after `attempts` candidates.
pub(super) fn keep_accepted<'f>(
    found: &'f mut Option<Value>,
    mut attempts: usize,
    mut accept: impl FnMut(&Value) -> bool + 'f,
) -> impl FnMut(Value) -> ControlFlow<()> + 'f {
    debug_assert!(attempts > 0, "a search tries at least one candidate");
    move |value| {
        if accept(&value) {
            *found = Some(value);
            return ControlFlow::Break(());
        }
        attempts -= 1;
        if attempts == 0 {
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    }
}

/// The shortest instance of a type.
fn shortest_instance(ty: JsonType) -> Value {
    match ty {
        JsonType::Null => Value::Null,
        JsonType::Boolean => Value::Bool(false),
        JsonType::String => Value::String(String::new()),
        JsonType::Integer | JsonType::Number => Value::Number(0.into()),
        JsonType::Array => Value::Array(Vec::new()),
        JsonType::Object => Value::Object(serde_json::Map::new()),
    }
}
