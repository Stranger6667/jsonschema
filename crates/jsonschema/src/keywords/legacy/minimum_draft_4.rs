use crate::{
    compiler,
    error::ValidationError,
    ext::numeric,
    keywords::{minmax, CompilationResult},
    paths::{LazyLocation, Location},
    tracing::{TracingCallback, TracingContext},
    types::JsonType,
    validator::{Validate, ValidationContext},
};
use num_cmp::NumCmp;
use serde_json::{Map, Value};

#[inline]
pub(crate) fn compile<'a>(
    ctx: &compiler::Context,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>> {
    if let Some(Value::Bool(true)) = parent.get("exclusiveMinimum") {
        compile_exclusive(ctx, parent, schema)
    } else {
        minmax::compile_minimum(ctx, parent, schema)
    }
}

pub(crate) struct ExclusiveMinimumU64Validator {
    limit: u64,
    limit_val: Value,
    location: Location,
    minimum_location: Location,
}
pub(crate) struct ExclusiveMinimumI64Validator {
    limit: i64,
    limit_val: Value,
    location: Location,
    minimum_location: Location,
}
pub(crate) struct ExclusiveMinimumF64Validator {
    limit: f64,
    limit_val: Value,
    location: Location,
    minimum_location: Location,
}

macro_rules! validate {
    ($validator: ty) => {
        impl Validate for $validator {
            fn validate<'i>(
                &self,
                instance: &'i Value,
                location: &LazyLocation,
                ctx: &mut ValidationContext,
            ) -> Result<(), ValidationError<'i>> {
                if self.is_valid(instance, ctx) {
                    Ok(())
                } else {
                    Err(ValidationError::exclusive_minimum(
                        self.location.clone(),
                        location.into(),
                        instance,
                        self.limit_val.clone(),
                    ))
                }
            }

            fn is_valid(&self, instance: &Value, _ctx: &mut ValidationContext) -> bool {
                if let Value::Number(item) = instance {
                    numeric::gt(item, self.limit)
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
                ctx: &mut ValidationContext,
            ) -> bool {
                let result = self.is_valid(instance, ctx);
                let rv = if self.matches_type(instance) {
                    Some(result)
                } else {
                    None
                };
                TracingContext::new(location, self.schema_path(), rv).call(callback);
                TracingContext::new(location, &self.minimum_location, rv).call(callback);
                result
            }
        }
    };
}

validate!(ExclusiveMinimumU64Validator);
validate!(ExclusiveMinimumI64Validator);

impl Validate for ExclusiveMinimumF64Validator {
    fn is_valid(&self, instance: &Value, _ctx: &mut ValidationContext) -> bool {
        if let Value::Number(item) = instance {
            return if let Some(item) = item.as_u64() {
                NumCmp::num_gt(item, self.limit)
            } else if let Some(item) = item.as_i64() {
                NumCmp::num_gt(item, self.limit)
            } else {
                let item = item.as_f64().expect("Always valid");
                NumCmp::num_gt(item, self.limit)
            };
        }
        true
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
            Err(ValidationError::exclusive_minimum(
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
        ctx: &mut ValidationContext,
    ) -> bool {
        let result = self.is_valid(instance, ctx);
        let rv = if self.matches_type(instance) {
            Some(result)
        } else {
            None
        };
        TracingContext::new(location, self.schema_path(), rv).call(callback);
        TracingContext::new(location, &self.minimum_location, rv).call(callback);
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
        let location = ctx.location().join("exclusiveMinimum");
        if let Some(limit) = limit.as_u64() {
            Some(Ok(Box::new(ExclusiveMinimumU64Validator {
                limit,
                limit_val: (*schema).clone(),
                location,
                minimum_location: ctx.location().join("minimum"),
            })))
        } else if let Some(limit) = limit.as_i64() {
            Some(Ok(Box::new(ExclusiveMinimumI64Validator {
                limit,
                limit_val: (*schema).clone(),
                location,
                minimum_location: ctx.location().join("minimum"),
            })))
        } else {
            let limit = limit.as_f64().expect("Always valid");
            Some(Ok(Box::new(ExclusiveMinimumF64Validator {
                limit,
                limit_val: (*schema).clone(),
                location,
                minimum_location: ctx.location().join("minimum"),
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
