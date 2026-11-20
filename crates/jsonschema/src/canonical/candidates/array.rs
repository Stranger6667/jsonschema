//! Candidates for an array leaf.
use std::ops::ControlFlow;

use serde_json::Value;

use super::{demanded_length, holds, keep_accepted, Visit, Walk};
use crate::{
    canonical::{
        algebra,
        ir::{ArrayLeaf, ContainsFacet, Schema, SchemaKind, UncheckableFacet, Verdict},
    },
    JsonType,
};

/// Candidates checked per search for an element valid under a `contains` demand and the schema of
/// the index it sits at.
const DEMAND_ATTEMPTS: usize = 8;

/// Branches of a union demand intersected with the index schema one at a time before the whole
/// union is.
const DEMAND_BRANCHES: usize = 8;

/// Instances worth trying against an array leaf: one array as long as the leaf demands.
pub(super) fn array<'a>(
    leaf: &'a ArrayLeaf,
    walk: &Walk<'a, '_>,
    visit: &mut Visit<'_>,
) -> ControlFlow<()> {
    match build(leaf, walk) {
        Some(array) => visit(array),
        None => ControlFlow::Continue(()),
    }
}

/// The array as long as the leaf demands, or `None` where an element cannot be built.
fn build<'a>(leaf: &'a ArrayLeaf, walk: &Walk<'a, '_>) -> Option<Value> {
    let length = demanded_length(leaf.lengths.minimum.as_ref())?;
    // A `contains` demand needs an element valid under it, which `minItems` alone never asks
    // for; each demand contributes one, and the caller's check turns down what does not hold.
    let demanded: Vec<Value> = leaf
        .contains
        .iter()
        .map(|facet| demanded_element(facet, leaf.items.as_ref(), walk))
        .collect::<Option<_>>()?;
    let element = |index: usize| {
        let schema = leaf.prefix.get(index).or(leaf.items.as_ref());
        schema.map_or(Some(Value::Null), |schema| walk.first(schema))
    };
    // A prefix schema governs the index it names, so demanded elements sit past the whole
    // prefix rather than at the head.
    let mut items = Vec::new();
    for index in 0..leaf.prefix.len() {
        items.push(element(index)?);
    }
    items.extend(demanded);
    // One candidate per index promises no distinctness; the caller's check turns that down.
    for index in items.len()..length {
        items.push(element(index)?);
    }
    Some(Value::Array(items))
}

/// An element valid under `facet` that `position`, the schema of the index it sits at, accepts
/// too. Where none is found, the demand's first value, which the caller's check turns down.
fn demanded_element<'a>(
    facet: &'a ContainsFacet,
    position: Option<&'a Schema>,
    walk: &Walk<'a, '_>,
) -> Option<Value> {
    let Some(position) = position else {
        return walk.first(&facet.schema);
    };
    let ctx = walk.ctx();
    let accepted = |schema: &Schema, value: &Value| {
        holds(ctx, || {
            algebra::accepts_value(schema, value, UncheckableFacet::Undecided, ctx)
                == Verdict::Accepts
        })
    };
    // A candidate of the demand need not be valid under it, so both schemas are asked; one of a type the
    // position never holds is turned down unasked. The first one is kept for the fallback below.
    let mut first = None;
    let mut found = None;
    let mut attempts = DEMAND_ATTEMPTS;
    let _ = walk.each(&facet.schema, &mut |value| {
        if !bars_type(position, &value, walk)
            && accepted(position, &value)
            && accepted(&facet.schema, &value)
        {
            found = Some(value);
            return ControlFlow::Break(());
        }
        first.get_or_insert(value);
        attempts -= 1;
        if attempts == 0 {
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    });
    if found.is_some() {
        return found;
    }
    // One branch of a union demand intersected with the position is enough to offer a value, and
    // costs far less than the whole union, whose branches are normalized against each other once
    // intersected.
    if let SchemaKind::AnyOf(branches) = facet.schema.kind() {
        for branch in branches.as_slice().iter().take(DEMAND_BRANCHES) {
            let (both, inexact) =
                ctx.probe(|| algebra::intersect(branch.clone(), position.clone(), ctx));
            if inexact {
                continue;
            }
            let _ = walk.each_owned(
                &both,
                &mut keep_accepted(&mut found, DEMAND_ATTEMPTS, |value| accepted(&both, value)),
            );
            if found.is_some() {
                return found;
            }
        }
    }
    // A value of both at once, where the form expresses their intersection exactly.
    let (both, inexact) =
        ctx.probe(|| algebra::intersect(facet.schema.clone(), position.clone(), ctx));
    if !inexact {
        let _ = walk.each_owned(
            &both,
            &mut keep_accepted(&mut found, DEMAND_ATTEMPTS, |value| accepted(&both, value)),
        );
        if found.is_some() {
            return found;
        }
    }
    first
}

/// Pointers followed before `bars_type` gives up on a chain.
const TYPE_HOPS: usize = 8;

/// Whether `schema` holds no value of `value`'s type, as its kind alone tells: `accepts_value`
/// would turn the value down. `false` wherever the kind does not tell.
fn bars_type<'a>(schema: &'a Schema, value: &Value, walk: &Walk<'a, '_>) -> bool {
    // Integers and other numbers are one type here: which numbers a schema holds is the check's call.
    let ty = match value {
        Value::Null => JsonType::Null,
        Value::Bool(_) => JsonType::Boolean,
        Value::Number(_) => JsonType::Number,
        Value::String(_) => JsonType::String,
        Value::Array(_) => JsonType::Array,
        Value::Object(_) => JsonType::Object,
    };
    let mut schema = schema;
    for _ in 0..TYPE_HOPS {
        let SchemaKind::Reference(uri) = schema.kind() else {
            return kind_bars_type(schema.kind(), ty);
        };
        let Some(named) = (walk.target)(uri) else {
            return false;
        };
        schema = named;
    }
    false
}

/// `bars_type` for a kind other than a pointer.
fn kind_bars_type(kind: &SchemaKind, ty: JsonType) -> bool {
    match kind {
        SchemaKind::Integer(_) | SchemaKind::Number(_) => ty != JsonType::Number,
        SchemaKind::String(_) => ty != JsonType::String,
        SchemaKind::Array(_) => ty != JsonType::Array,
        SchemaKind::Object(_) => ty != JsonType::Object,
        SchemaKind::MultiType(set) => {
            let holds =
                set.contains(ty) || (ty == JsonType::Number && set.contains(JsonType::Integer));
            !holds
        }
        SchemaKind::False => true,
        SchemaKind::True
        | SchemaKind::Const(_)
        | SchemaKind::Enum(_)
        | SchemaKind::TypedGroup { .. }
        | SchemaKind::AllOf(_)
        | SchemaKind::AnyOf(_)
        | SchemaKind::OneOf(_)
        | SchemaKind::Not(_)
        | SchemaKind::Reference(_)
        | SchemaKind::Raw(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, collections::BTreeSet, sync::Arc};

    use referencing::Draft;
    use serde_json::{json, Value};
    use test_case::test_case;

    use super::*;
    use crate::{
        canonical::{context::CanonicalizationContext, DefinitionMap},
        options::PatternEngineOptions,
    };

    #[test_case(&json!({"type": "integer", "minimum": 0}), &json!("a"), true ; "integer leaf bars a string")]
    #[test_case(&json!({"type": "integer", "minimum": 0}), &json!(1.5), false ; "integer leaf leaves numbers to the check")]
    #[test_case(&json!({"type": "number", "minimum": 0}), &json!(null), true ; "number leaf bars null")]
    #[test_case(&json!({"type": "string", "minLength": 1}), &json!(1), true ; "string leaf bars a number")]
    #[test_case(&json!({"type": "string", "minLength": 1}), &json!("a"), false ; "string leaf holds strings")]
    #[test_case(&json!({"type": "array", "minItems": 1}), &json!({}), true ; "array leaf bars an object")]
    #[test_case(&json!({"type": "object", "minProperties": 1}), &json!([]), true ; "object leaf bars an array")]
    #[test_case(&json!({"type": "integer"}), &json!(1.5), false ; "integer type set leaves numbers to the check")]
    #[test_case(&json!({"type": ["null", "string"]}), &json!(null), false ; "type set holding the type")]
    #[test_case(&json!({"type": ["null", "string"]}), &json!(false), true ; "type set without the type")]
    #[test_case(&json!(false), &json!(null), true ; "false bars everything")]
    #[test_case(&json!({"anyOf": [{"type": "integer", "minimum": 0}, {"type": "string", "minLength": 1}]}), &json!(null), false ; "composite form does not tell")]
    #[test_case(&json!({"$ref": "#/$defs/n", "$defs": {"n": {"type": "integer", "minimum": 0}}}), &json!("a"), true ; "pointer read through")]
    fn type_bars_only_values_the_check_rejects(schema: &Value, value: &Value, expected: bool) {
        let canonical = crate::canonical::options()
            .with_draft(Draft::Draft202012)
            .canonicalize(schema)
            .expect("valid schema");
        let node = Schema::new(canonical.schema_kind().clone());
        let definitions: DefinitionMap = canonical
            .definitions()
            .map(|(uri, named)| (Arc::from(uri), Schema::new(named.schema_kind().clone())))
            .collect();
        let ctx = CanonicalizationContext::new(
            Draft::Draft202012,
            PatternEngineOptions::default(),
            false,
        )
        .resolving(Arc::new(definitions), BTreeSet::new());
        let budget = Cell::new(super::super::NODES);
        let walk = Walk {
            target: &|uri| ctx.definition(uri),
            depth: super::super::DEPTH,
            budget: &budget,
            ctx: &ctx,
        };
        assert_eq!(bars_type(&node, value, &walk), expected);
        if expected {
            assert_eq!(
                algebra::accepts_value(&node, value, UncheckableFacet::Undecided, &ctx),
                Verdict::Rejects
            );
        }
    }
}
