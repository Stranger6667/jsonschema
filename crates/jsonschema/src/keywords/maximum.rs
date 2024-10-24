use crate::{
    compiler,
    error::ValidationError,
    keywords::CompilationResult,
    paths::{LazyLocation, Location},
    primitive_type::PrimitiveType,
    validator::Validate,
};
use num_cmp::NumCmp;
use serde_json::{Map, Value};

pub(crate) struct MaximumU64Validator {
    limit: u64,
    limit_val: Value,
    location: Location,
}
pub(crate) struct MaximumI64Validator {
    limit: i64,
    limit_val: Value,
    location: Location,
}
pub(crate) struct MaximumF64Validator {
    limit: f64,
    limit_val: Value,
    location: Location,
}

macro_rules! validate {
    ($validator: ty) => {
        impl Validate for $validator {
            fn validate<'i>(
                &self,
                instance: &'i Value,
                location: &LazyLocation,
            ) -> Result<(), ValidationError<'i>> {
                if self.is_valid(instance) {
                    Ok(())
                } else {
                    Err(ValidationError::maximum(
                        self.location.clone(),
                        location.into(),
                        instance,
                        self.limit_val.clone(),
                    )) // do not cast
                }
            }

            fn is_valid(&self, instance: &Value) -> bool {
                if let Value::Number(item) = instance {
                    return if let Some(item) = item.as_u64() {
                        !NumCmp::num_gt(item, self.limit)
                    } else if let Some(item) = item.as_i64() {
                        !NumCmp::num_gt(item, self.limit)
                    } else {
                        let item = item.as_f64().expect("Always valid");
                        !NumCmp::num_gt(item, self.limit)
                    };
                }
                true
            }
        }
    };
}

validate!(MaximumU64Validator);
validate!(MaximumI64Validator);

impl Validate for MaximumF64Validator {
    fn is_valid(&self, instance: &Value) -> bool {
        if let Value::Number(item) = instance {
            return if let Some(item) = item.as_u64() {
                !NumCmp::num_gt(item, self.limit)
            } else if let Some(item) = item.as_i64() {
                !NumCmp::num_gt(item, self.limit)
            } else {
                let item = item.as_f64().expect("Always valid");
                !NumCmp::num_gt(item, self.limit)
            };
        }
        true
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> Result<(), ValidationError<'i>> {
        if self.is_valid(instance) {
            Ok(())
        } else {
            Err(ValidationError::maximum(
                self.location.clone(),
                location.into(),
                instance,
                self.limit_val.clone(),
            ))
        }
    }
}

#[inline]
pub(crate) fn compile<'a>(
    ctx: &compiler::Context,
    _: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>> {
    if let Value::Number(limit) = schema {
        let location = ctx.location().join("maximum");
        if let Some(limit) = limit.as_u64() {
            Some(Ok(Box::new(MaximumU64Validator {
                limit,
                limit_val: schema.clone(),
                location,
            })))
        } else if let Some(limit) = limit.as_i64() {
            Some(Ok(Box::new(MaximumI64Validator {
                limit,
                limit_val: schema.clone(),
                location,
            })))
        } else {
            let limit = limit.as_f64().expect("Always valid");
            Some(Ok(Box::new(MaximumF64Validator {
                limit,
                limit_val: schema.clone(),
                location,
            })))
        }
    } else {
        Some(Err(ValidationError::single_type_error(
            Location::new(),
            ctx.location().clone(),
            schema,
            PrimitiveType::Number,
        )))
    }
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use serde_json::{json, Value};
    use test_case::test_case;

    #[test_case(&json!({"maximum": 1_u64 << 54}), &json!((1_u64 << 54) + 1))]
    #[test_case(&json!({"maximum": 1_i64 << 54}), &json!((1_i64 << 54) + 1))]
    fn is_not_valid(schema: &Value, instance: &Value) {
        tests_util::is_not_valid(schema, instance)
    }

    #[test_case(&json!({"maximum": 5}), &json!(10), "/maximum")]
    #[test_case(&json!({"maximum": 6}), &json!(10), "/maximum")]
    #[test_case(&json!({"maximum": 7}), &json!(10), "/maximum")]
    fn location(schema: &Value, instance: &Value, expected: &str) {
        tests_util::assert_schema_location(schema, instance, expected)
    }
}
