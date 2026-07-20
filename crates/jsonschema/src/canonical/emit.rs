//! IR -> JSON Schema emit.

use referencing::Draft;
use serde_json::{json, Map, Value};

use crate::{
    canonical::ir::{CanonicalJson, Schema, SchemaKind},
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
        SchemaKind::Enum(values) => emit_enum(values),
        SchemaKind::MultiType(set) => emit_multi_type(*set),
        // The body emits a `const`/`enum` object without a `type` key, so adding `type` beside it
        // expresses "both must hold" and re-parses to the same IR.
        SchemaKind::TypedGroup { ty, body } => {
            let mut map = match emit(body.kind(), draft) {
                Value::Object(map) => map,
                other => unreachable!("value-set body emits an object: {other:?}"),
            };
            map.insert("type".into(), Value::String(ty.to_string()));
            Value::Object(map)
        }
        SchemaKind::Raw(value) => value.get().clone(),
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
        other => unreachable!("emit yields only objects or booleans: {other:?}"),
    };
    map.insert("$schema".into(), Value::String(uri.into()));
    Value::Object(map)
}
