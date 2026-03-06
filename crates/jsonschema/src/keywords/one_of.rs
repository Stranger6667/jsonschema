use crate::{
    compiler,
    error::ValidationError,
    evaluation::ErrorDescription,
    keywords::CompilationResult,
    node::SchemaNode,
    paths::{LazyLocation, Location, RefTracker},
    types::JsonType,
    validator::{EvaluationResult, Validate, ValidationContext},
};
use serde_json::{Map, Value};

pub(crate) struct OneOfValidator {
    schemas: Vec<SchemaNode>,
    location: Location,
}

impl OneOfValidator {
    #[inline]
    pub(crate) fn compile<'a>(ctx: &compiler::Context, schema: &'a Value) -> CompilationResult<'a> {
        if let Value::Array(items) = schema {
            let ctx = ctx.new_at_location("oneOf");
            let mut schemas = Vec::with_capacity(items.len());
            for (idx, item) in items.iter().enumerate() {
                let ctx = ctx.new_at_location(idx);
                let node = compiler::compile(&ctx, ctx.as_resource_ref(item))?;
                schemas.push(node);
            }
            Ok(Box::new(OneOfValidator {
                schemas,
                location: ctx.location().clone(),
            }))
        } else {
            let location = ctx.location().join("oneOf");
            Err(ValidationError::single_type_error(
                location.clone(),
                location,
                Location::new(),
                schema,
                JsonType::Array,
            ))
        }
    }

    fn get_first_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> Option<usize> {
        let mut first_valid_idx = None;
        for (idx, node) in self.schemas.iter().enumerate() {
            if node.is_valid(instance, ctx) {
                first_valid_idx = Some(idx);
                break;
            }
        }
        first_valid_idx
    }

    #[allow(clippy::arithmetic_side_effects)]
    fn are_others_valid(&self, instance: &Value, idx: usize, ctx: &mut ValidationContext) -> bool {
        self.schemas
            .iter()
            .skip(idx + 1)
            .any(|n| n.is_valid(instance, ctx))
    }
}

/// Optimized validator for `oneOf` with a single subschema.
/// With exactly one schema, `oneOf` behaves identically to `anyOf`.
pub(crate) struct SingleOneOfValidator {
    node: SchemaNode,
    location: Location,
}

impl SingleOneOfValidator {
    #[inline]
    pub(crate) fn compile<'a>(ctx: &compiler::Context, schema: &'a Value) -> CompilationResult<'a> {
        let one_of_ctx = ctx.new_at_location("oneOf");
        let item_ctx = one_of_ctx.new_at_location(0);
        let node = compiler::compile(&item_ctx, item_ctx.as_resource_ref(schema))?;
        Ok(Box::new(SingleOneOfValidator {
            node,
            location: one_of_ctx.location().clone(),
        }))
    }
}

impl Validate for SingleOneOfValidator {
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
            Err(ValidationError::one_of_not_valid(
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

    fn matches_type(&self, _: &Value) -> bool {
        true
    }

    fn schema_path(&self) -> &Location {
        &self.location
    }

    fn trace(
        &self,
        instance: &Value,
        instance_path: &LazyLocation,
        callback: crate::tracing::TracingCallback<'_>,
        ctx: &mut ValidationContext,
    ) -> bool {
        let is_valid = self.node.trace(instance, instance_path, callback, ctx);
        crate::tracing::TracingContext::new(instance_path, self.schema_path(), is_valid)
            .call(callback);
        is_valid
    }
}

impl Validate for OneOfValidator {
    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        let first_valid_idx = self.get_first_valid(instance, ctx);
        first_valid_idx.is_some_and(|idx| !self.are_others_valid(instance, idx, ctx))
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        let first_valid_idx = self.get_first_valid(instance, ctx);
        if let Some(idx) = first_valid_idx {
            if self.are_others_valid(instance, idx, ctx) {
                return Err(ValidationError::one_of_multiple_valid(
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
                ));
            }
            Ok(())
        } else {
            Err(ValidationError::one_of_not_valid(
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
        // Use cheap `is_valid` first, then run full `evaluate` only on matching schemas.
        let first_valid_idx = self.get_first_valid(instance, ctx);

        let Some(first_idx) = first_valid_idx else {
            let failures: Vec<_> = self
                .schemas
                .iter()
                .map(|node| node.evaluate_instance(instance, location, tracker, ctx))
                .collect();
            return EvaluationResult::Invalid {
                errors: Vec::new(),
                children: failures,
                annotations: None,
            };
        };

        if self.are_others_valid(instance, first_idx, ctx) {
            let mut successes = Vec::new();
            for (idx, node) in self.schemas.iter().enumerate() {
                if idx == first_idx || node.is_valid(instance, ctx) {
                    let child = node.evaluate_instance(instance, location, tracker, ctx);
                    if child.valid {
                        successes.push(child);
                    }
                }
            }
            EvaluationResult::Invalid {
                errors: vec![ErrorDescription::new(
                    "oneOf",
                    "more than one subschema succeeded".to_string(),
                )],
                children: successes,
                annotations: None,
            }
        } else {
            let child = self.schemas[first_idx].evaluate_instance(instance, location, tracker, ctx);
            EvaluationResult::from(child)
        }
    }
    fn matches_type(&self, _: &Value) -> bool {
        true
    }
    fn schema_path(&self) -> &Location {
        &self.location
    }
    fn trace(
        &self,
        instance: &Value,
        instance_path: &LazyLocation,
        callback: crate::tracing::TracingCallback<'_>,
        ctx: &mut ValidationContext,
    ) -> bool {
        let mut valid_count = 0;
        for node in &self.schemas {
            let schema_is_valid = node.trace(instance, instance_path, callback, ctx);
            crate::tracing::TracingContext::new(instance_path, node.schema_path(), schema_is_valid)
                .call(callback);
            if schema_is_valid {
                valid_count += 1;
            }
        }
        // oneOf is valid if exactly one branch matches
        let is_valid = valid_count == 1;
        crate::tracing::TracingContext::new(instance_path, self.schema_path(), is_valid)
            .call(callback);
        is_valid
    }
}

#[inline]
pub(crate) fn compile<'a>(
    ctx: &compiler::Context,
    _: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>> {
    match schema {
        Value::Array(items) => match items.as_slice() {
            [item] => Some(SingleOneOfValidator::compile(ctx, item)),
            _ => Some(OneOfValidator::compile(ctx, schema)),
        },
        _ => Some(OneOfValidator::compile(ctx, schema)),
    }
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use serde_json::{json, Value};
    use test_case::test_case;

    #[test_case(&json!({"oneOf": [{"type": "string"}]}), &json!(0), "/oneOf")]
    #[test_case(&json!({"oneOf": [{"type": "string"}, {"maxLength": 3}]}), &json!(""), "/oneOf")]
    fn location(schema: &Value, instance: &Value, expected: &str) {
        tests_util::assert_schema_location(schema, instance, expected);
    }

    #[test]
    fn trace_single_one_of_propagates() {
        let schema = serde_json::json!({"oneOf": [{"type": "string"}]});
        let validator = crate::validator_for(&schema).unwrap();
        let instance = serde_json::json!("hello");
        let mut schema_locations: Vec<String> = Vec::new();
        let _ = validator.trace(&instance, &mut |ctx| {
            schema_locations.push(ctx.schema_location.as_str().to_string());
        });
        assert!(
            schema_locations.iter().any(|s| s == "/oneOf/0/type"),
            "expected /oneOf/0/type in {schema_locations:?}"
        );
        assert!(
            schema_locations.iter().any(|s| s == "/oneOf"),
            "expected /oneOf in {schema_locations:?}"
        );
    }

    #[test]
    fn trace_single_one_of_failing_instance() {
        let schema = serde_json::json!({"oneOf": [{"type": "string"}]});
        let validator = crate::validator_for(&schema).unwrap();
        let instance = serde_json::json!(42); // fails type: string
        let mut schema_locations: Vec<String> = Vec::new();
        let result = validator.trace(&instance, &mut |ctx| {
            schema_locations.push(ctx.schema_location.as_str().to_string());
        });
        assert!(!result, "expected validation to fail");
        assert!(
            schema_locations.iter().any(|s| s == "/oneOf"),
            "expected /oneOf in {schema_locations:?}"
        );
    }
}
