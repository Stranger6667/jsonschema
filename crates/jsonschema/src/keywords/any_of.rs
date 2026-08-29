use crate::LazyInstance;
use std::borrow::Cow;

use crate::{
    compiler,
    error::ValidationError,
    node::SchemaNode,
    paths::{LazyLocation, Location, RefTracker},
    types::JsonType,
    validator::{EvaluationResult, Validate, ValidationContext},
    Json, Node, SerdeJson,
};
use serde_json::{Map, Value};

use super::CompilationResult;

pub(crate) struct AnyOfValidator<F: Json> {
    schemas: Vec<SchemaNode<F>>,
    location: Location,
}

impl AnyOfValidator<SerdeJson> {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        schema: &'a Value,
    ) -> CompilationResult<'a, F> {
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
                LazyInstance::Ready(Cow::Borrowed(schema)),
                JsonType::Array,
            ))
        }
    }
}

impl<F: Json> Validate<F> for AnyOfValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        self.schemas.iter().any(|s| s.is_valid(instance, ctx))
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
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

    fn collect_errors<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
        errors: &mut Vec<ValidationError<'i>>,
    ) {
        if self.is_valid(instance, ctx) {
            return;
        }
        let mut branches = Vec::with_capacity(self.schemas.len());
        for schema in &self.schemas {
            let mut branch = Vec::new();
            schema.collect_errors(instance, location, tracker, ctx, &mut branch);
            branches.push(branch);
        }
        errors.push(ValidationError::any_of(
            self.location.clone(),
            crate::paths::capture_evaluation_path(tracker, &self.location),
            location.into(),
            instance.lazy_value(),
            branches,
        ));
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
        // Per spec §10.2.1.2, annotations must be collected from ALL valid branches.
        // First detect all valid branches cheaply, then evaluate only those branches to avoid
        // constructing dropped error trees for invalid branches in the common case.
        let valid_indices: Vec<_> = self
            .schemas
            .iter()
            .enumerate()
            .filter_map(|(idx, node)| node.is_valid(instance, ctx).then_some(idx))
            .collect();

        if valid_indices.is_empty() {
            // No valid schemas - evaluate all for error output.
            let failures: Vec<_> = self
                .schemas
                .iter()
                .map(|node| {
                    node.evaluate_instance_at(instance, location, instance_location, tracker, ctx)
                })
                .collect();
            EvaluationResult::from_children(failures)
        } else {
            let valid_results: Vec<_> = valid_indices
                .into_iter()
                .map(|idx| {
                    self.schemas[idx].evaluate_instance_at(
                        instance,
                        location,
                        instance_location,
                        tracker,
                        ctx,
                    )
                })
                .collect();
            EvaluationResult::from_children(valid_results)
        }
    }
}

/// Optimized validator for `anyOf` with a single subschema.
pub(crate) struct SingleAnyOfValidator<F: Json> {
    node: SchemaNode<F>,
    location: Location,
}

impl SingleAnyOfValidator<SerdeJson> {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        schema: &'a Value,
    ) -> CompilationResult<'a, F> {
        let any_of_ctx = ctx.new_at_location("anyOf");
        let item_ctx = any_of_ctx.new_at_location(0);
        let node = compiler::compile(&item_ctx, item_ctx.as_resource_ref(schema))?;
        Ok(Box::new(SingleAnyOfValidator {
            node,
            location: any_of_ctx.location().clone(),
        }))
    }
}

impl<F: Json> Validate<F> for SingleAnyOfValidator<F> {
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
            Err(ValidationError::any_of(
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

    fn collect_errors<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
        errors: &mut Vec<ValidationError<'i>>,
    ) {
        if self.node.is_valid(instance, ctx) {
            return;
        }
        let mut branch = Vec::new();
        self.node
            .collect_errors(instance, location, tracker, ctx, &mut branch);
        errors.push(ValidationError::any_of(
            self.location.clone(),
            crate::paths::capture_evaluation_path(tracker, &self.location),
            location.into(),
            instance.lazy_value(),
            vec![branch],
        ));
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
}

#[inline]
pub(crate) fn compile<'a, F: Json>(
    ctx: &compiler::Context<F>,
    _: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a, F>> {
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
            LazyInstance::Ready(Cow::Borrowed(schema)),
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
