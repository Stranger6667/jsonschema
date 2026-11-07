use crate::{evaluation::InstancePosition, LazyInstance};
use std::borrow::Cow;

use crate::{
    compiler,
    error::ValidationError,
    evaluation::{ChildList, ErrorDescription},
    keywords::{
        discriminator::{Candidates, Discriminator, Dispatch, NoDispatch},
        CompilationResult,
    },
    node::SchemaNode,
    paths::{LazyLocation, Location, RefTracker},
    types::JsonType,
    validator::{EvaluationResult, Validate, ValidationContext},
    Json, Node, SerdeJson,
};
use serde_json::{Map, Value};

pub(crate) struct OneOfValidator<F: Json, D = NoDispatch> {
    schemas: Vec<SchemaNode<F>>,
    dispatch: D,
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
            let location = ctx.location().clone();
            Ok(match Discriminator::compile(&ctx, items) {
                Some(dispatch) => Box::new(OneOfValidator {
                    schemas,
                    dispatch,
                    location,
                }),
                None => Box::new(OneOfValidator {
                    schemas,
                    dispatch: NoDispatch,
                    location,
                }),
            })
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

impl<F: Json, D: Dispatch<F>> OneOfValidator<F, D> {
    #[inline]
    fn candidates(&self, instance: &F::Node<'_>) -> Candidates<'_> {
        self.dispatch.candidates(instance)
    }

    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn get_first_valid(
        &self,
        instance: &F::Node<'_>,
        candidates: Candidates<'_>,
        ctx: &mut ValidationContext,
    ) -> Option<usize> {
        if let Candidates::Only(branches) = candidates {
            return branches
                .iter()
                .copied()
                .find(|&idx| self.schemas[idx].is_valid(instance, ctx));
        }
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
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn are_others_valid(
        &self,
        instance: &F::Node<'_>,
        candidates: Candidates<'_>,
        idx: usize,
        ctx: &mut ValidationContext,
    ) -> bool {
        if let Candidates::Only(branches) = candidates {
            return branches
                .iter()
                .filter(|&&other| other > idx)
                .any(|&other| self.schemas[other].is_valid(instance, ctx));
        }
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
        self.evaluate_with_location(
            instance,
            location,
            ctx.instance_position(location),
            tracker,
            ctx,
        )
    }

    fn evaluate_with_location(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        instance_location: InstancePosition,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        let node =
            self.node
                .evaluate_instance_at(instance, location, instance_location, tracker, ctx);
        EvaluationResult::from_node(ctx.arena(), node)
    }
}

impl<F: Json, D: Dispatch<F>> Validate<F> for OneOfValidator<F, D> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        let candidates = self.candidates(instance);
        let first_valid_idx = self.get_first_valid(instance, candidates, ctx);
        first_valid_idx.is_some_and(|idx| !self.are_others_valid(instance, candidates, idx, ctx))
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        let candidates = self.candidates(instance);
        let first_valid_idx = self.get_first_valid(instance, candidates, ctx);
        if let Some(idx) = first_valid_idx {
            if self.are_others_valid(instance, candidates, idx, ctx) {
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
        self.evaluate_with_location(
            instance,
            location,
            ctx.instance_position(location),
            tracker,
            ctx,
        )
    }

    fn evaluate_with_location(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        instance_location: InstancePosition,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        // Use cheap `is_valid` first, then run full `evaluate` only on matching schemas.
        let candidates = self.candidates(instance);
        let first_valid_idx = self.get_first_valid(instance, candidates, ctx);

        let Some(first_idx) = first_valid_idx else {
            let mut failures = ChildList::default();
            for node in &self.schemas {
                let child =
                    node.evaluate_instance_at(instance, location, instance_location, tracker, ctx);
                failures.push(ctx.arena(), child);
            }
            return EvaluationResult::Invalid {
                errors: Vec::new(),
                children: failures,
                annotations: None,
            };
        };

        if self.are_others_valid(instance, candidates, first_idx, ctx) {
            let mut successes = ChildList::default();
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
                        successes.push(ctx.arena(), child);
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
            EvaluationResult::from_node(ctx.arena(), child)
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

    #[test]
    fn malformed() {
        tests_util::assert_compile_error(
            &json!({"oneOf": 5}),
            "5 is not of type \"array\"",
            "/oneOf",
        );
    }
}
