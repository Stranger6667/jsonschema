use crate::LazyInstance;
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
    Json, Node, SerdeJson,
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
                LazyInstance::Ready(Cow::Borrowed(schema)),
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
                instance.lazy_value(),
                vec![{
                    let mut branch = Vec::new();
                    self.node
                        .collect_errors(instance, location, tracker, ctx, &mut branch);
                    branch
                }],
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
        self.evaluate_with_location(instance, location, &location.into(), tracker, ctx)
    }

    fn evaluate_with_location(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        instance_location: &Location,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        EvaluationResult::from(self.node.evaluate_instance_at(
            instance,
            location,
            instance_location,
            tracker,
            ctx,
        ))
    }

    fn schema_path(&self) -> &Location {
        &self.location
    }

    fn trace(
        &self,
        instance: &F::Node<'_>,
        instance_path: &LazyLocation,
        callback: crate::tracing::TracingCallback<'_>,
        ctx: &mut ValidationContext,
    ) -> bool {
        let is_valid = self.node.trace(instance, instance_path, callback, ctx);
        crate::tracing::TracingContext::new(instance_path, self.node.schema_path(), is_valid)
            .call(callback);
        crate::tracing::TracingContext::new(instance_path, self.schema_path(), is_valid)
            .call(callback);
        is_valid
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
                    instance.lazy_value(),
                    self.schemas
                        .iter()
                        .map(|schema| {
                            let mut branch = Vec::new();
                            schema.collect_errors(instance, location, tracker, ctx, &mut branch);
                            branch
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
                instance.lazy_value(),
                self.schemas
                    .iter()
                    .map(|schema| {
                        let mut branch = Vec::new();
                        schema.collect_errors(instance, location, tracker, ctx, &mut branch);
                        branch
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
        self.evaluate_with_location(instance, location, &location.into(), tracker, ctx)
    }

    fn evaluate_with_location(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        instance_location: &Location,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        // Use cheap `is_valid` first, then run full `evaluate` only on matching schemas.
        let first_valid_idx = self.get_first_valid(instance, ctx);

        let Some(first_idx) = first_valid_idx else {
            let failures: Vec<_> = self
                .schemas
                .iter()
                .map(|node| {
                    node.evaluate_instance_at(instance, location, instance_location, tracker, ctx)
                })
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
                    let child = node.evaluate_instance_at(
                        instance,
                        location,
                        instance_location,
                        tracker,
                        ctx,
                    );
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
            let child = self.schemas[first_idx].evaluate_instance_at(
                instance,
                location,
                instance_location,
                tracker,
                ctx,
            );
            EvaluationResult::from(child)
        }
    }
    fn matches_type(&self, _: &F::Node<'_>) -> bool {
        true
    }
    fn schema_path(&self) -> &Location {
        &self.location
    }
    fn trace(
        &self,
        instance: &F::Node<'_>,
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
    use crate::{
        paths::Location,
        tests_util,
        tracing::NodeEvaluationResult::{self, Invalid, Valid},
        Keyword, ValidationError,
    };
    use serde_json::{json, Map, Value};
    use test_case::test_case;

    #[test_case(&json!({"oneOf": [{"type": "string"}]}), &json!(0), "/oneOf")]
    #[test_case(&json!({"oneOf": [{"type": "string"}, {"maxLength": 3}]}), &json!(""), "/oneOf")]
    fn location(schema: &Value, instance: &Value, expected: &str) {
        tests_util::assert_schema_location(schema, instance, expected);
    }

    // A single branch is traced like the branches of a longer `oneOf`.
    #[test_case(&json!({"oneOf": [{"type": "integer"}, {"type": "string"}]}), &json!(1), &[("/oneOf/0/type", Valid), ("/oneOf/0", Valid), ("/oneOf/1/type", Invalid), ("/oneOf/1", Invalid), ("/oneOf", Valid), ("", Valid)]; "two branches")]
    #[test_case(&json!({"oneOf": [{"type": "integer"}]}), &json!(1), &[("/oneOf/0/type", Valid), ("/oneOf/0", Valid), ("/oneOf", Valid), ("", Valid)]; "one branch")]
    #[test_case(&json!({"oneOf": [{"type": "integer"}]}), &json!("a"), &[("/oneOf/0/type", Invalid), ("/oneOf/0", Invalid), ("/oneOf", Invalid), ("", Invalid)]; "one branch rejecting")]
    fn trace(schema: &Value, instance: &Value, expected: &[(&str, NodeEvaluationResult)]) {
        tests_util::assert_trace(schema, instance, expected);
    }

    struct Informational;

    impl Keyword<'_> for Informational {
        fn validate(&self, _: &Value) -> Result<(), ValidationError<'static>> {
            Err(ValidationError::custom("informational"))
        }

        fn is_valid(&self, _: &Value) -> bool {
            false
        }

        fn is_informational(&self) -> bool {
            true
        }
    }

    #[allow(clippy::unnecessary_wraps, clippy::result_large_err)]
    fn informational<'a>(
        _: &'a Map<String, Value>,
        _: &'a Value,
        _: Location,
    ) -> Result<Box<dyn for<'i> Keyword<'i>>, ValidationError<'a>> {
        Ok(Box::new(Informational))
    }

    // An informational keyword decides neither validity nor a trace, under one branch as under
    // several.
    #[test_case(&json!({"oneOf": [{"type": "integer", "note": 1}]}); "one branch")]
    #[test_case(&json!({"oneOf": [{"type": "integer", "note": 1}, {"type": "string"}]}); "two branches")]
    fn informational_keywords_do_not_decide_a_trace(schema: &Value) {
        let validator = crate::options()
            .with_keyword("note", informational)
            .build(schema)
            .expect("builds");
        assert!(validator.is_valid(&json!(1)));
        assert!(validator.trace(&json!(1), &mut |_| {}));
    }
}
