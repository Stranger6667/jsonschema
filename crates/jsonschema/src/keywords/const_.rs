use crate::{
    compiler,
    error::ValidationError,
    keywords::CompilationResult,
    paths::Location,
    validator::{Validate, ValidationContext},
    Json, JsonArrayAccess, JsonNode,
};
use serde_json::{Map, Number, Value};

use crate::paths::{LazyLocation, RefTracker};

struct ConstArrayValidator {
    value: Vec<Value>,
    location: Location,
}
impl ConstArrayValidator {
    #[inline]
    pub(crate) fn compile<F: Json>(
        value: &[Value],
        location: Location,
    ) -> CompilationResult<'_, F> {
        Ok(Box::new(ConstArrayValidator {
            value: value.to_vec(),
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
            Err(ValidationError::constant_array(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance.to_value(),
                &self.value,
            ))
        }
    }

    #[inline]
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        if let Some(items) = instance.as_array() {
            items.len() == self.value.len()
                && items
                    .elements()
                    .zip(&self.value)
                    .all(|(item, expected)| item.equals_value(expected))
        } else {
            false
        }
    }
}

struct ConstBooleanValidator {
    value: bool,
    location: Location,
}
impl ConstBooleanValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        value: bool,
        location: Location,
    ) -> CompilationResult<'a, F> {
        Ok(Box::new(ConstBooleanValidator { value, location }))
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
            Err(ValidationError::constant_boolean(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance.to_value(),
                self.value,
            ))
        }
    }

    #[inline]
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        instance.as_boolean() == Some(self.value)
    }
}

struct ConstNullValidator {
    location: Location,
}
impl ConstNullValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(location: Location) -> CompilationResult<'a, F> {
        Ok(Box::new(ConstNullValidator { location }))
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
            Err(ValidationError::constant_null(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance.to_value(),
            ))
        }
    }
    #[inline]
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        instance.is_null()
    }
}

struct ConstNumberValidator {
    // This is saved in order to ensure that the error message is not altered by precision loss
    original_value: Number,
    location: Location,
}

impl ConstNumberValidator {
    #[inline]
    pub(crate) fn compile<F: Json>(
        original_value: &Number,
        location: Location,
    ) -> CompilationResult<'_, F> {
        Ok(Box::new(ConstNumberValidator {
            original_value: original_value.clone(),
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
            Err(ValidationError::constant_number(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance.to_value(),
                &self.original_value,
            ))
        }
    }

    #[inline]
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        if let Some(item) = instance.as_number() {
            crate::ext::cmp::equal_numbers(&item, &self.original_value)
        } else {
            false
        }
    }
}

pub(crate) struct ConstObjectValidator {
    value: Value,
    location: Location,
}

impl ConstObjectValidator {
    #[inline]
    pub(crate) fn compile<F: Json>(
        value: &Map<String, Value>,
        location: Location,
    ) -> CompilationResult<'_, F> {
        Ok(Box::new(ConstObjectValidator {
            value: Value::Object(value.clone()),
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
            Err(ValidationError::constant_object(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance.to_value(),
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
    value: String,
    location: Location,
}

impl ConstStringValidator {
    #[inline]
    pub(crate) fn compile<F: Json>(value: &str, location: Location) -> CompilationResult<'_, F> {
        Ok(Box::new(ConstStringValidator {
            value: value.to_string(),
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
            Err(ValidationError::constant_string(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance.to_value(),
                &self.value,
            ))
        }
    }

    #[inline]
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        if let Some(item) = instance.as_string() {
            self.value == item.as_ref()
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
        Value::Array(items) => Some(ConstArrayValidator::compile(items, location)),
        Value::Bool(item) => Some(ConstBooleanValidator::compile(*item, location)),
        Value::Null => Some(ConstNullValidator::compile(location)),
        Value::Number(item) => Some(ConstNumberValidator::compile(item, location)),
        Value::Object(map) => Some(ConstObjectValidator::compile(map, location)),
        Value::String(string) => Some(ConstStringValidator::compile(string, location)),
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

    // Tests for arbitrary-precision const validation
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
