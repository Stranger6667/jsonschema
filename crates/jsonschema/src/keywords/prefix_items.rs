use crate::{
    compiler,
    error::{no_error, ErrorIterator, ValidationError},
    evaluation::Annotations,
    node::SchemaNode,
    paths::{LazyLocation, Location, RefTracker},
    types::JsonType,
    validator::{EvaluationResult, Validate, ValidationContext},
};
use serde_json::{Map, Value};

use super::CompilationResult;

pub(crate) struct PrefixItemsValidator {
    schemas: Vec<SchemaNode>,
}

impl PrefixItemsValidator {
    #[inline]
    pub(crate) fn compile<'a>(
        ctx: &compiler::Context,
        items: &'a [Value],
    ) -> CompilationResult<'a> {
        let ctx = ctx.new_at_location("prefixItems");
        let mut schemas = Vec::with_capacity(items.len());
        for (idx, item) in items.iter().enumerate() {
            let ctx = ctx.new_at_location(idx);
            let validators = compiler::compile(&ctx, ctx.as_resource_ref(item))?;
            schemas.push(validators);
        }
        Ok(Box::new(PrefixItemsValidator { schemas }))
    }
}

impl Validate for PrefixItemsValidator {
    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        if let Value::Array(items) = instance {
            for (schema, item) in self.schemas.iter().zip(items.iter()) {
                if !schema.is_valid(item, ctx) {
                    return false;
                }
            }
            true
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Array(items) = instance {
            for (idx, (schema, item)) in self.schemas.iter().zip(items.iter()).enumerate() {
                schema.validate(item, &location.push(idx), tracker, ctx)?;
            }
        }
        Ok(())
    }

    fn iter_errors<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Value::Array(items) = instance {
            let mut errors = Vec::new();
            for (idx, (schema, item)) in self.schemas.iter().zip(items.iter()).enumerate() {
                errors.extend(schema.iter_errors(item, &location.push(idx), tracker, ctx));
            }
            ErrorIterator::from_iterator(errors.into_iter())
        } else {
            no_error()
        }
    }

    fn evaluate(
        &self,
        instance: &Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Value::Array(items) = instance {
            if !items.is_empty() {
                let mut children = Vec::with_capacity(self.schemas.len().min(items.len()));
                let mut max_index_applied = 0usize;
                for (idx, (schema_node, item)) in self.schemas.iter().zip(items.iter()).enumerate()
                {
                    children.push(schema_node.evaluate_instance(
                        item,
                        &location.push(idx),
                        tracker,
                        ctx,
                    ));
                    max_index_applied = idx;
                }
                let annotation = if children.len() == items.len() {
                    Value::Bool(true)
                } else {
                    Value::from(max_index_applied)
                };
                let mut result = EvaluationResult::from_children(children);
                result.annotate(Annotations::new(annotation));
                return result;
            }
        }
        EvaluationResult::valid_empty()
    }
}

#[inline]
pub(crate) fn compile<'a>(
    ctx: &compiler::Context,
    _: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>> {
    if let Value::Array(items) = schema {
        Some(PrefixItemsValidator::compile(ctx, items))
    } else {
        let location = ctx.location().join("prefixItems");
        Some(Err(ValidationError::single_type_error(
            location.clone(),
            location,
            Location::new(),
            schema,
            JsonType::Array,
        )))
    }
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use serde_json::{json, Value};
    use test_case::test_case;

    fn evaluation_entry_sort_key(value: &Value) -> (&str, &str, &str) {
        let Some(entry) = value.as_object() else {
            return ("", "", "");
        };
        let evaluation_path = entry
            .get("evaluationPath")
            .and_then(Value::as_str)
            .unwrap_or("");
        let schema_location = entry
            .get("schemaLocation")
            .and_then(Value::as_str)
            .unwrap_or("");
        let instance_location = entry
            .get("instanceLocation")
            .and_then(Value::as_str)
            .unwrap_or("");
        (evaluation_path, schema_location, instance_location)
    }

    fn sort_details_arrays(value: &mut Value) {
        match value {
            Value::Object(map) => {
                for nested in map.values_mut() {
                    sort_details_arrays(nested);
                }
                if let Some(details) = map.get_mut("details").and_then(Value::as_array_mut) {
                    details.sort_by(|left, right| {
                        evaluation_entry_sort_key(left).cmp(&evaluation_entry_sort_key(right))
                    });
                }
            }
            Value::Array(items) => {
                for item in items {
                    sort_details_arrays(item);
                }
            }
            _ => {}
        }
    }

    fn sort_string_arrays(value: &mut Value) {
        match value {
            Value::Object(map) => {
                for nested in map.values_mut() {
                    sort_string_arrays(nested);
                }
            }
            Value::Array(items) => {
                for item in items.iter_mut() {
                    sort_string_arrays(item);
                }
                if items.iter().all(Value::is_string) {
                    items.sort_by(|left, right| {
                        left.as_str()
                            .unwrap_or("")
                            .cmp(right.as_str().unwrap_or(""))
                    });
                }
            }
            _ => {}
        }
    }

    fn normalize_evaluation_output(value: &mut Value) {
        sort_string_arrays(value);
        sort_details_arrays(value);
    }

    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "integer"}, {"maximum": 5}]}), &json!(["string"]), "/prefixItems/0/type")]
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "integer"}, {"maximum": 5}]}), &json!([42, 42]), "/prefixItems/1/maximum")]
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "integer"}, {"maximum": 5}], "items": {"type": "boolean"}}), &json!([42, 1, 42]), "/items/type")]
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "integer"}, {"maximum": 5}], "items": {"type": "boolean"}}), &json!([42, 42, true]), "/prefixItems/1/maximum")]
    fn location(schema: &Value, instance: &Value, expected: &str) {
        tests_util::assert_schema_location(schema, instance, expected);
    }

    #[test]
    fn evaluation_outputs_cover_prefix_items() {
        // Validator execution order: type (1) → required (26) → properties (40)
        let schema = json!({
            "type": "object",
            "properties": {"name": {"type": "string"}, "age": {"type": "number", "minimum": 0}},
            "required": ["name"]
        });
        let validator = crate::validator_for(&schema).expect("schema compiles");
        let evaluation = validator.evaluate(&json!({"name": "Alice", "age": 1}));

        let mut actual = serde_json::to_value(evaluation.list()).unwrap();
        normalize_evaluation_output(&mut actual);
        let mut expected = json!({
            "valid": true,
            "details": [
                {"valid":true, "evaluationPath": "", "schemaLocation": "", "instanceLocation": ""},
                {
                    "valid": true,
                    "evaluationPath": "/type",
                    "schemaLocation": "/type",
                    "instanceLocation": "",
                },
                {
                    "valid": true,
                    "evaluationPath": "/required",
                    "schemaLocation": "/required",
                    "instanceLocation": "",
                },
                {
                    "valid": true,
                    "evaluationPath": "/properties",
                    "schemaLocation": "/properties",
                    "instanceLocation": "",
                    "annotations": ["age", "name"]
                },

                {
                    "valid": true,
                    "evaluationPath": "/properties/age",
                    "schemaLocation": "/properties/age",
                    "instanceLocation": "/age",
                },
                {
                    "valid": true,
                    "evaluationPath": "/properties/age/type",
                    "schemaLocation": "/properties/age/type",
                    "instanceLocation": "/age",
                },
                {
                    "valid": true,
                    "evaluationPath": "/properties/age/minimum",
                    "schemaLocation": "/properties/age/minimum",
                    "instanceLocation": "/age",
                },
                {
                    "valid": true,
                    "evaluationPath": "/properties/name",
                    "schemaLocation": "/properties/name",
                    "instanceLocation": "/name",
                },
                {
                    "valid": true,
                    "evaluationPath": "/properties/name/type",
                    "schemaLocation": "/properties/name/type",
                    "instanceLocation": "/name",
                },
            ]
        });
        normalize_evaluation_output(&mut expected);
        assert_eq!(actual, expected);

        let mut actual = serde_json::to_value(evaluation.hierarchical()).unwrap();
        normalize_evaluation_output(&mut actual);
        let mut expected = json!({
            "valid": true,
            "evaluationPath": "",
            "schemaLocation": "",
            "instanceLocation": "",
            "details": [
                {
                    "valid": true,
                    "evaluationPath": "/type",
                    "schemaLocation": "/type",
                    "instanceLocation": "",
                },
                {
                    "valid": true,
                    "evaluationPath": "/required",
                    "schemaLocation": "/required",
                    "instanceLocation": "",
                },
                {
                    "valid": true,
                    "evaluationPath": "/properties",
                    "schemaLocation": "/properties",
                    "instanceLocation": "",
                    "annotations": ["age", "name"],
                    "details": [
                        {
                            "valid": true,
                            "evaluationPath": "/properties/age",
                            "instanceLocation": "/age",
                            "schemaLocation": "/properties/age",
                            "details": [
                                {
                                    "valid": true,
                                    "evaluationPath": "/properties/age/type",
                                    "instanceLocation": "/age",
                                    "schemaLocation": "/properties/age/type"
                                },
                                {
                                    "valid": true,
                                    "evaluationPath": "/properties/age/minimum",
                                    "instanceLocation": "/age",
                                    "schemaLocation": "/properties/age/minimum"
                                }
                            ]
                        },
                        {
                            "valid": true,
                            "evaluationPath": "/properties/name",
                            "instanceLocation": "/name",
                            "schemaLocation": "/properties/name",
                            "details": [
                                {
                                    "valid": true,
                                    "evaluationPath": "/properties/name/type",
                                    "instanceLocation": "/name",
                                    "schemaLocation": "/properties/name/type"
                                }
                            ]
                        }
                    ]
                }
            ]
        });
        normalize_evaluation_output(&mut expected);
        assert_eq!(actual, expected);
    }
}
