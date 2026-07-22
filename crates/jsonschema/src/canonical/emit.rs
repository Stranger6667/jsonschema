//! IR -> JSON Schema emit.

use referencing::Draft;
use serde_json::{json, Map, Value};

use crate::{
    canonical::ir::{
        ArrayLeaf, CanonicalJson, Divisors, IntegerLeaf, NumberLeaf, ObjectLeaf, Schema,
        SchemaKind, StringLeaf,
    },
    JsonTypeSet,
};

pub(crate) fn to_json_schema(root: &Schema, draft: Draft) -> Value {
    let value = emit(root.kind(), draft);
    if matches!(root.kind(), SchemaKind::Raw(_)) {
        return value;
    }
    match schema_uri(draft) {
        Some(uri) => with_schema_uri(value, uri),
        None => value,
    }
}

fn emit(kind: &SchemaKind, draft: Draft) -> Value {
    match kind {
        SchemaKind::True if matches!(draft, Draft::Draft4) => Value::Object(Map::new()),
        SchemaKind::True => Value::Bool(true),
        SchemaKind::False if matches!(draft, Draft::Draft4) => json!({"not": {}}),
        SchemaKind::False => Value::Bool(false),
        // `{"const": null}` is identical to `{"type": "null"}` - prefer the type form.
        SchemaKind::Const(value) if value.as_value().is_null() => json!({"type": "null"}),
        SchemaKind::Const(value) if matches!(draft, Draft::Draft4) => {
            json!({"enum": [value.to_value()]})
        }
        SchemaKind::Const(value) => json!({"const": value.to_value()}),
        SchemaKind::Enum(values) => emit_enum(values.as_slice()),
        SchemaKind::String(leaf) => emit_string(leaf.get()),
        SchemaKind::Integer(leaf) => emit_integer(leaf.get()),
        SchemaKind::Number(leaf) => emit_number(leaf.get(), draft),
        SchemaKind::Array(leaf) => emit_array(leaf.get()),
        SchemaKind::Object(leaf) => emit_object(leaf.get()),
        SchemaKind::MultiType(set) => emit_multi_type(*set),
        // The body emits a `const`/`enum` object without a `type` key, so adding `type` beside it
        // expresses "both must hold" and re-parses to the same IR.
        SchemaKind::TypedGroup { ty, body } => {
            let mut map = match emit(body.kind(), draft) {
                Value::Object(map) => map,
                other @ (Value::Null
                | Value::Bool(_)
                | Value::Number(_)
                | Value::String(_)
                | Value::Array(_)) => unreachable!("value-set body emits an object: {other:?}"),
            };
            map.insert("type".into(), Value::String(ty.to_string()));
            Value::Object(map)
        }
        SchemaKind::AnyOf(branches) => json!({
            "anyOf": branches
                .as_slice()
                .iter()
                .map(|branch| emit(branch.kind(), draft))
                .collect::<Vec<_>>()
        }),
        SchemaKind::Raw(value) => value.get().clone(),
    }
}

/// Emit a string leaf as `{"type":"string"}` plus its length bounds and patterns. A single pattern is
/// inline; several become an `allOf` of `{"pattern": ...}`, since one leaf can hold only one `pattern`.
fn emit_string(leaf: &StringLeaf) -> Value {
    let mut map = Map::new();
    map.insert("type".into(), Value::String("string".into()));
    if let Some(min) = &leaf.lengths.minimum {
        map.insert("minLength".into(), Value::Number(min.to_number()));
    }
    if let Some(max) = &leaf.lengths.maximum {
        map.insert("maxLength".into(), Value::Number(max.to_number()));
    }
    let mut conjuncts: Vec<Value> = Vec::new();
    match leaf.patterns.as_slice() {
        [] => {}
        [pattern] => {
            map.insert("pattern".into(), Value::String(pattern.to_string()));
        }
        patterns => conjuncts.extend(
            patterns
                .iter()
                .map(|pattern| json!({ "pattern": pattern.as_ref() })),
        ),
    }
    match leaf.formats.as_slice() {
        [] => {}
        [format] => {
            map.insert("format".into(), Value::String(format.to_string()));
        }
        formats => conjuncts.extend(
            formats
                .iter()
                .map(|format| json!({ "format": format.as_ref() })),
        ),
    }
    if !conjuncts.is_empty() {
        map.insert("allOf".into(), Value::Array(conjuncts));
    }
    Value::Object(map)
}

/// Emit a number leaf as `{"type":"number"}` plus its interval bounds, using the exclusive spelling
/// for an endpoint the interval does not admit.
fn emit_number(leaf: &NumberLeaf, draft: Draft) -> Value {
    let mut map = Map::new();
    map.insert("type".into(), Value::String("number".into()));
    // Draft 4 spells exclusivity as a boolean flag beside the bound; later drafts give it its own
    // numeric keyword.
    let draft4 = matches!(draft, Draft::Draft4);
    for (bound, inclusive_key, exclusive_key) in [
        (leaf.minimum.as_ref(), "minimum", "exclusiveMinimum"),
        (leaf.maximum.as_ref(), "maximum", "exclusiveMaximum"),
    ] {
        let Some(bound) = bound else {
            continue;
        };
        let limit = Value::Number(bound.to_number());
        if bound.is_inclusive() {
            map.insert(inclusive_key.into(), limit);
        } else if draft4 {
            map.insert(inclusive_key.into(), limit);
            map.insert(exclusive_key.into(), Value::Bool(true));
        } else {
            map.insert(exclusive_key.into(), limit);
        }
    }
    emit_divisors(&mut map, &leaf.multiple_of);
    Value::Object(map)
}

/// Emit an array leaf as `{"type":"array"}` plus its length bounds and uniqueness.
fn emit_array(leaf: &ArrayLeaf) -> Value {
    let mut map = Map::new();
    map.insert("type".into(), Value::String("array".into()));
    if leaf.unique {
        map.insert("uniqueItems".into(), Value::Bool(true));
    }
    if let Some(min) = &leaf.lengths.minimum {
        map.insert("minItems".into(), Value::Number(min.to_number()));
    }
    if let Some(max) = &leaf.lengths.maximum {
        map.insert("maxItems".into(), Value::Number(max.to_number()));
    }
    Value::Object(map)
}

/// Emit an object leaf as `{"type":"object"}` plus its required keys and property-count bounds.
fn emit_object(leaf: &ObjectLeaf) -> Value {
    let mut map = Map::new();
    map.insert("type".into(), Value::String("object".into()));
    if !leaf.required.is_empty() {
        map.insert(
            "required".into(),
            Value::Array(
                leaf.required
                    .iter()
                    .map(|key| Value::String(key.to_string()))
                    .collect(),
            ),
        );
    }
    if let Some(min) = &leaf.sizes.minimum {
        map.insert("minProperties".into(), Value::Number(min.to_number()));
    }
    if let Some(max) = &leaf.sizes.maximum {
        map.insert("maxProperties".into(), Value::Number(max.to_number()));
    }
    Value::Object(map)
}

/// Emit an integer leaf as `{"type":"integer"}` plus its interval bounds.
fn emit_integer(leaf: &IntegerLeaf) -> Value {
    let mut map = Map::new();
    map.insert("type".into(), Value::String("integer".into()));
    if let Some(min) = &leaf.bounds.minimum {
        map.insert("minimum".into(), Value::Number(min.to_number()));
    }
    if let Some(max) = &leaf.bounds.maximum {
        map.insert("maximum".into(), Value::Number(max.to_number()));
    }
    emit_divisors(&mut map, &leaf.multiple_of);
    Value::Object(map)
}

/// A lone divisor sits beside the other facets; several are spelled as an `allOf`, since one
/// `multipleOf` cannot carry them.
fn emit_divisors(map: &mut Map<String, Value>, divisors: &Divisors) {
    match divisors.as_slice() {
        [] => {}
        [step] => {
            map.insert("multipleOf".into(), Value::Number(step.to_number()));
        }
        steps => {
            let conjuncts = steps
                .iter()
                .map(|step| {
                    let mut object = Map::new();
                    object.insert("multipleOf".into(), Value::Number(step.to_number()));
                    Value::Object(object)
                })
                .collect();
            map.insert("allOf".into(), Value::Array(conjuncts));
        }
    }
}

/// Emit a standalone `Enum`; collapse to `type:[...]` when the value set saturates one or more JSON types.
fn emit_enum(values: &[CanonicalJson]) -> Value {
    if let Some(set) = SchemaKind::finite_values_saturated_domain(values) {
        return emit_multi_type(set);
    }
    json!({
        "enum": values.iter().map(CanonicalJson::to_value).collect::<Vec<_>>()
    })
}

/// Emit a type set as `{"type": "x"}` for a singleton or `{"type": [...]}` otherwise.
fn emit_multi_type(set: JsonTypeSet) -> Value {
    // `set.iter()` yields in canonical order (null, boolean, integer, ...).
    let mut names = set.iter().map(|ty| ty.to_string());
    match (names.next(), names.next()) {
        (Some(only), None) => json!({"type": only}),
        (first, second) => {
            let names: Vec<Value> = first
                .into_iter()
                .chain(second)
                .chain(names)
                .map(Value::String)
                .collect();
            json!({"type": names})
        }
    }
}

pub(crate) fn schema_uri(draft: Draft) -> Option<&'static str> {
    match draft {
        Draft::Draft4 => Some("http://json-schema.org/draft-04/schema#"),
        Draft::Draft6 => Some("http://json-schema.org/draft-06/schema#"),
        Draft::Draft7 => Some("http://json-schema.org/draft-07/schema#"),
        Draft::Draft201909 => Some("https://json-schema.org/draft/2019-09/schema"),
        Draft::Draft202012 => Some("https://json-schema.org/draft/2020-12/schema"),
        // `Draft::Unknown` (unrecognised `$schema`) has no canonical meta-schema; omit `$schema`.
        _ => None,
    }
}

/// Insert `$schema` into the document, first rewriting a boolean form into its object shape.
fn with_schema_uri(value: Value, uri: &'static str) -> Value {
    let mut map = match value {
        Value::Object(map) => map,
        Value::Bool(true) => Map::new(),
        Value::Bool(false) => {
            let mut map = Map::new();
            map.insert("not".into(), Value::Object(Map::new()));
            map
        }
        other @ (Value::Null | Value::Number(_) | Value::String(_) | Value::Array(_)) => {
            unreachable!("emit yields only objects or booleans: {other:?}")
        }
    };
    map.insert("$schema".into(), Value::String(uri.into()));
    Value::Object(map)
}
