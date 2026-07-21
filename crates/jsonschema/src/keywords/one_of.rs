use std::borrow::Cow;

use crate::{
    compiler,
    error::ValidationError,
    evaluation::ErrorDescription,
    keywords::CompilationResult,
    node::SchemaNode,
    paths::{LazyLocation, Location, RefTracker},
    types::JsonType,
    validator::{EvaluationResult, Validate, ValidationContext},
    Json, JsonNode, SerdeJson,
};
use serde_json::{Map, Value};

pub(crate) struct OneOfValidator<F: Json> {
    schemas: Vec<SchemaNode<F>>,
    location: Location,
}

impl OneOfValidator<SerdeJson> {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        schema: &'a Value,
    ) -> CompilationResult<'a, F> {
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
                Cow::Borrowed(schema),
                JsonType::Array,
            ))
        }
    }
}

impl<F: Json> OneOfValidator<F> {
    fn get_first_valid(
        &self,
        instance: &F::Node<'_>,
        ctx: &mut ValidationContext,
    ) -> Option<usize> {
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
    fn are_others_valid(
        &self,
        instance: &F::Node<'_>,
        idx: usize,
        ctx: &mut ValidationContext,
    ) -> bool {
        self.schemas
            .iter()
            .skip(idx + 1)
            .any(|n| n.is_valid(instance, ctx))
    }
}

/// Optimized validator for `oneOf` with a single subschema.
/// With exactly one schema, `oneOf` behaves identically to `anyOf`.
pub(crate) struct SingleOneOfValidator<F: Json> {
    node: SchemaNode<F>,
    location: Location,
}

impl SingleOneOfValidator<SerdeJson> {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        schema: &'a Value,
    ) -> CompilationResult<'a, F> {
        let one_of_ctx = ctx.new_at_location("oneOf");
        let item_ctx = one_of_ctx.new_at_location(0);
        let node = compiler::compile(&item_ctx, item_ctx.as_resource_ref(schema))?;
        Ok(Box::new(SingleOneOfValidator {
            node,
            location: one_of_ctx.location().clone(),
        }))
    }
}

impl<F: Json> Validate<F> for SingleOneOfValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        self.node.is_valid(instance, ctx)
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
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
                instance.to_value(),
                vec![self
                    .node
                    .iter_errors(instance, location, tracker, ctx)
                    .collect()],
            ))
        }
    }

    fn evaluate(
        &self,
        instance: &F::Node<'_>,
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

impl<F: Json> Validate<F> for OneOfValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        let first_valid_idx = self.get_first_valid(instance, ctx);
        first_valid_idx.is_some_and(|idx| !self.are_others_valid(instance, idx, ctx))
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
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
                    instance.to_value(),
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
                instance.to_value(),
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
        instance: &F::Node<'_>,
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
}

#[inline]
pub(crate) fn compile<'a, F: Json>(
    ctx: &compiler::Context<F>,
    _: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a, F>> {
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
}
