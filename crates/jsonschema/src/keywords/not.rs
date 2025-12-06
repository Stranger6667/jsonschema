use crate::{
    compiler,
    error::ValidationError,
    keywords::CompilationResult,
    node::SchemaNode,
    paths::{LazyLocation, Location},
    validator::{Validate, ValidationContext},
};
use serde_json::{Map, Value};

pub(crate) struct NotValidator {
    // needed only for error representation
    original: Value,
    node: SchemaNode,
}

impl NotValidator {
    #[inline]
    pub(crate) fn compile<'a>(ctx: &compiler::Context, schema: &'a Value) -> CompilationResult<'a> {
        let ctx = ctx.new_at_location("not");
        Ok(Box::new(NotValidator {
            original: schema.clone(),
            node: compiler::compile(&ctx, ctx.as_resource_ref(schema))?,
        }))
    }
}

impl Validate for NotValidator {
    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        !self.node.is_valid(instance, ctx)
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if self.is_valid(instance, ctx) {
            Ok(())
        } else {
            Err(ValidationError::not(
                self.node.location().clone(),
                location.into(),
                instance,
                self.original.clone(),
            ))
        }
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
        callback: crate::tracing::TracingCallback<'_>,
        ctx: &mut ValidationContext,
    ) -> bool {
        // Trace the inner schema
        let inner_is_valid = self.node.trace(instance, instance_path, callback, ctx);
        crate::tracing::TracingContext::new(instance_path, self.node.location(), inner_is_valid)
            .call(callback);
        // not is valid when inner schema is invalid
        let is_valid = !inner_is_valid;
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
    Some(NotValidator::compile(ctx, schema))
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use serde_json::json;

    #[test]
    fn location() {
        tests_util::assert_schema_location(
            &json!({"not": {"type": "string"}}),
            &json!("foo"),
            "/not",
        );
    }
}
