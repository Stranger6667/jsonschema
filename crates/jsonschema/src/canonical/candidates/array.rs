//! Candidates for an array leaf.
use serde_json::Value;

use super::{demanded_length, Walk};
use crate::canonical::ir::ArrayLeaf;

/// Instances worth trying against an array leaf: one array as long as the leaf demands.
pub(super) fn array<'a>(leaf: &'a ArrayLeaf, walk: &Walk<'a, '_>) -> Vec<Value> {
    let Some(length) = demanded_length(leaf.lengths.minimum.as_ref()) else {
        return Vec::new();
    };
    // A `contains` demand needs an element meeting it, which the floor alone never asks
    // for; each demand contributes one, and the check below turns down what does not hold.
    let demanded: Option<Vec<Value>> = leaf
        .contains
        .iter()
        .map(|facet| walk.first(&facet.schema))
        .collect();
    let Some(demanded) = demanded else {
        return Vec::new();
    };
    let element = |index: usize| {
        let schema = leaf.prefix.get(index).or(leaf.items.as_ref());
        schema.map_or(Some(Value::Null), |schema| walk.first(schema))
    };
    // A prefix schema governs the index it names, so demanded elements sit past the whole
    // prefix rather than at the head.
    let mut items = Vec::new();
    for index in 0..leaf.prefix.len() {
        let Some(value) = element(index) else {
            return Vec::new();
        };
        items.push(value);
    }
    items.extend(demanded);
    // One candidate per index promises no distinctness; the check below turns that down.
    for index in items.len()..length {
        let Some(value) = element(index) else {
            return Vec::new();
        };
        items.push(value);
    }
    vec![Value::Array(items)]
}
