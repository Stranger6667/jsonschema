use crate::{
    compiler, error::ValidationError, ext::cmp, keywords::CompilationResult, paths::Location,
    validator::Validate,
};
use serde_json::{Map, Number, Value};
use std::sync::Arc;

use crate::paths::LazyLocation;

struct ConstArrayValidator {
    value: Vec<Value>,
    location: Location,
    absolute_path: Option<Arc<referencing::Uri<String>>>,
}
impl ConstArrayValidator {
    #[inline]
    pub(crate) fn compile(
        value: &[Value],
        location: Location,
        absolute_path: Option<Arc<referencing::Uri<String>>>,
    ) -> CompilationResult<'_> {
        Ok(Box::new(ConstArrayValidator {
            value: value.to_vec(),
            location,
            absolute_path,
        }))
    }
}
impl Validate for ConstArrayValidator {
    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> Result<(), ValidationError<'i>> {
        if self.is_valid(instance) {
            Ok(())
        } else {
            Err(ValidationError::constant_array(
                self.location.clone(),
                location.into(),
                instance,
                &self.value,
                self.absolute_path.clone(),
            ))
        }
    }

    #[inline]
    fn is_valid(&self, instance: &Value) -> bool {
        if let Value::Array(instance_value) = instance {
            cmp::equal_arrays(&self.value, instance_value)
        } else {
            false
        }
    }
}

struct ConstBooleanValidator {
    value: bool,
    location: Location,
    absolute_path: Option<Arc<referencing::Uri<String>>>,
}
impl ConstBooleanValidator {
    #[inline]
    pub(crate) fn compile<'a>(
        value: bool,
        location: Location,
        absolute_path: Option<Arc<referencing::Uri<String>>>,
    ) -> CompilationResult<'a> {
        Ok(Box::new(ConstBooleanValidator {
            value,
            location,
            absolute_path,
        }))
    }
}
impl Validate for ConstBooleanValidator {
    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> Result<(), ValidationError<'i>> {
        if self.is_valid(instance) {
            Ok(())
        } else {
            Err(ValidationError::constant_boolean(
                self.location.clone(),
                location.into(),
                instance,
                self.value,
                self.absolute_path.clone(),
            ))
        }
    }

    #[inline]
    fn is_valid(&self, instance: &Value) -> bool {
        if let Value::Bool(instance_value) = instance {
            &self.value == instance_value
        } else {
            false
        }
    }
}

struct ConstNullValidator {
    location: Location,
    absolute_path: Option<Arc<referencing::Uri<String>>>,
}
impl ConstNullValidator {
    #[inline]
    pub(crate) fn compile<'a>(
        location: Location,
        absolute_path: Option<Arc<referencing::Uri<String>>>,
    ) -> CompilationResult<'a> {
        Ok(Box::new(ConstNullValidator {
            location,
            absolute_path,
        }))
    }
}
impl Validate for ConstNullValidator {
    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> Result<(), ValidationError<'i>> {
        if self.is_valid(instance) {
            Ok(())
        } else {
            Err(ValidationError::constant_null(
                self.location.clone(),
                location.into(),
                instance,
                self.absolute_path.clone(),
            ))
        }
    }
    #[inline]
    fn is_valid(&self, instance: &Value) -> bool {
        instance.is_null()
    }
}

struct ConstNumberValidator {
    // This is saved in order to ensure that the error message is not altered by precision loss
    original_value: Number,
    location: Location,
    absolute_path: Option<Arc<referencing::Uri<String>>>,
}

impl ConstNumberValidator {
    #[inline]
    pub(crate) fn compile(
        original_value: &Number,
        location: Location,
        absolute_path: Option<Arc<referencing::Uri<String>>>,
    ) -> CompilationResult<'_> {
        Ok(Box::new(ConstNumberValidator {
            original_value: original_value.clone(),
            location,
            absolute_path,
        }))
    }
}

impl Validate for ConstNumberValidator {
    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> Result<(), ValidationError<'i>> {
        if self.is_valid(instance) {
            Ok(())
        } else {
            Err(ValidationError::constant_number(
                self.location.clone(),
                location.into(),
                instance,
                &self.original_value,
                self.absolute_path.clone(),
            ))
        }
    }

    fn is_valid(&self, instance: &Value) -> bool {
        if let Value::Number(item) = instance {
            crate::ext::cmp::equal_numbers(item, &self.original_value)
        } else {
            false
        }
    }
}

pub(crate) struct ConstObjectValidator {
    value: Map<String, Value>,
    location: Location,
    absolute_path: Option<Arc<referencing::Uri<String>>>,
}

impl ConstObjectValidator {
    #[inline]
    pub(crate) fn compile(
        value: &Map<String, Value>,
        location: Location,
        absolute_path: Option<Arc<referencing::Uri<String>>>,
    ) -> CompilationResult<'_> {
        Ok(Box::new(ConstObjectValidator {
            value: value.clone(),
            location,
            absolute_path,
        }))
    }
}

impl Validate for ConstObjectValidator {
    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> Result<(), ValidationError<'i>> {
        if self.is_valid(instance) {
            Ok(())
        } else {
            Err(ValidationError::constant_object(
                self.location.clone(),
                location.into(),
                instance,
                &self.value,
                self.absolute_path.clone(),
            ))
        }
    }
    fn is_valid(&self, instance: &Value) -> bool {
        if let Value::Object(item) = instance {
            cmp::equal_objects(&self.value, item)
        } else {
            false
        }
    }
}

pub(crate) struct ConstStringValidator {
    value: String,
    location: Location,
    absolute_path: Option<Arc<referencing::Uri<String>>>,
}

impl ConstStringValidator {
    #[inline]
    pub(crate) fn compile(
        value: &str,
        location: Location,
        absolute_path: Option<Arc<referencing::Uri<String>>>,
    ) -> CompilationResult<'_> {
        Ok(Box::new(ConstStringValidator {
            value: value.to_string(),
            location,
            absolute_path,
        }))
    }
}

impl Validate for ConstStringValidator {
    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> Result<(), ValidationError<'i>> {
        if self.is_valid(instance) {
            Ok(())
        } else {
            Err(ValidationError::constant_string(
                self.location.clone(),
                location.into(),
                instance,
                &self.value,
                self.absolute_path.clone(),
            ))
        }
    }
    fn is_valid(&self, instance: &Value) -> bool {
        if let Value::String(item) = instance {
            &self.value == item
        } else {
            false
        }
    }
}

#[inline]
pub(crate) fn compile<'a>(
    ctx: &compiler::Context,
    _: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>> {
    let kctx = ctx.new_at_location("const");
    let location = kctx.location().clone();
    let absolute_path = kctx.base_uri();
    match schema {
        Value::Array(items) => Some(ConstArrayValidator::compile(items, location, absolute_path)),
        Value::Bool(item) => Some(ConstBooleanValidator::compile(
            *item,
            location,
            absolute_path,
        )),
        Value::Null => Some(ConstNullValidator::compile(location, absolute_path)),
        Value::Number(item) => Some(ConstNumberValidator::compile(item, location, absolute_path)),
        Value::Object(map) => Some(ConstObjectValidator::compile(map, location, absolute_path)),
        Value::String(string) => Some(ConstStringValidator::compile(
            string,
            location,
            absolute_path,
        )),
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
