use crate::{
    compiler,
    error::ValidationError,
    keywords::{
        helpers::{fail_on_non_positive_integer, size_limit},
        CompilationResult,
    },
    paths::{LazyLocation, Location, RefTracker},
    validator::{Validate, ValidationContext},
    Json, Node, Object,
};
use serde_json::{Map, Value};

pub(crate) struct MinPropertiesValidator {
    limit: u64,
    location: Location,
}

impl MinPropertiesValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        schema: &'a Value,
        location: Location,
    ) -> CompilationResult<'a, F> {
        let Some(limit) = size_limit(ctx, schema) else {
            return Err(fail_on_non_positive_integer(schema, location));
        };
        Ok(Box::new(MinPropertiesValidator { limit, location }))
    }
}

impl<F: Json> Validate<F> for MinPropertiesValidator {
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        if let Some(object) = instance.as_object() {
            if (object.len() as u64) < self.limit {
                return false;
            }
        }
        true
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Some(object) = instance.as_object() {
            if (object.len() as u64) < self.limit {
                return Err(ValidationError::min_properties(
                    self.location.clone(),
                    crate::paths::capture_evaluation_path(tracker, &self.location),
                    location.into(),
                    instance.to_value(),
                    self.limit,
                ));
            }
        }
        Ok(())
    }
}

#[inline]
pub(crate) fn compile<'a, F: Json>(
    ctx: &compiler::Context<F>,
    _: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a, F>> {
    let location = ctx.location().join("minProperties");
    Some(MinPropertiesValidator::compile(ctx, schema, location))
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use serde_json::json;

    #[test]
    fn location() {
        tests_util::assert_schema_location(
            &json!({"minProperties": 2}),
            &json!({"a": 1}),
            "/minProperties",
        );
    }
}
