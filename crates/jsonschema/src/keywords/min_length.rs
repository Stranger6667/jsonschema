#![allow(clippy::float_cmp, clippy::cast_sign_loss)]

use crate::{
    compiler,
    error::ValidationError,
    keywords::{helpers::fail_on_non_positive_integer, CompilationResult},
    paths::{LazyLocation, Location, RefTracker},
    validator::{Validate, ValidationContext},
};
use serde_json::{Map, Value};

pub(crate) struct MinLengthValidator {
    limit: u64,
    location: Location,
}

impl MinLengthValidator {
    #[inline]
    pub(crate) fn compile<'a>(
        ctx: &compiler::Context,
        schema: &'a Value,
        location: Location,
    ) -> CompilationResult<'a> {
        if let Some(limit) = schema.as_u64() {
            return Ok(Box::new(MinLengthValidator { limit, location }));
        }
        if ctx.supports_integer_valued_numbers() {
            if let Some(limit) = schema.as_f64() {
                if limit.trunc() == limit {
                    #[allow(clippy::cast_possible_truncation)]
                    return Ok(Box::new(MinLengthValidator {
                        // NOTE: Imprecise cast as big integers are not supported yet
                        limit: limit as u64,
                        location,
                    }));
                }
            }
        }
        Err(fail_on_non_positive_integer(schema, location))
    }
}

impl Validate for MinLengthValidator {
    fn is_valid(&self, instance: &Value, _ctx: &mut ValidationContext) -> bool {
        if let Value::String(item) = instance {
            if (bytecount::num_chars(item.as_bytes()) as u64) < self.limit {
                return false;
            }
        }
        true
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::String(item) = instance {
            if (bytecount::num_chars(item.as_bytes()) as u64) < self.limit {
                return Err(ValidationError::min_length(
                    self.location.clone(),
                    crate::paths::capture_evaluation_path(tracker, &self.location),
                    location.into(),
                    instance,
                    self.limit,
                ));
            }
        }
        Ok(())
    }
}

#[inline]
pub(crate) fn compile<'a>(
    ctx: &compiler::Context,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>> {
    use super::string_length::{parse_limit, MinMaxLengthValidator};

    let min_location = ctx.location().join("minLength");

    // Check if maxLength is also present - if so, create fused validator
    if let Some(max_schema) = parent.get("maxLength") {
        if let (Some(min), Some(max)) = (parse_limit(ctx, schema), parse_limit(ctx, max_schema)) {
            let max_location = ctx.location().join("maxLength");
            return Some(MinMaxLengthValidator::compile(
                min,
                max,
                min_location,
                max_location,
            ));
        }
    }

    Some(MinLengthValidator::compile(ctx, schema, min_location))
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
