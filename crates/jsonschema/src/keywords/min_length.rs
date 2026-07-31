use crate::{
    compiler,
    error::ValidationError,
    keywords::{
        helpers::{fail_on_non_positive_integer, size_limit},
        CompilationResult,
    },
    paths::{LazyLocation, Location, RefTracker},
    validator::{Validate, ValidationContext},
    Json, Node,
};
use serde_json::{Map, Value};

pub(crate) struct MinLengthValidator {
    limit: u64,
    location: Location,
}

impl MinLengthValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        schema: &'a Value,
        location: Location,
    ) -> CompilationResult<'a, F> {
        let Some(limit) = size_limit(ctx, schema) else {
            return Err(fail_on_non_positive_integer(schema, location));
        };
        Ok(Box::new(MinLengthValidator { limit, location }))
    }
}

impl<F: Json> Validate<F> for MinLengthValidator {
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        if let Some(length) = instance.string_length() {
            if length < self.limit {
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
        if let Some(length) = instance.string_length() {
            if length < self.limit {
                return Err(ValidationError::min_length(
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
    let location = ctx.location().join("minLength");
    Some(MinLengthValidator::compile(ctx, schema, location))
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use serde_json::json;

    #[test]
    fn location() {
        tests_util::assert_schema_location(&json!({"minLength": 1}), &json!(""), "/minLength");
    }
}
