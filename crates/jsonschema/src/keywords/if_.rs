use crate::{
    compiler,
    error::{no_error, ErrorIterator},
    keywords::CompilationResult,
    node::SchemaNode,
    paths::{LazyLocation, Location},
    validator::{EvaluationResult, Validate, ValidationContext},
    ValidationError,
};
use serde_json::{Map, Value};

pub(crate) struct IfThenValidator {
    schema: SchemaNode,
    then_schema: SchemaNode,
}

impl IfThenValidator {
    #[inline]
    pub(crate) fn compile<'a>(
        ctx: &compiler::Context,
        schema: &'a Value,
        then_schema: &'a Value,
    ) -> CompilationResult<'a> {
        Ok(Box::new(IfThenValidator {
            schema: {
                let ctx = ctx.new_at_location("if");
                compiler::compile(&ctx, ctx.as_resource_ref(schema))?
            },
            then_schema: {
                let ctx = ctx.new_at_location("then");
                compiler::compile(&ctx, ctx.as_resource_ref(then_schema))?
            },
        }))
    }
}

impl Validate for IfThenValidator {
    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        if self.schema.is_valid(instance, ctx) {
            self.then_schema.is_valid(instance, ctx)
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if self.schema.is_valid(instance, ctx) {
            self.then_schema.validate(instance, location, ctx)
        } else {
            Ok(())
        }
    }

    #[allow(clippy::needless_collect)]
    fn iter_errors<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if self.schema.is_valid(instance, ctx) {
            let errors: Vec<_> = self
                .then_schema
                .iter_errors(instance, location, ctx)
                .collect();
            ErrorIterator::from_iterator(errors.into_iter())
        } else {
            no_error()
        }
    }

    fn evaluate(
        &self,
        instance: &Value,
        location: &LazyLocation,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        let if_node = self.schema.evaluate_instance(instance, location, ctx);
        if if_node.valid {
            let then_node = self.then_schema.evaluate_instance(instance, location, ctx);
            EvaluationResult::from_children(vec![if_node, then_node])
        } else {
            EvaluationResult::valid_empty()
        }
    }
    fn schema_path(&self) -> &Location {
        self.schema.location()
    }

    fn matches_type(&self, _: &Value) -> bool {
        true
    }

    fn trace(
        &self,
        instance: &Value,
        instance_path: &LazyLocation,
        callback: crate::tracing::TracingCallback<'_>,
        ctx: &mut ValidationContext,
    ) -> bool {
        // Always trace the if condition
        let if_valid = self.schema.trace(instance, instance_path, callback, ctx);
        crate::tracing::TracingContext::new(instance_path, self.schema.location(), if_valid)
            .call(callback);
        if if_valid {
            // If condition passed, trace then branch
            let then_valid = self
                .then_schema
                .trace(instance, instance_path, callback, ctx);
            crate::tracing::TracingContext::new(
                instance_path,
                self.then_schema.location(),
                then_valid,
            )
            .call(callback);
            then_valid
        } else {
            // If condition failed, then doesn't apply - overall valid
            true
        }
    }
}

pub(crate) struct IfElseValidator {
    schema: SchemaNode,
    else_schema: SchemaNode,
}

impl IfElseValidator {
    #[inline]
    pub(crate) fn compile<'a>(
        ctx: &compiler::Context,
        schema: &'a Value,
        else_schema: &'a Value,
    ) -> CompilationResult<'a> {
        Ok(Box::new(IfElseValidator {
            schema: {
                let ctx = ctx.new_at_location("if");
                compiler::compile(&ctx, ctx.as_resource_ref(schema))?
            },
            else_schema: {
                let ctx = ctx.new_at_location("else");
                compiler::compile(&ctx, ctx.as_resource_ref(else_schema))?
            },
        }))
    }
}

impl Validate for IfElseValidator {
    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        if self.schema.is_valid(instance, ctx) {
            true
        } else {
            self.else_schema.is_valid(instance, ctx)
        }
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if self.schema.is_valid(instance, ctx) {
            Ok(())
        } else {
            self.else_schema.validate(instance, location, ctx)
        }
    }

    #[allow(clippy::needless_collect)]
    fn iter_errors<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if self.schema.is_valid(instance, ctx) {
            no_error()
        } else {
            let errors: Vec<_> = self
                .else_schema
                .iter_errors(instance, location, ctx)
                .collect();
            ErrorIterator::from_iterator(errors.into_iter())
        }
    }

    fn evaluate(
        &self,
        instance: &Value,
        location: &LazyLocation,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        let if_node = self.schema.evaluate_instance(instance, location, ctx);
        if if_node.valid {
            EvaluationResult::from_children(vec![if_node])
        } else {
            let else_node = self.else_schema.evaluate_instance(instance, location, ctx);
            EvaluationResult::from_children(vec![else_node])
        }
    }
    fn schema_path(&self) -> &Location {
        self.schema.location()
    }

    fn matches_type(&self, _: &Value) -> bool {
        true
    }

    fn trace(
        &self,
        instance: &Value,
        instance_path: &LazyLocation,
        callback: crate::tracing::TracingCallback<'_>,
        ctx: &mut ValidationContext,
    ) -> bool {
        // Always trace the if condition
        let if_valid = self.schema.trace(instance, instance_path, callback, ctx);
        crate::tracing::TracingContext::new(instance_path, self.schema.location(), if_valid)
            .call(callback);
        if if_valid {
            // If condition passed, else doesn't apply - overall valid
            true
        } else {
            // If condition failed, trace else branch
            let else_valid = self
                .else_schema
                .trace(instance, instance_path, callback, ctx);
            crate::tracing::TracingContext::new(
                instance_path,
                self.else_schema.location(),
                else_valid,
            )
            .call(callback);
            else_valid
        }
    }
}

pub(crate) struct IfThenElseValidator {
    schema: SchemaNode,
    then_schema: SchemaNode,
    else_schema: SchemaNode,
}

impl IfThenElseValidator {
    #[inline]
    pub(crate) fn compile<'a>(
        ctx: &compiler::Context,
        schema: &'a Value,
        then_schema: &'a Value,
        else_schema: &'a Value,
    ) -> CompilationResult<'a> {
        Ok(Box::new(IfThenElseValidator {
            schema: {
                let ctx = ctx.new_at_location("if");
                compiler::compile(&ctx, ctx.as_resource_ref(schema))?
            },
            then_schema: {
                let ctx = ctx.new_at_location("then");
                compiler::compile(&ctx, ctx.as_resource_ref(then_schema))?
            },
            else_schema: {
                let ctx = ctx.new_at_location("else");
                compiler::compile(&ctx, ctx.as_resource_ref(else_schema))?
            },
        }))
    }
}

impl Validate for IfThenElseValidator {
    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        if self.schema.is_valid(instance, ctx) {
            self.then_schema.is_valid(instance, ctx)
        } else {
            self.else_schema.is_valid(instance, ctx)
        }
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if self.schema.is_valid(instance, ctx) {
            self.then_schema.validate(instance, location, ctx)
        } else {
            self.else_schema.validate(instance, location, ctx)
        }
    }

    #[allow(clippy::needless_collect)]
    fn iter_errors<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if self.schema.is_valid(instance, ctx) {
            let errors: Vec<_> = self
                .then_schema
                .iter_errors(instance, location, ctx)
                .collect();
            ErrorIterator::from_iterator(errors.into_iter())
        } else {
            let errors: Vec<_> = self
                .else_schema
                .iter_errors(instance, location, ctx)
                .collect();
            ErrorIterator::from_iterator(errors.into_iter())
        }
    }

    fn evaluate(
        &self,
        instance: &Value,
        location: &LazyLocation,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        let if_node = self.schema.evaluate_instance(instance, location, ctx);
        if if_node.valid {
            let then_node = self.then_schema.evaluate_instance(instance, location, ctx);
            EvaluationResult::from_children(vec![if_node, then_node])
        } else {
            let else_node = self.else_schema.evaluate_instance(instance, location, ctx);
            EvaluationResult::from_children(vec![else_node])
        }
    }
    fn schema_path(&self) -> &Location {
        self.schema.location()
    }

    fn matches_type(&self, _: &Value) -> bool {
        true
    }

    fn trace(
        &self,
        instance: &Value,
        instance_path: &LazyLocation,
        callback: crate::tracing::TracingCallback<'_>,
        ctx: &mut ValidationContext,
    ) -> bool {
        // Always trace the if condition
        let if_valid = self.schema.trace(instance, instance_path, callback, ctx);
        crate::tracing::TracingContext::new(instance_path, self.schema.location(), if_valid)
            .call(callback);
        if if_valid {
            // If condition passed, trace then branch
            let then_valid = self
                .then_schema
                .trace(instance, instance_path, callback, ctx);
            crate::tracing::TracingContext::new(
                instance_path,
                self.then_schema.location(),
                then_valid,
            )
            .call(callback);
            then_valid
        } else {
            // If condition failed, trace else branch
            let else_valid = self
                .else_schema
                .trace(instance, instance_path, callback, ctx);
            crate::tracing::TracingContext::new(
                instance_path,
                self.else_schema.location(),
                else_valid,
            )
            .call(callback);
            else_valid
        }
    }
}

#[inline]
pub(crate) fn compile<'a>(
    ctx: &compiler::Context,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>> {
    let then = parent.get("then");
    let else_ = parent.get("else");
    match (then, else_) {
        (Some(then_schema), Some(else_schema)) => Some(IfThenElseValidator::compile(
            ctx,
            schema,
            then_schema,
            else_schema,
        )),
        (None, Some(else_schema)) => Some(IfElseValidator::compile(ctx, schema, else_schema)),
        (Some(then_schema), None) => Some(IfThenValidator::compile(ctx, schema, then_schema)),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use serde_json::{json, Value};
    use test_case::test_case;

    #[test_case(&json!({"if": {"minimum": 0}, "else": {"multipleOf": 2}}), &json!(-1), "/else/multipleOf")]
    #[test_case(&json!({"if": {"minimum": 0}, "then": {"multipleOf": 2}}), &json!(3), "/then/multipleOf")]
    #[test_case(&json!({"if": {"minimum": 0}, "then": {"multipleOf": 2}, "else": {"multipleOf": 2}}), &json!(-1), "/else/multipleOf")]
    #[test_case(&json!({"if": {"minimum": 0}, "then": {"multipleOf": 2}, "else": {"multipleOf": 2}}), &json!(3), "/then/multipleOf")]
    fn location(schema: &Value, instance: &Value, expected: &str) {
        tests_util::assert_schema_location(schema, instance, expected);
    }
}
