//! Structural complement of a canonical node.
use crate::{
    canonical::ir::{type_set_schema, CanonicalJson, Schema, SchemaKind},
    JsonType, JsonTypeSet,
};

/// The complement schema, or `None` when the IR cannot spell it and the caller keeps the document
/// `Raw`. Negation has no safe default direction, so every arm is exact or declines.
pub(crate) fn negate(schema: &Schema) -> Option<Schema> {
    match schema.kind() {
        SchemaKind::True => Some(Schema::new(SchemaKind::False)),
        SchemaKind::False => Some(Schema::new(SchemaKind::True)),
        SchemaKind::MultiType(set) => negate_type_set(*set),
        SchemaKind::Const(value) => negate_finite_values(std::slice::from_ref(value)),
        SchemaKind::Enum(values) => negate_finite_values(values.as_slice()),
        SchemaKind::TypedGroup { .. }
        | SchemaKind::String(_)
        | SchemaKind::Integer(_)
        | SchemaKind::Number(_)
        | SchemaKind::Array(_)
        | SchemaKind::Object(_)
        | SchemaKind::AnyOf(_)
        | SchemaKind::Raw(_) => None,
    }
}

/// Complement of a finite value set, expressible only when the values saturate whole types.
/// ```text
/// e.g.  {"not": {"const": null}}  =>  {"type": ["boolean", "number", "string", "array", "object"]}
/// e.g.  {"not": {"enum": [null, true]}}  =>  unchanged
/// ```
fn negate_finite_values(values: &[CanonicalJson]) -> Option<Schema> {
    negate_type_set(SchemaKind::finite_values_saturated_domain(values)?)
}

/// Complement of a type set over the value space. `None` when the set admits `integer` but not
/// `number`: the complement then admits non-integer numbers, which no type set can name.
/// ```text
/// e.g.  {"not": {"type": "string"}}  =>  {"type": ["null", "boolean", "number", "array", "object"]}
/// ```
fn negate_type_set(set: JsonTypeSet) -> Option<Schema> {
    if set.contains(JsonType::Integer) && !set.contains(JsonType::Number) {
        return None;
    }
    let mut complement = JsonTypeSet::empty();
    for ty in [
        JsonType::Null,
        JsonType::Boolean,
        JsonType::String,
        JsonType::Array,
        JsonType::Object,
    ] {
        if !set.contains(ty) {
            complement = complement.insert(ty);
        }
    }
    // A set carrying `number` admits every number, so its complement admits none; a set carrying
    // neither numeric type admits no number, so its complement admits all of them.
    if !set.contains(JsonType::Number) {
        complement = complement.insert(JsonType::Number);
    }
    if complement.is_empty() {
        return Some(Schema::new(SchemaKind::False));
    }
    // The shared constructor, so a complement spelling a lone `null` or `boolean` lands on the same
    // canonical node as the direct spelling.
    Some(type_set_schema(complement))
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::*;

    const TYPES: [JsonType; 7] = [
        JsonType::Null,
        JsonType::Boolean,
        JsonType::Integer,
        JsonType::Number,
        JsonType::String,
        JsonType::Array,
        JsonType::Object,
    ];

    // One value per equivalence class of the type vocabulary; `1` and `1.5` are distinct classes
    // because an integer satisfies both `integer` and `number` while a fraction satisfies only
    // `number`.
    fn representatives() -> [Value; 7] {
        [
            json!(null),
            json!(true),
            json!(1),
            json!(1.5),
            json!("x"),
            json!([]),
            json!({}),
        ]
    }

    fn admits(set: JsonTypeSet, value: &Value) -> bool {
        match value {
            Value::Null => set.contains(JsonType::Null),
            Value::Bool(_) => set.contains(JsonType::Boolean),
            Value::Number(number) if number.is_i64() => {
                set.contains(JsonType::Integer) || set.contains(JsonType::Number)
            }
            Value::Number(_) => set.contains(JsonType::Number),
            Value::String(_) => set.contains(JsonType::String),
            Value::Array(_) => set.contains(JsonType::Array),
            Value::Object(_) => set.contains(JsonType::Object),
        }
    }

    // Membership for the canonical shapes a complement can take: a type set, its boolean-schema
    // collapses, and the value-set spellings of a lone `null` or `boolean` type.
    #[allow(clippy::wildcard_enum_match_arm)]
    fn complement_admits(schema: &Schema, value: &Value) -> bool {
        match schema.kind() {
            SchemaKind::True => true,
            SchemaKind::False => false,
            SchemaKind::MultiType(set) => admits(*set, value),
            SchemaKind::Const(constant) => {
                assert_eq!(constant.as_value(), &Value::Null);
                value.is_null()
            }
            SchemaKind::Enum(values) => {
                let members: Vec<&Value> = values
                    .as_slice()
                    .iter()
                    .map(CanonicalJson::as_value)
                    .collect();
                assert_eq!(members, [&Value::Bool(false), &Value::Bool(true)]);
                value.is_boolean()
            }
            other => {
                panic!("scaffold complement of a type set is a type-set shape, got {other:?}")
            }
        }
    }

    // The scaffold's domain is finite, so the complement-membership law is proven exhaustively: for
    // every one of the 128 type sets, either negate declines (integer without number) or its result
    // admits a value exactly when the original does not.
    #[test]
    fn type_set_complement_partitions_the_value_space() {
        for mask in 0u8..128 {
            let mut set = JsonTypeSet::empty();
            for ty in TYPES {
                if mask & ty as u8 != 0 {
                    set = set.insert(ty);
                }
            }
            let schema = Schema::new(SchemaKind::MultiType(set));
            let complement = negate(&schema);
            if set.contains(JsonType::Integer) && !set.contains(JsonType::Number) {
                assert!(
                    complement.is_none(),
                    "integer-only set {set:?} must decline"
                );
                continue;
            }
            let complement = complement.expect("expressible complement");
            for value in &representatives() {
                assert_ne!(
                    admits(set, value),
                    complement_admits(&complement, value),
                    "membership not partitioned for set {set:?} on {value}"
                );
            }
        }
    }

    #[test]
    fn boolean_schemas_negate_to_each_other() {
        assert!(matches!(
            negate(&Schema::new(SchemaKind::True)).map(|s| s.kind().clone()),
            Some(SchemaKind::False)
        ));
        assert!(matches!(
            negate(&Schema::new(SchemaKind::False)).map(|s| s.kind().clone()),
            Some(SchemaKind::True)
        ));
    }
}
