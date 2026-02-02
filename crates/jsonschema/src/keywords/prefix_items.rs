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

/// Fused validator for single-item tuples. Avoids Vec overhead and iterator machinery.
pub(crate) struct PrefixItems1Validator {
    schema: SchemaNode,
}

/// Fused validator for two-item tuples. Avoids Vec overhead and iterator machinery.
pub(crate) struct PrefixItems2Validator {
    first: SchemaNode,
    second: SchemaNode,
}

/// Fused validator for three-item tuples. Avoids Vec overhead and iterator machinery.
pub(crate) struct PrefixItems3Validator {
    first: SchemaNode,
    second: SchemaNode,
    third: SchemaNode,
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

impl PrefixItems1Validator {
    #[inline]
    pub(crate) fn compile<'a>(
        ctx: &compiler::Context,
        items: &'a [Value],
    ) -> CompilationResult<'a> {
        let ctx = ctx.new_at_location("prefixItems");
        let schema = compiler::compile(&ctx.new_at_location(0), ctx.as_resource_ref(&items[0]))?;
        Ok(Box::new(PrefixItems1Validator { schema }))
    }
}

impl PrefixItems2Validator {
    #[inline]
    pub(crate) fn compile<'a>(
        ctx: &compiler::Context,
        items: &'a [Value],
    ) -> CompilationResult<'a> {
        let ctx = ctx.new_at_location("prefixItems");
        let first = compiler::compile(&ctx.new_at_location(0), ctx.as_resource_ref(&items[0]))?;
        let second = compiler::compile(&ctx.new_at_location(1), ctx.as_resource_ref(&items[1]))?;
        Ok(Box::new(PrefixItems2Validator { first, second }))
    }
}

impl PrefixItems3Validator {
    #[inline]
    pub(crate) fn compile<'a>(
        ctx: &compiler::Context,
        items: &'a [Value],
    ) -> CompilationResult<'a> {
        let ctx = ctx.new_at_location("prefixItems");
        let first = compiler::compile(&ctx.new_at_location(0), ctx.as_resource_ref(&items[0]))?;
        let second = compiler::compile(&ctx.new_at_location(1), ctx.as_resource_ref(&items[1]))?;
        let third = compiler::compile(&ctx.new_at_location(2), ctx.as_resource_ref(&items[2]))?;
        Ok(Box::new(PrefixItems3Validator {
            first,
            second,
            third,
        }))
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

impl Validate for PrefixItems1Validator {
    #[inline]
    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        if let Value::Array(items) = instance {
            if let Some(first) = items.first() {
                self.schema.is_valid(first, ctx)
            } else {
                true
            }
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
            if let Some(first) = items.first() {
                self.schema
                    .validate(first, &location.push(0), tracker, ctx)?;
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
            if let Some(first) = items.first() {
                return self
                    .schema
                    .iter_errors(first, &location.push(0), tracker, ctx);
            }
        }
        no_error()
    }

    fn evaluate(
        &self,
        instance: &Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Value::Array(items) = instance {
            if let Some(first) = items.first() {
                let child = self
                    .schema
                    .evaluate_instance(first, &location.push(0), tracker, ctx);
                let annotation = if items.len() == 1 {
                    Value::Bool(true)
                } else {
                    Value::from(0)
                };
                let mut result = EvaluationResult::from_children(vec![child]);
                result.annotate(Annotations::new(annotation));
                return result;
            }
        }
        EvaluationResult::valid_empty()
    }
}

impl Validate for PrefixItems2Validator {
    #[inline]
    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        if let Value::Array(items) = instance {
            if let Some(first) = items.first() {
                if !self.first.is_valid(first, ctx) {
                    return false;
                }
            }
            if let Some(second) = items.get(1) {
                if !self.second.is_valid(second, ctx) {
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
            if let Some(first) = items.first() {
                self.first
                    .validate(first, &location.push(0), tracker, ctx)?;
            }
            if let Some(second) = items.get(1) {
                self.second
                    .validate(second, &location.push(1), tracker, ctx)?;
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
            if let Some(first) = items.first() {
                errors.extend(
                    self.first
                        .iter_errors(first, &location.push(0), tracker, ctx),
                );
            }
            if let Some(second) = items.get(1) {
                errors.extend(
                    self.second
                        .iter_errors(second, &location.push(1), tracker, ctx),
                );
            }
            if !errors.is_empty() {
                return ErrorIterator::from_iterator(errors.into_iter());
            }
        }
        no_error()
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
                let mut children = Vec::new();
                let mut max_index = 0;
                if let Some(first) = items.first() {
                    children.push(self.first.evaluate_instance(
                        first,
                        &location.push(0),
                        tracker,
                        ctx,
                    ));
                }
                if let Some(second) = items.get(1) {
                    children.push(self.second.evaluate_instance(
                        second,
                        &location.push(1),
                        tracker,
                        ctx,
                    ));
                    max_index = 1;
                }
                let annotation = if children.len() == items.len() {
                    Value::Bool(true)
                } else {
                    Value::from(max_index)
                };
                let mut result = EvaluationResult::from_children(children);
                result.annotate(Annotations::new(annotation));
                return result;
            }
        }
        EvaluationResult::valid_empty()
    }
}

impl Validate for PrefixItems3Validator {
    #[inline]
    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        if let Value::Array(items) = instance {
            if let Some(first) = items.first() {
                if !self.first.is_valid(first, ctx) {
                    return false;
                }
            }
            if let Some(second) = items.get(1) {
                if !self.second.is_valid(second, ctx) {
                    return false;
                }
            }
            if let Some(third) = items.get(2) {
                if !self.third.is_valid(third, ctx) {
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
            if let Some(first) = items.first() {
                self.first
                    .validate(first, &location.push(0), tracker, ctx)?;
            }
            if let Some(second) = items.get(1) {
                self.second
                    .validate(second, &location.push(1), tracker, ctx)?;
            }
            if let Some(third) = items.get(2) {
                self.third
                    .validate(third, &location.push(2), tracker, ctx)?;
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
            if let Some(first) = items.first() {
                errors.extend(
                    self.first
                        .iter_errors(first, &location.push(0), tracker, ctx),
                );
            }
            if let Some(second) = items.get(1) {
                errors.extend(
                    self.second
                        .iter_errors(second, &location.push(1), tracker, ctx),
                );
            }
            if let Some(third) = items.get(2) {
                errors.extend(
                    self.third
                        .iter_errors(third, &location.push(2), tracker, ctx),
                );
            }
            if !errors.is_empty() {
                return ErrorIterator::from_iterator(errors.into_iter());
            }
        }
        no_error()
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
                let mut children = Vec::new();
                let mut max_index = 0;
                if let Some(first) = items.first() {
                    children.push(self.first.evaluate_instance(
                        first,
                        &location.push(0),
                        tracker,
                        ctx,
                    ));
                }
                if let Some(second) = items.get(1) {
                    children.push(self.second.evaluate_instance(
                        second,
                        &location.push(1),
                        tracker,
                        ctx,
                    ));
                    max_index = 1;
                }
                if let Some(third) = items.get(2) {
                    children.push(self.third.evaluate_instance(
                        third,
                        &location.push(2),
                        tracker,
                        ctx,
                    ));
                    max_index = 2;
                }
                let annotation = if children.len() == items.len() {
                    Value::Bool(true)
                } else {
                    Value::from(max_index)
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
        // Use specialized validators for small tuples to avoid Vec overhead
        Some(match items.len() {
            1 => PrefixItems1Validator::compile(ctx, items),
            2 => PrefixItems2Validator::compile(ctx, items),
            3 => PrefixItems3Validator::compile(ctx, items),
            _ => PrefixItemsValidator::compile(ctx, items),
        })
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

        assert_eq!(
            serde_json::to_value(evaluation.list()).unwrap(),
            json!({
                "valid": true,
                "details": [
                    {"evaluationPath": "", "instanceLocation": "", "schemaLocation": "", "valid": true},
                    {
                        "valid": true,
                        "evaluationPath": "/type",
                        "instanceLocation": "",
                        "schemaLocation": "/type"
                    },
                    {
                        "valid": true,
                        "evaluationPath": "/required",
                        "instanceLocation": "",
                        "schemaLocation": "/required"
                    },
                    {
                        "valid": true,
                        "evaluationPath": "/properties",
                        "instanceLocation": "",
                        "schemaLocation": "/properties",
                        "annotations": ["age", "name"]
                    },
                    {
                        "valid": true,
                        "evaluationPath": "/properties/age",
                        "instanceLocation": "/age",
                        "schemaLocation": "/properties/age"
                    },
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
                    },
                    {
                        "valid": true,
                        "evaluationPath": "/properties/name",
                        "instanceLocation": "/name",
                        "schemaLocation": "/properties/name"
                    },
                    {
                        "valid": true,
                        "evaluationPath": "/properties/name/type",
                        "instanceLocation": "/name",
                        "schemaLocation": "/properties/name/type"
                    }
                ]
            })
        );

        assert_eq!(
            serde_json::to_value(evaluation.hierarchical()).unwrap(),
            json!({
                "valid": true,
                "evaluationPath": "",
                "instanceLocation": "",
                "schemaLocation": "",
                "details": [
                    {
                        "valid": true,
                        "evaluationPath": "/type",
                        "instanceLocation": "",
                        "schemaLocation": "/type"
                    },
                    {
                        "valid": true,
                        "evaluationPath": "/required",
                        "instanceLocation": "",
                        "schemaLocation": "/required"
                    },
                    {
                        "valid": true,
                        "evaluationPath": "/properties",
                        "instanceLocation": "",
                        "schemaLocation": "/properties",
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
            })
        );
    }

    // Tests for fused PrefixItems1Validator
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "integer"}]}), &json!([42]), true)]
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "integer"}]}), &json!([42, "extra"]), true)]
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "integer"}]}), &json!(["invalid"]), false)]
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "integer"}]}), &json!([]), true)]
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "integer"}]}), &json!("not-array"), true)]
    fn fused_prefix_items_1_is_valid(schema: &Value, instance: &Value, expected: bool) {
        let validator = crate::validator_for(schema).unwrap();
        assert_eq!(validator.is_valid(instance), expected);
    }

    // Tests for fused PrefixItems2Validator
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "integer"}, {"type": "string"}]}), &json!([42, "hello"]), true)]
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "integer"}, {"type": "string"}]}), &json!([42, "hello", true]), true)]
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "integer"}, {"type": "string"}]}), &json!(["invalid", "hello"]), false)]
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "integer"}, {"type": "string"}]}), &json!([42, 99]), false)]
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "integer"}, {"type": "string"}]}), &json!([42]), true)]
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "integer"}, {"type": "string"}]}), &json!([]), true)]
    fn fused_prefix_items_2_is_valid(schema: &Value, instance: &Value, expected: bool) {
        let validator = crate::validator_for(schema).unwrap();
        assert_eq!(validator.is_valid(instance), expected);
    }

    // Tests for fused PrefixItems3Validator
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "integer"}, {"type": "string"}, {"type": "boolean"}]}), &json!([42, "hello", true]), true)]
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "integer"}, {"type": "string"}, {"type": "boolean"}]}), &json!([42, "hello", true, 99]), true)]
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "integer"}, {"type": "string"}, {"type": "boolean"}]}), &json!(["invalid", "hello", true]), false)]
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "integer"}, {"type": "string"}, {"type": "boolean"}]}), &json!([42, 99, true]), false)]
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "integer"}, {"type": "string"}, {"type": "boolean"}]}), &json!([42, "hello", "invalid"]), false)]
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "integer"}, {"type": "string"}, {"type": "boolean"}]}), &json!([42, "hello"]), true)]
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "integer"}, {"type": "string"}, {"type": "boolean"}]}), &json!([42]), true)]
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "integer"}, {"type": "string"}, {"type": "boolean"}]}), &json!([]), true)]
    fn fused_prefix_items_3_is_valid(schema: &Value, instance: &Value, expected: bool) {
        let validator = crate::validator_for(schema).unwrap();
        assert_eq!(validator.is_valid(instance), expected);
    }

    // Test validation errors for fused validators
    #[test]
    fn fused_prefix_items_1_validate_error() {
        let schema = json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "integer"}]});
        let validator = crate::validator_for(&schema).unwrap();
        let instance = json!(["not-an-integer"]);
        let result = validator.validate(&instance);
        assert!(result.is_err());
    }

    #[test]
    fn fused_prefix_items_2_validate_error() {
        let schema = json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "integer"}, {"type": "string"}]});
        let validator = crate::validator_for(&schema).unwrap();
        let instance = json!([42, 99]);
        let result = validator.validate(&instance);
        assert!(result.is_err());
    }

    #[test]
    fn fused_prefix_items_3_validate_error() {
        let schema = json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "integer"}, {"type": "string"}, {"type": "boolean"}]});
        let validator = crate::validator_for(&schema).unwrap();
        let instance = json!([42, "hello", "not-a-bool"]);
        let result = validator.validate(&instance);
        assert!(result.is_err());
    }

    // Test error collection with iter_errors
    #[test]
    fn fused_prefix_items_2_iter_errors() {
        let schema = json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "integer"}, {"type": "string"}]});
        let validator = crate::validator_for(&schema).unwrap();
        let instance = json!(["not-int", 99]);
        let errors: Vec<_> = validator.iter_errors(&instance).collect();
        assert_eq!(errors.len(), 2); // Both items should fail
    }

    #[test]
    fn fused_prefix_items_3_iter_errors() {
        let schema = json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "integer"}, {"type": "string"}, {"type": "boolean"}]});
        let validator = crate::validator_for(&schema).unwrap();
        let instance = json!(["not-int", 99, "not-bool"]);
        let errors: Vec<_> = validator.iter_errors(&instance).collect();
        assert_eq!(errors.len(), 3); // All three items should fail
    }

    // Test that fused validators work with items keyword
    #[test]
    fn fused_prefix_items_with_items() {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "prefixItems": [{"type": "integer"}, {"type": "string"}],
            "items": {"type": "boolean"}
        });
        let validator = crate::validator_for(&schema).unwrap();
        assert!(validator.is_valid(&json!([42, "hello", true, false])));
        assert!(!validator.is_valid(&json!([42, "hello", "invalid"])));
    }

    // Test annotations for fused validators
    #[test]
    fn fused_prefix_items_1_annotation() {
        let schema = json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "integer"}]});
        let validator = crate::validator_for(&schema).unwrap();

        // When all items are covered
        let instance = json!([42]);
        let eval = validator.evaluate(&instance);
        assert!(eval.flag().valid);

        // When there are extra items
        let instance = json!([42, "extra"]);
        let eval = validator.evaluate(&instance);
        assert!(eval.flag().valid);
    }

    #[test]
    fn fused_prefix_items_2_annotation() {
        let schema = json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "integer"}, {"type": "string"}]});
        let validator = crate::validator_for(&schema).unwrap();

        // When all items are covered
        let instance = json!([42, "hello"]);
        let eval = validator.evaluate(&instance);
        assert!(eval.flag().valid);

        // When there are extra items
        let instance = json!([42, "hello", true]);
        let eval = validator.evaluate(&instance);
        assert!(eval.flag().valid);
    }

    #[test]
    fn fused_prefix_items_3_annotation() {
        let schema = json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "prefixItems": [{"type": "integer"}, {"type": "string"}, {"type": "boolean"}]});
        let validator = crate::validator_for(&schema).unwrap();

        // When all items are covered
        let instance = json!([42, "hello", true]);
        let eval = validator.evaluate(&instance);
        assert!(eval.flag().valid);

        // When there are extra items
        let instance = json!([42, "hello", true, 99]);
        let eval = validator.evaluate(&instance);
        assert!(eval.flag().valid);
    }
}
