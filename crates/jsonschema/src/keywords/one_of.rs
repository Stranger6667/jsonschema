use crate::{
    compiler,
    error::ValidationError,
    evaluation::ErrorDescription,
    keywords::CompilationResult,
    node::SchemaNode,
    paths::{LazyLocation, LazyRefPath, Location},
    types::JsonType,
    validator::{capture_evaluation_path, EvaluationResult, Validate, ValidationContext},
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

impl Validate for OneOfValidator {
    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        let first_valid_idx = self.get_first_valid(instance, ctx);
        first_valid_idx.is_some_and(|idx| !self.are_others_valid(instance, idx, ctx))
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        evaluation_path: &LazyRefPath,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        let first_valid_idx = self.get_first_valid(instance, ctx);
        if let Some(idx) = first_valid_idx {
            if self.are_others_valid(instance, idx, ctx) {
                return Err(ValidationError::one_of_multiple_valid(
                    self.location.clone(),
                    capture_evaluation_path(&self.location, evaluation_path),
                    location.into(),
                    instance,
                    self.schemas
                        .iter()
                        .map(|schema| {
                            schema
                                .iter_errors(instance, location, evaluation_path, ctx)
                                .collect()
                        })
                        .collect(),
                ));
            }
            Ok(())
        } else {
            Err(ValidationError::one_of_not_valid(
                self.location.clone(),
                capture_evaluation_path(&self.location, evaluation_path),
                location.into(),
                instance,
                self.schemas
                    .iter()
                    .map(|schema| {
                        schema
                            .iter_errors(instance, location, evaluation_path, ctx)
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
        evaluation_path: &LazyRefPath,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        // Use cheap `is_valid` first, then run full `evaluate` only on matching schemas.
        let first_valid_idx = self.get_first_valid(instance, ctx);

        let Some(first_idx) = first_valid_idx else {
            let failures: Vec<_> = self
                .schemas
                .iter()
                .map(|node| node.evaluate_instance(instance, location, evaluation_path, ctx))
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
                    let child = node.evaluate_instance(instance, location, evaluation_path, ctx);
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
            let child =
                self.schemas[first_idx].evaluate_instance(instance, location, evaluation_path, ctx);
            EvaluationResult::from(child)
        }
    }
}

#[inline]
pub(crate) fn compile<'a>(
    ctx: &compiler::Context,
    _: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>> {
    Some(OneOfValidator::compile(ctx, schema))
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
