//! Instances worth trying against a node, for the questions a form alone cannot answer.
use std::cell::Cell;

use serde_json::{Number, Value};

use crate::{
    canonical::{
        algebra,
        context::CanonicalizationContext,
        ir::{
            BoundCardinality, BoundInteger, BoundNumber, LengthBounds, ObjectLeaf, Schema,
            SchemaKind, StringFormat, StringLeaf, UncheckableFacet, Verdict,
        },
    },
    JsonType,
};

/// How far a candidate walks into a node before giving up.
pub(crate) const CANDIDATE_DEPTH: u32 = 6;

/// The longest instance worth building. A bound past this is a bound no instance is built for: the
/// answer is left undecided rather than spending the memory a schema keyword asked for.
const CANDIDATE_LENGTH: u64 = 64;

/// The key a filler entry is given, where an object needs more of them than it requires.
const FILLER_KEY: &str = "a";

/// How many instances one walk may build. Depth and width bound a candidate on their own, but a
/// node wide at every level multiplies them - 64 keys six levels down is a number of values no
/// machine holds - so the walk also spends from one count and gives up where it runs out.
pub(crate) const CANDIDATE_NODES: u32 = 4_096;

/// Names tried per seed before the next one. A leaf wanting more is answered by later seeds, or
/// left undecided.
const CANDIDATE_KEYS_PER_SEED: usize = 8;

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
pub(crate) fn candidate_instances<'a>(
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
    match node.kind() {
        // A pointer accepts what its target does, so the instances worth trying are the target's.
        SchemaKind::Reference(uri) => target(uri)
            .map(|named| candidate_instances(named, target, depth - 1, budget, ctx))
            .unwrap_or_default(),
        SchemaKind::True => vec![Value::Null],
        SchemaKind::Const(value) => vec![value.as_value().clone()],
        SchemaKind::Enum(values) => values
            .as_slice()
            .iter()
            .map(|value| value.as_value().clone())
            .collect(),
        SchemaKind::MultiType(set) => set.iter().map(shortest_instance).collect(),
        SchemaKind::TypedGroup { body, .. } => {
            candidate_instances(body, target, depth - 1, budget, ctx)
        }
        SchemaKind::String(leaf) => string_candidates(leaf.get()),
        SchemaKind::Integer(leaf) => {
            // An integer bound is stored as the first integer it admits.
            let bounds = &leaf.get().bounds;
            let end = |bound: &BoundInteger| WindowEnd {
                limit: bound.to_number(),
                admitted: true,
            };
            whole_number_candidates(
                bounds.minimum.as_ref().map(end),
                bounds.maximum.as_ref().map(end),
            )
        }
        SchemaKind::Number(leaf) => {
            let leaf = leaf.get();
            let end = |bound: &BoundNumber| WindowEnd {
                limit: bound.to_number(),
                admitted: bound.is_inclusive(),
            };
            whole_number_candidates(
                leaf.minimum.as_ref().map(end),
                leaf.maximum.as_ref().map(end),
            )
        }
        SchemaKind::Array(leaf) => {
            let leaf = leaf.get();
            let Some(length) = demanded_length(leaf.lengths.minimum.as_ref()) else {
                return Vec::new();
            };
            // A `contains` demand needs an element meeting it, which the floor alone never asks
            // for; each demand contributes one, and the check below turns down what does not hold.
            let demanded: Option<Vec<Value>> = leaf
                .contains
                .iter()
                .map(|facet| {
                    candidate_instances(&facet.schema, target, depth - 1, budget, ctx)
                        .into_iter()
                        .next()
                })
                .collect();
            let Some(demanded) = demanded else {
                return Vec::new();
            };
            let element = |index: usize| {
                let schema = leaf.prefix.get(index).or(leaf.items.as_ref());
                schema.map_or(Some(Value::Null), |schema| {
                    candidate_instances(schema, target, depth - 1, budget, ctx)
                        .into_iter()
                        .next()
                })
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
        SchemaKind::Object(leaf) => {
            let leaf = leaf.get();
            let Some(size) = demanded_length(leaf.sizes.minimum.as_ref()) else {
                return Vec::new();
            };
            let mut object = serde_json::Map::new();
            for key in candidate_keys(leaf, size, ctx) {
                // A key answers to the entry declaring it, then a pattern entry it matches, then
                // the shield. Where several govern one key, the check below turns down a value that
                // satisfies only the one taken here.
                let governing = leaf
                    .properties
                    .get(key.as_str())
                    .or_else(|| {
                        leaf.pattern_properties
                            .iter()
                            .find(|(pattern, _)| algebra::matches_key(pattern, &key, ctx))
                            .map(|(_, schema)| schema)
                    })
                    .or(leaf.additional.as_ref());
                let Some(value) = governing.map_or(Some(Value::Null), |schema| {
                    candidate_instances(schema, target, depth - 1, budget, ctx)
                        .into_iter()
                        .next()
                }) else {
                    return Vec::new();
                };
                object.insert(key, value);
            }
            vec![Value::Object(object)]
        }
        // A branch's values are worth trying against the whole: `admits_value` decides.
        SchemaKind::AllOf(branches) | SchemaKind::AnyOf(branches) => branches
            .as_slice()
            .iter()
            .flat_map(|branch| candidate_instances(branch, target, depth - 1, budget, ctx))
            .collect(),
        SchemaKind::OneOf(branches) => branches
            .iter()
            .flat_map(|branch| candidate_instances(branch, target, depth - 1, budget, ctx))
            .collect(),
        SchemaKind::False | SchemaKind::Not(_) | SchemaKind::Raw(_) => Vec::new(),
    }
}

/// Keys to build a candidate object out of: the ones the leaf demands, then names its constraints
/// admit, up to the size floor.
///
/// A key constraint can turn down every name of the plain filler sequence, so each pattern seeds
/// names of its own. A name already demanded is never offered twice.
fn candidate_keys(leaf: &ObjectLeaf, size: usize, ctx: &CanonicalizationContext) -> Vec<String> {
    let mut keys: Vec<String> = leaf.required.iter().map(ToString::to_string).collect();
    let wanted = size.max(keys.len());
    if keys.len() >= wanted {
        return keys;
    }
    let admitted = |key: &str| {
        leaf.property_names.as_ref().is_none_or(|names| {
            algebra::admits_value(
                names,
                &Value::String(key.to_string()),
                UncheckableFacet::Undecided,
                ctx,
            ) == Verdict::Admits
        })
    };
    let seeds = leaf
        .properties
        .keys()
        .map(ToString::to_string)
        .chain(
            leaf.pattern_properties
                .keys()
                .map(|pattern| literal_prefix(pattern)),
        )
        .chain(std::iter::once(FILLER_KEY.to_string()));
    for seed in seeds {
        for index in 0..CANDIDATE_KEYS_PER_SEED {
            if keys.len() >= wanted {
                return keys;
            }
            let key = format!("{seed}{index}");
            if !keys.contains(&key) && admitted(&key) {
                keys.push(key);
            }
        }
    }
    keys
}

/// Strings to try against a string leaf: one per pattern, one per format, and their concatenation,
/// which is the only one that can match a pattern anchored at the start and one anchored at the end.
fn string_candidates(leaf: &StringLeaf) -> Vec<Value> {
    let mut proposals: Vec<String> = Vec::new();
    let per_pattern: Vec<String> = leaf
        .patterns
        .iter()
        .filter_map(|pattern| jsonschema_regex::pattern_witness(pattern))
        .collect();
    // Skip the concatenation when a pattern produced no string of its own: it would not be
    // checked against that pattern.
    if per_pattern.len() > 1 && per_pattern.len() == leaf.patterns.len() {
        proposals.push(per_pattern.concat());
    }
    proposals.extend(per_pattern);
    proposals.extend(
        leaf.formats
            .iter()
            .filter_map(StringFormat::witness)
            .map(ToOwned::to_owned),
    );
    // The shortest string the window allows, for a leaf constrained only by length.
    if let Some(length) = demanded_length(leaf.lengths.minimum.as_ref()) {
        proposals.push("a".repeat(length));
    }
    proposals
        .into_iter()
        .filter_map(|text| fit_to_window(text, &leaf.lengths))
        .map(Value::String)
        .collect()
}

/// A string moved into the length window: padded when too short, dropped when too long or when the
/// window asks for more characters than a candidate is built with.
fn fit_to_window(mut text: String, window: &LengthBounds) -> Option<String> {
    let length = text.chars().count();
    if window
        .maximum
        .as_ref()
        .and_then(BoundCardinality::to_usize)
        .is_some_and(|maximum| length > maximum)
    {
        return None;
    }
    let minimum = window
        .minimum
        .as_ref()
        .and_then(BoundCardinality::to_usize)
        .unwrap_or(0);
    if minimum > length {
        if minimum as u64 > CANDIDATE_LENGTH {
            return None;
        }
        text.push_str(&"a".repeat(minimum - length));
    }
    Some(text)
}

/// The literal head of a key pattern, which every key an anchored one matches carries. Empty where
/// the pattern spells nothing a name can be built from.
fn literal_prefix(pattern: &str) -> String {
    pattern
        .strip_prefix('^')
        .unwrap_or(pattern)
        .chars()
        .take_while(|character| character.is_alphanumeric() || *character == '_')
        .collect()
}

/// Zero, which every divisor takes, and the ends of the window.
fn whole_number_candidates(minimum: Option<WindowEnd>, maximum: Option<WindowEnd>) -> Vec<Value> {
    // A point between the two ends, or one step in from the only end there is.
    let interior = interior_point(
        minimum.as_ref().map(|end| &end.limit),
        maximum.as_ref().map(|end| &end.limit),
    );
    // Admitted ends first: a caller taking one candidate needs one the window holds, and an
    // excluded end is no value of it.
    let mut candidates: Vec<Value> = [minimum, maximum]
        .into_iter()
        .flatten()
        .filter(|end| end.admitted)
        .map(|end| Value::Number(end.limit))
        .collect();
    candidates.extend(interior);
    // Zero, which every divisor takes.
    candidates.push(Value::Number(0.into()));
    candidates
}

/// One end of a numeric window, and whether the window admits it.
pub(crate) struct WindowEnd {
    limit: Number,
    admitted: bool,
}

/// A number strictly inside the window, where one can be spelled. A window narrower than the gap
/// between two neighbouring floats has none.
fn interior_point(minimum: Option<&Number>, maximum: Option<&Number>) -> Option<Value> {
    let point = match (
        minimum.and_then(Number::as_f64),
        maximum.and_then(Number::as_f64),
    ) {
        (Some(low), Some(high)) => low + (high - low) / 2.0,
        (Some(low), None) => low + 1.0,
        (None, Some(high)) => high - 1.0,
        (None, None) => return None,
    };
    Number::from_f64(point).map(Value::Number)
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
