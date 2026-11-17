//! Instances worth trying against a node, for the questions a form alone cannot answer.
use std::cell::Cell;

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

/// Instances worth trying against `node`, shortest first. A candidate proves nothing on its own -
/// `algebra::admits_value` decides - so the list only has to be short.
pub(crate) fn instances<'a>(
    node: &'a Schema,
    target: &dyn Fn(&str) -> Option<&'a Schema>,
    depth: u32,
    budget: &Cell<u32>,
    ctx: &CanonicalizationContext,
) -> Vec<Value> {
    if depth == 0 || budget.get() == 0 {
        return Vec::new();
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
        SchemaKind::Reference(uri) => target(uri).map(|named| walk.all(named)).unwrap_or_default(),
        SchemaKind::True => vec![Value::Null],
        SchemaKind::Const(value) => vec![value.as_value().clone()],
        SchemaKind::Enum(values) => values
            .as_slice()
            .iter()
            .map(|value| value.as_value().clone())
            .collect(),
        SchemaKind::MultiType(set) => set.iter().map(shortest_instance).collect(),
        SchemaKind::TypedGroup { body, .. } => walk.all(body),
        SchemaKind::String(leaf) => string::string(leaf.get()),
        SchemaKind::Integer(leaf) => number::integer(leaf.get()),
        SchemaKind::Number(leaf) => number::number(leaf.get()),
        SchemaKind::Array(leaf) => array::array(leaf.get(), &walk),
        SchemaKind::Object(leaf) => object::object(leaf.get(), &walk),
        // A branch's values are worth trying against the whole: `admits_value` decides.
        SchemaKind::AllOf(branches) | SchemaKind::AnyOf(branches) => branches
            .as_slice()
            .iter()
            .flat_map(|branch| walk.all(branch))
            .collect(),
        SchemaKind::OneOf(branches) => branches
            .iter()
            .flat_map(|branch| walk.all(branch))
            .collect(),
        SchemaKind::False | SchemaKind::Not(_) | SchemaKind::Raw(_) => Vec::new(),
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
    /// Instances worth trying against `schema`, one level down.
    pub(super) fn all(&self, schema: &'a Schema) -> Vec<Value> {
        instances(schema, self.target, self.depth - 1, self.budget, self.ctx)
    }

    /// The first instance worth trying against `schema`, one level down.
    pub(super) fn first(&self, schema: &'a Schema) -> Option<Value> {
        self.all(schema).into_iter().next()
    }

    pub(super) fn ctx(&self) -> &CanonicalizationContext {
        self.ctx
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
