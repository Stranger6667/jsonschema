use crate::{
    compiler,
    error::{error, no_error, ErrorIterator, ValidationError},
    node::SchemaNode,
    paths::{LazyLocation, Location, RefTracker},
    types::JsonType,
    validator::{EvaluationResult, Validate, ValidationContext},
};
use serde_json::{Map, Value};

use super::CompilationResult;

pub(crate) struct AnyOfValidator {
    schemas: Vec<SchemaNode>,
    location: Location,
}

impl AnyOfValidator {
    #[inline]
    pub(crate) fn compile<'a>(ctx: &compiler::Context, schema: &'a Value) -> CompilationResult<'a> {
        if let Value::Array(items) = schema {
            let ctx = ctx.new_at_location("anyOf");
            let mut schemas = Vec::with_capacity(items.len());
            for (idx, item) in items.iter().enumerate() {
                let ctx = ctx.new_at_location(idx);
                let node = compiler::compile(&ctx, ctx.as_resource_ref(item))?;
                schemas.push(node);
            }
            Ok(Box::new(AnyOfValidator {
                schemas,
                location: ctx.location().clone(),
            }))
        } else {
            let location = ctx.location().join("anyOf");
            Err(ValidationError::single_type_error(
                location.clone(),
                location,
                Location::new(),
                schema,
                JsonType::Array,
            ))
        }
    }
}

impl Validate for AnyOfValidator {
    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        self.schemas.iter().any(|s| s.is_valid(instance, ctx))
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if self.is_valid(instance, ctx) {
            Ok(())
        } else {
            Err(ValidationError::any_of(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance,
                self.schemas
                    .iter()
                    .map(|schema| {
                        schema
                            .iter_errors(instance, location, tracker, ctx)
                            .collect()
                    })
                    .collect(),
            ))
        }
    }

    fn iter_errors<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if self.is_valid(instance, ctx) {
            no_error()
        } else {
            error(ValidationError::any_of(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance,
                self.schemas
                    .iter()
                    .map(|schema| {
                        schema
                            .iter_errors(instance, location, tracker, ctx)
                            .collect()
                    })
                    .collect(),
            ))
        }
    }

    fn evaluate(
        &self,
        instance: &Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        // For anyOf, we only need ONE valid schema. Find first valid with cheap is_valid,
        // then evaluate only that one. No need to collect all valid indices.
        let first_valid = self
            .schemas
            .iter()
            .enumerate()
            .find(|(_, node)| node.is_valid(instance, ctx));

        if let Some((idx, _)) = first_valid {
            // Found a valid schema - evaluate just that one
            let result = self.schemas[idx].evaluate_instance(instance, location, tracker, ctx);
            EvaluationResult::from_children(vec![result])
        } else {
            // No valid schemas - evaluate all for error output
            let failures: Vec<_> = self
                .schemas
                .iter()
                .map(|node| node.evaluate_instance(instance, location, tracker, ctx))
                .collect();
            EvaluationResult::from_children(failures)
        }
    }
}

/// Optimized validator for `anyOf` with a single subschema.
pub(crate) struct SingleAnyOfValidator {
    node: SchemaNode,
    location: Location,
}

impl SingleAnyOfValidator {
    #[inline]
    pub(crate) fn compile<'a>(ctx: &compiler::Context, schema: &'a Value) -> CompilationResult<'a> {
        let any_of_ctx = ctx.new_at_location("anyOf");
        let item_ctx = any_of_ctx.new_at_location(0);
        let node = compiler::compile(&item_ctx, item_ctx.as_resource_ref(schema))?;
        Ok(Box::new(SingleAnyOfValidator {
            node,
            location: any_of_ctx.location().clone(),
        }))
    }
}

impl Validate for SingleAnyOfValidator {
    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        self.node.is_valid(instance, ctx)
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if self.node.is_valid(instance, ctx) {
            Ok(())
        } else {
            Err(ValidationError::any_of(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance,
                vec![self
                    .node
                    .iter_errors(instance, location, tracker, ctx)
                    .collect()],
            ))
        }
    }

    fn iter_errors<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if self.node.is_valid(instance, ctx) {
            no_error()
        } else {
            error(ValidationError::any_of(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance,
                vec![self
                    .node
                    .iter_errors(instance, location, tracker, ctx)
                    .collect()],
            ))
        }
    }

    fn evaluate(
        &self,
        instance: &Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        EvaluationResult::from(
            self.node
                .evaluate_instance(instance, location, tracker, ctx),
        )
    }
}

#[inline]
pub(crate) fn compile<'a>(
    ctx: &compiler::Context,
    _: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>> {
    if let Value::Array(items) = schema {
        match items.as_slice() {
            [item] => Some(SingleAnyOfValidator::compile(ctx, item)),
            _ => Some(AnyOfValidator::compile(ctx, schema)),
        }
    } else {
        let location = ctx.location().join("anyOf");
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

    #[test_case(&json!({"anyOf": [{"type": "string"}]}), &json!(1), "/anyOf")]
    #[test_case(&json!({"anyOf": [{"type": "integer"}, {"type": "string"}]}), &json!({}), "/anyOf")]
    fn location(schema: &Value, instance: &Value, expected: &str) {
        tests_util::assert_schema_location(schema, instance, expected);
    }
}
