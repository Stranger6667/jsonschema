use crate::{
    compiler,
    error::{ErrorIterator, ValidationError},
    node::SchemaNode,
    paths::{LazyLocation, Location, RefTracker},
    tracing::{TracingCallback, TracingContext},
    types::JsonType,
    validator::{EvaluationResult, Validate, ValidationContext},
};
use serde_json::{Map, Value};

use super::CompilationResult;

pub(crate) struct AllOfValidator {
    schemas: Vec<SchemaNode>,
    location: Location,
}

impl AllOfValidator {
    #[inline]
    pub(crate) fn compile<'a>(
        ctx: &compiler::Context,
        items: &'a [Value],
    ) -> CompilationResult<'a> {
        let ctx = ctx.new_at_location("allOf");
        let mut schemas = Vec::with_capacity(items.len());
        for (idx, item) in items.iter().enumerate() {
            let ctx = ctx.new_at_location(idx);
            let validators = compiler::compile(&ctx, ctx.as_resource_ref(item))?;
            schemas.push(validators);
        }
        Ok(Box::new(AllOfValidator {
            schemas,
            location: ctx.location().clone(),
        }))
    }
}

impl Validate for AllOfValidator {
    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        self.schemas.iter().all(|n| n.is_valid(instance, ctx))
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
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
        instance: &'i Value,
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
        instance: &Value,
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
        callback: TracingCallback<'_>,
        ctx: &mut ValidationContext,
    ) -> bool {
        let mut is_valid = true;
        for node in &self.schemas {
            let schema_is_valid = node.trace(instance, instance_path, callback, ctx);
            TracingContext::new(instance_path, node.schema_path(), schema_is_valid).call(callback);
            is_valid &= schema_is_valid;
        }
        TracingContext::new(instance_path, self.schema_path(), is_valid).call(callback);
        is_valid
    }
}

pub(crate) struct SingleValueAllOfValidator {
    node: SchemaNode,
}

impl SingleValueAllOfValidator {
    #[inline]
    pub(crate) fn compile<'a>(ctx: &compiler::Context, schema: &'a Value) -> CompilationResult<'a> {
        let ctx = ctx.new_at_location("allOf");
        let ctx = ctx.new_at_location(0);
        let node = compiler::compile(&ctx, ctx.as_resource_ref(schema))?;
        Ok(Box::new(SingleValueAllOfValidator { node }))
    }
}

impl Validate for SingleValueAllOfValidator {
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
        self.node.validate(instance, location, tracker, ctx)
    }

    fn iter_errors<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        self.node.iter_errors(instance, location, tracker, ctx)
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
        self.node.location()
    }
    fn trace(
        &self,
        instance: &Value,
        instance_path: &LazyLocation,
        callback: TracingCallback<'_>,
        ctx: &mut ValidationContext,
    ) -> bool {
        let is_valid = self.node.trace(instance, instance_path, callback, ctx);
        TracingContext::new(instance_path, self.schema_path(), is_valid).call(callback);
        is_valid
    }
}

#[inline]
pub(crate) fn compile<'a>(
    ctx: &compiler::Context,
    _: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>> {
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

    #[test_case(&json!({"allOf": [{"type": "string"}]}), &json!(1), "/allOf/0/type")]
    #[test_case(&json!({"allOf": [{"type": "integer"}, {"maximum": 5}]}), &json!(6), "/allOf/1/maximum")]
    fn location(schema: &Value, instance: &Value, expected: &str) {
        tests_util::assert_schema_location(schema, instance, expected);
    }
}
