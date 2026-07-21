use std::borrow::Cow;

use crate::{
    compiler,
    error::{ErrorIterator, ValidationError},
    node::SchemaNode,
    paths::{LazyLocation, Location, RefTracker},
    types::JsonType,
    validator::{EvaluationResult, Validate, ValidationContext},
    Json, SerdeJson,
};
use serde_json::{Map, Value};

use super::CompilationResult;

pub(crate) struct AllOfValidator<F: Json> {
    schemas: Vec<SchemaNode<F>>,
}

impl AllOfValidator<SerdeJson> {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        items: &'a [Value],
    ) -> CompilationResult<'a, F> {
        let ctx = ctx.new_at_location("allOf");
        let mut schemas = Vec::with_capacity(items.len());
        for (idx, item) in items.iter().enumerate() {
            let ctx = ctx.new_at_location(idx);
            let validators = compiler::compile(&ctx, ctx.as_resource_ref(item))?;
            schemas.push(validators);
        }
        Ok(Box::new(AllOfValidator { schemas }))
    }
}

impl<F: Json> Validate<F> for AllOfValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        self.schemas.iter().all(|n| n.is_valid(instance, ctx))
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        for schema in &self.schemas {
            schema.validate(instance, location, tracker, ctx)?;
        }
        Ok(())
    }

    #[allow(clippy::needless_collect)]
    fn iter_errors<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        let errors: Vec<_> = self
            .schemas
            .iter()
            .flat_map(move |node| node.iter_errors(instance, location, tracker, ctx))
            .collect();
        ErrorIterator::from_iterator(errors.into_iter())
    }

    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        let mut children = Vec::with_capacity(self.schemas.len());
        for node in &self.schemas {
            children.push(node.evaluate_instance(instance, location, tracker, ctx));
        }
        EvaluationResult::from_children(children)
    }
}

pub(crate) struct SingleValueAllOfValidator<F: Json> {
    node: SchemaNode<F>,
}

impl SingleValueAllOfValidator<SerdeJson> {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        schema: &'a Value,
    ) -> CompilationResult<'a, F> {
        let ctx = ctx.new_at_location("allOf");
        let ctx = ctx.new_at_location(0);
        let node = compiler::compile(&ctx, ctx.as_resource_ref(schema))?;
        Ok(Box::new(SingleValueAllOfValidator { node }))
    }
}

impl<F: Json> Validate<F> for SingleValueAllOfValidator<F> {
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
        self.node.validate(instance, location, tracker, ctx)
    }

    fn iter_errors<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        self.node.iter_errors(instance, location, tracker, ctx)
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

#[inline]
pub(crate) fn compile<'a, F: Json>(
    ctx: &compiler::Context<F>,
    _: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a, F>> {
    if let Value::Array(items) = schema {
        if items.len() == 1 {
            let value = items.iter().next().expect("Vec is not empty");
            Some(SingleValueAllOfValidator::compile(ctx, value))
        } else {
            Some(AllOfValidator::compile(ctx, items))
        }
    } else {
        let location = ctx.location().join("allOf");
        Some(Err(ValidationError::single_type_error(
            location.clone(),
            location,
            Location::new(),
            Cow::Borrowed(schema),
            JsonType::Array,
        )))
    }
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use serde_json::{json, Value};
    use test_case::test_case;

    #[test_case(&json!({"allOf": [{"type": "string"}]}), &json!(1), "/allOf/0/type")]
    #[test_case(&json!({"allOf": [{"type": "integer"}, {"maximum": 5}]}), &json!(6), "/allOf/1/maximum")]
    fn location(schema: &Value, instance: &Value, expected: &str) {
        tests_util::assert_schema_location(schema, instance, expected);
    }
}
