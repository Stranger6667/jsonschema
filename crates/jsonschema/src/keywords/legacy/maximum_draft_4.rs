use crate::{
    compiler,
    error::ValidationError,
    ext::numeric,
    keywords::{minmax, CompilationResult},
    paths::{LazyLocation, Location},
    types::JsonType,
    validator::Validate,
    TracingCallback, TracingContext,
};
use serde_json::{Map, Value};

#[inline]
pub(crate) fn compile<'a>(
    ctx: &compiler::Context,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>> {
    if let Some(Value::Bool(true)) = parent.get("exclusiveMaximum") {
        compile_exclusive(ctx, parent, schema)
    } else {
        minmax::compile_maximum(ctx, parent, schema)
    }
}

pub(crate) struct ExclusiveMaximumU64Validator {
    limit: u64,
    limit_val: Value,
    location: Location,
    maximum_location: Location,
}
pub(crate) struct ExclusiveMaximumI64Validator {
    limit: i64,
    limit_val: Value,
    location: Location,
    maximum_location: Location,
}
pub(crate) struct ExclusiveMaximumF64Validator {
    limit: f64,
    limit_val: Value,
    location: Location,
    maximum_location: Location,
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
                    Err(ValidationError::exclusive_maximum(
                        self.location.clone(),
                        location.into(),
                        instance,
                        self.limit_val.clone(),
                    ))
                }
            }

            fn is_valid(&self, instance: &Value) -> bool {
                if let Value::Number(item) = instance {
                    numeric::lt(item, self.limit)
                } else {
                    true
                }
            }
            fn matches_type(&self, instance: &Value) -> bool {
                matches!(instance, Value::Number(_))
            }
            fn schema_path(&self) -> &Location {
                &self.location
            }
            fn trace(
                &self,
                instance: &Value,
                location: &LazyLocation,
                callback: TracingCallback<'_>,
            ) -> bool {
                let result = self.is_valid(instance);
                let rv = if self.matches_type(instance) {
                    Some(result)
                } else {
                    None
                };
                TracingContext::new(location, self.schema_path(), rv).call(callback);
                TracingContext::new(location, &self.maximum_location, rv).call(callback);
                result
            }
        }
    };
}

validate!(ExclusiveMaximumU64Validator);
validate!(ExclusiveMaximumI64Validator);

impl Validate for ExclusiveMaximumF64Validator {
    fn is_valid(&self, instance: &Value) -> bool {
        if let Value::Number(item) = instance {
            numeric::lt(item, self.limit)
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> Result<(), ValidationError<'i>> {
        if self.is_valid(instance) {
            Ok(())
        } else {
            Err(ValidationError::exclusive_maximum(
                self.location.clone(),
                location.into(),
                instance,
                self.limit_val.clone(),
            ))
        }
    }
    fn matches_type(&self, instance: &Value) -> bool {
        matches!(instance, Value::Number(_))
    }
    fn schema_path(&self) -> &Location {
        &self.location
    }
    fn trace(
        &self,
        instance: &Value,
        location: &LazyLocation,
        callback: TracingCallback<'_>,
    ) -> bool {
        let result = self.is_valid(instance);
        let rv = if self.matches_type(instance) {
            Some(result)
        } else {
            None
        };
        TracingContext::new(location, self.schema_path(), rv).call(callback);
        TracingContext::new(location, &self.maximum_location, rv).call(callback);
        result
    }
}

#[inline]
pub(crate) fn compile_exclusive<'a>(
    ctx: &compiler::Context,
    _: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>> {
    if let Value::Number(limit) = schema {
        let location = ctx.location().join("exclusiveMaximum");
        if let Some(limit) = limit.as_u64() {
            Some(Ok(Box::new(ExclusiveMaximumU64Validator {
                limit,
                limit_val: (*schema).clone(),
                location,
                maximum_location: ctx.location().join("maximum"),
            })))
        } else if let Some(limit) = limit.as_i64() {
            Some(Ok(Box::new(ExclusiveMaximumI64Validator {
                limit,
                limit_val: (*schema).clone(),
                location,
                maximum_location: ctx.location().join("maximum"),
            })))
        } else {
            let limit = limit.as_f64().expect("Always valid");
            Some(Ok(Box::new(ExclusiveMaximumF64Validator {
                limit,
                limit_val: (*schema).clone(),
                location,
                maximum_location: ctx.location().join("maximum"),
            })))
        }
    } else {
        Some(Err(ValidationError::single_type_error(
            Location::new(),
            ctx.location().clone(),
            schema,
            JsonType::Number,
        )))
    }
}
