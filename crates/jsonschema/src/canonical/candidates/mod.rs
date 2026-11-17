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

/// The longest instance worth building. A bound past this is a bound no instance is built for: the
/// answer is left undecided rather than spending the memory a schema keyword asked for.
const CANDIDATE_LENGTH: u64 = 64;

/// How many instances one walk may build. Depth and width bound a candidate on their own, but a
/// node wide at every level multiplies them - 64 keys six levels down is a number of values no
/// machine holds - so the walk also spends from one count and gives up where it runs out.
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

/// Offer each instance worth trying against `node` to `visit`, shortest first, until it breaks. A
/// candidate proves nothing on its own - `algebra::admits_value` decides - so the sequence only has
/// to be short.
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
        SchemaKind::Array(leaf) => array::array(leaf.get(), &walk)
            .into_iter()
            .try_for_each(visit),
        SchemaKind::Object(leaf) => object::object(leaf.get(), &walk)
            .into_iter()
            .try_for_each(visit),
        // A branch's values are worth trying against the whole: `admits_value` decides.
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

    /// The first instance worth trying against `schema`, one level down. Only that one is built, so
    /// a wide schema leaves the walk's node count to the schemas after it.
    pub(super) fn first(&self, schema: &'a Schema) -> Option<Value> {
        let mut found = None;
        let _ = self.each(schema, &mut keep_first(&mut found));
        found
    }

    pub(super) fn ctx(&self) -> &CanonicalizationContext {
        self.ctx
    }
}

/// A visitor keeping the first candidate it is offered.
fn keep_first(found: &mut Option<Value>) -> impl FnMut(Value) -> ControlFlow<()> + '_ {
    |value| {
        *found = Some(value);
        ControlFlow::Break(())
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
