#![allow(clippy::float_cmp, clippy::cast_sign_loss)]

use crate::{
    compiler,
    error::ValidationError,
    keywords::{helpers::fail_on_non_positive_integer, CompilationResult},
    paths::{LazyLocation, Location},
    validator::Validate,
};
use referencing::Uri;
use serde_json::{Map, Value};
use std::sync::Arc;

pub(crate) struct MaxItemsValidator {
    limit: u64,
    location: Location,
    absolute_path: Option<Arc<Uri<String>>>,
}

impl MaxItemsValidator {
    #[inline]
    pub(crate) fn compile<'a>(
        ctx: &compiler::Context,
        schema: &'a Value,
        location: Location,
    ) -> CompilationResult<'a> {
        let absolute_path = ctx.base_uri();
        if let Some(limit) = schema.as_u64() {
            return Ok(Box::new(MaxItemsValidator {
                limit,
                location,
                absolute_path,
            }));
        }
        if ctx.supports_integer_valued_numbers() {
            if let Some(limit) = schema.as_f64() {
                if limit.trunc() == limit {
                    #[allow(clippy::cast_possible_truncation)]
                    return Ok(Box::new(MaxItemsValidator {
                        // NOTE: Imprecise cast as big integers are not supported yet
                        limit: limit as u64,
                        location,
                        absolute_path,
                    }));
                }
            }
        }
        Err(fail_on_non_positive_integer(
            schema,
            location,
            ctx.base_uri(),
        ))
    }
}

impl Validate for MaxItemsValidator {
    fn is_valid(&self, instance: &Value) -> bool {
        if let Value::Array(items) = instance {
            if (items.len() as u64) > self.limit {
                return false;
            }
        }
        true
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Array(items) = instance {
            if (items.len() as u64) > self.limit {
                return Err(ValidationError::max_items(
                    self.location.clone(),
                    location.into(),
                    instance,
                    self.limit,
                    self.absolute_path.clone(),
                ));
            }
        }
        Ok(())
    }
}

#[inline]
pub(crate) fn compile<'a>(
    ctx: &compiler::Context,
    _: &Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>> {
    let location = ctx.location().join("maxItems");
    Some(MaxItemsValidator::compile(ctx, schema, location))
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use serde_json::json;

    #[test]
    fn location() {
        tests_util::assert_schema_location(&json!({"maxItems": 1}), &json!([1, 2]), "/maxItems");
    }
}
