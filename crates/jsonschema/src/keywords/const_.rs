use crate::{
    compiler,
    error::ValidationError,
    keywords::CompilationResult,
    paths::Location,
    validator::{Validate, ValidationContext},
    Array, Json, Node,
};
use serde_json::{Map, Number, Value};
use std::sync::Arc;

use crate::paths::{LazyLocation, RefTracker};

struct ConstArrayValidator {
    value: Arc<Value>,
    location: Location,
}
impl ConstArrayValidator {
    #[inline]
    pub(crate) fn compile<F: Json>(value: &Value, location: Location) -> CompilationResult<'_, F> {
        Ok(Box::new(ConstArrayValidator {
            value: Arc::new(value.clone()),
            location,
        }))
    }
}
impl<F: Json> Validate<F> for ConstArrayValidator {
    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if Validate::<F>::is_valid(self, instance, ctx) {
            Ok(())
        } else {
            Err(ValidationError::constant(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance.lazy_value(),
                &self.value,
            ))
        }
    }

    #[inline]
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        if let (Some(items), Some(expected)) = (instance.as_array(), self.value.as_array()) {
            items.len() == expected.len()
                && items
                    .elements()
                    .zip(expected)
                    .all(|(item, expected)| item.equals_value(expected))
        } else {
            false
        }
    }
}

struct ConstBooleanValidator {
    value: Arc<Value>,
    location: Location,
}
impl ConstBooleanValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        value: bool,
        location: Location,
    ) -> CompilationResult<'a, F> {
        Ok(Box::new(ConstBooleanValidator {
            value: Arc::new(Value::Bool(value)),
            location,
        }))
    }
}
impl<F: Json> Validate<F> for ConstBooleanValidator {
    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if Validate::<F>::is_valid(self, instance, ctx) {
            Ok(())
        } else {
            Err(ValidationError::constant(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance.lazy_value(),
                &self.value,
            ))
        }
    }

    #[inline]
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        instance.as_boolean() == self.value.as_bool()
    }
}

struct ConstNullValidator {
    value: Arc<Value>,
    location: Location,
}
impl ConstNullValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(location: Location) -> CompilationResult<'a, F> {
        Ok(Box::new(ConstNullValidator {
            value: Arc::new(Value::Null),
            location,
        }))
    }
}
impl<F: Json> Validate<F> for ConstNullValidator {
    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if Validate::<F>::is_valid(self, instance, ctx) {
            Ok(())
        } else {
            Err(ValidationError::constant(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance.lazy_value(),
                &self.value,
            ))
        }
    }
    #[inline]
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        instance.is_null()
    }
}

struct ConstNumberValidator {
    // The original `Number` keeps the error message free of precision loss
    value: Arc<Value>,
    location: Location,
}

impl ConstNumberValidator {
    #[inline]
    pub(crate) fn compile<F: Json>(value: &Number, location: Location) -> CompilationResult<'_, F> {
        Ok(Box::new(ConstNumberValidator {
            value: Arc::new(Value::Number(value.clone())),
            location,
        }))
    }
}

impl<F: Json> Validate<F> for ConstNumberValidator {
    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if Validate::<F>::is_valid(self, instance, ctx) {
            Ok(())
        } else {
            Err(ValidationError::constant(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance.lazy_value(),
                &self.value,
            ))
        }
    }

    #[inline]
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        if let (Some(item), Some(expected)) = (instance.as_number(), self.value.as_number()) {
            crate::cmp::equal_numbers(&item, expected)
        } else {
            false
        }
    }
}

pub(crate) struct ConstObjectValidator {
    value: Arc<Value>,
    location: Location,
}

impl ConstObjectValidator {
    #[inline]
    pub(crate) fn compile<F: Json>(value: &Value, location: Location) -> CompilationResult<'_, F> {
        Ok(Box::new(ConstObjectValidator {
            value: Arc::new(value.clone()),
            location,
        }))
    }
}

impl<F: Json> Validate<F> for ConstObjectValidator {
    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if Validate::<F>::is_valid(self, instance, ctx) {
            Ok(())
        } else {
            Err(ValidationError::constant(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance.lazy_value(),
                &self.value,
            ))
        }
    }

    #[inline]
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        instance.equals_value(&self.value)
    }
}

pub(crate) struct ConstStringValidator {
    value: Arc<Value>,
    location: Location,
}

impl ConstStringValidator {
    #[inline]
    pub(crate) fn compile<F: Json>(value: &Value, location: Location) -> CompilationResult<'_, F> {
        Ok(Box::new(ConstStringValidator {
            value: Arc::new(value.clone()),
            location,
        }))
    }
}

impl<F: Json> Validate<F> for ConstStringValidator {
    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if Validate::<F>::is_valid(self, instance, ctx) {
            Ok(())
        } else {
            Err(ValidationError::constant(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance.lazy_value(),
                &self.value,
            ))
        }
    }

    #[inline]
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        if let Some(item) = instance.as_string() {
            self.value.as_str() == Some(item.as_ref())
        } else {
            false
        }
    }
}

#[inline]
pub(crate) fn compile<'a, F: Json>(
    ctx: &compiler::Context<F>,
    _: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a, F>> {
    let location = ctx.location().join("const");
    match schema {
        Value::Array(_) => Some(ConstArrayValidator::compile(schema, location)),
        Value::Bool(item) => Some(ConstBooleanValidator::compile(*item, location)),
        Value::Null => Some(ConstNullValidator::compile(location)),
        Value::Number(item) => Some(ConstNumberValidator::compile(item, location)),
        Value::Object(_) => Some(ConstObjectValidator::compile(schema, location)),
        Value::String(_) => Some(ConstStringValidator::compile(schema, location)),
    }
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use serde_json::{json, Value};
    use test_case::test_case;

    #[test_case(&json!({"const": 1}), &json!(2), "/const")]
    #[test_case(&json!({"const": null}), &json!(3), "/const")]
    #[test_case(&json!({"const": false}), &json!(4), "/const")]
    #[test_case(&json!({"const": []}), &json!(5), "/const")]
    #[test_case(&json!({"const": {}}), &json!(6), "/const")]
    #[test_case(&json!({"const": ""}), &json!(7), "/const")]
    fn location(schema: &Value, instance: &Value, expected: &str) {
        tests_util::assert_schema_location(schema, instance, expected);
    }

    #[cfg(feature = "arbitrary-precision")]
    mod arbitrary_precision {
        use crate::tests_util;
        use serde_json::Value;
        use test_case::test_case;

        fn parse_json(json: &str) -> Value {
            serde_json::from_str(json).unwrap()
        }

        #[test_case(r#"{"const": 18446744073709551617}"#, "18446744073709551617", true; "large int exact match")]
        #[test_case(r#"{"const": 18446744073709551617}"#, "18446744073709551616", false; "large int different by one")]
        #[test_case(r#"{"const": 18446744073709551617}"#, "18446744073709551618", false; "large int different by one above")]
        #[test_case(r#"{"const": -9223372036854775809}"#, "-9223372036854775809", true; "large negative int match")]
        #[test_case(r#"{"const": -9223372036854775809}"#, "-9223372036854775808", false; "large negative int different")]
        #[test_case(r#"{"const": 0.1}"#, "0.1", true; "decimal exact match")]
        #[test_case(r#"{"const": 0.1}"#, "0.10000000000000001", false; "decimal precision difference")]
        fn const_arbitrary_precision(schema_json: &str, instance_json: &str, expected_valid: bool) {
            let schema = parse_json(schema_json);
            let instance = parse_json(instance_json);
            if expected_valid {
                tests_util::is_valid(&schema, &instance);
            } else {
                tests_util::is_not_valid(&schema, &instance);
            }
        }
    }
}
