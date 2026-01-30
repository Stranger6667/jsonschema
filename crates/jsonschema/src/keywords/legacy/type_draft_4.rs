use crate::{
    compiler,
    error::ValidationError,
    keywords::{type_, CompilationResult},
    paths::{LazyLocation, Location, RefTracker},
    types::{JsonType, JsonTypeSet},
    validator::{Validate, ValidationContext},
};
use serde_json::{json, Map, Number, Value};
use std::str::FromStr;

pub(crate) struct MultipleTypesValidator {
    types: JsonTypeSet,
    location: Location,
}

impl MultipleTypesValidator {
    #[inline]
    pub(crate) fn compile(items: &[Value], location: Location) -> CompilationResult<'_> {
        let mut types = JsonTypeSet::empty();
        for item in items {
            match item {
                Value::String(string) => {
                    if let Ok(ty) = JsonType::from_str(string.as_str()) {
                        types = types.insert(ty);
                    } else {
                        return Err(ValidationError::enumeration(
                            location.clone(),
                            location,
                            Location::new(),
                            item,
                            &json!([
                                "array", "boolean", "integer", "null", "number", "object", "string"
                            ]),
                        ));
                    }
                }
                _ => {
                    return Err(ValidationError::single_type_error(
                        location.clone(),
                        location,
                        Location::new(),
                        item,
                        JsonType::String,
                    ))
                }
            }
        }
        Ok(Box::new(MultipleTypesValidator { types, location }))
    }
}

impl Validate for MultipleTypesValidator {
    fn is_valid(&self, instance: &Value, _ctx: &mut ValidationContext) -> bool {
        match instance {
            Value::Array(_) => self.types.contains(JsonType::Array),
            Value::Bool(_) => self.types.contains(JsonType::Boolean),
            Value::Null => self.types.contains(JsonType::Null),
            Value::Number(n) => {
                if is_integer(n) {
                    self.types.contains(JsonType::Integer) || self.types.contains(JsonType::Number)
                } else {
                    self.types.contains(JsonType::Number)
                }
            }
            Value::Object(_) => self.types.contains(JsonType::Object),
            Value::String(_) => self.types.contains(JsonType::String),
        }
    }
    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if self.is_valid(instance, ctx) {
            Ok(())
        } else {
            Err(ValidationError::multiple_type_error(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance,
                self.types,
            ))
        }
    }
}

pub(crate) struct IntegerTypeValidator {
    location: Location,
}

impl IntegerTypeValidator {
    #[inline]
    pub(crate) fn compile<'a>(location: Location) -> CompilationResult<'a> {
        Ok(Box::new(IntegerTypeValidator { location }))
    }
}

impl Validate for IntegerTypeValidator {
    fn is_valid(&self, instance: &Value, _ctx: &mut ValidationContext) -> bool {
        if let Value::Number(num) = instance {
            is_integer(num)
        } else {
            false
        }
    }
    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if self.is_valid(instance, ctx) {
            Ok(())
        } else {
            Err(ValidationError::single_type_error(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance,
                JsonType::Integer,
            ))
        }
    }
}

fn is_integer(num: &Number) -> bool {
    if num.is_u64() || num.is_i64() {
        return true;
    }
    // Draft 4 is strict: numbers written with decimal points are NOT integers,
    // regardless of whether the fractional part is zero.
    // See: tests/suite/tests/draft4/optional/zeroTerminatedFloats.json
    #[cfg(feature = "arbitrary-precision")]
    {
        use crate::ext::numeric::bignum;

        let s = num.as_str();
        if s.contains('.') {
            return false;
        }
        // Plain integers or scientific notation without decimal point
        bignum::try_parse_bigint(num).is_some()
    }
    #[cfg(not(feature = "arbitrary-precision"))]
    {
        false
    }
}

#[inline]
pub(crate) fn compile<'a>(
    ctx: &compiler::Context,
    _: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>> {
    let location = ctx.location().join("type");
    match schema {
        Value::String(item) => Some(compile_single_type(item.as_str(), location, schema)),
        Value::Array(items) => {
            if items.len() == 1 {
                let item = &items[0];
                if let Value::String(ty) = item {
                    Some(compile_single_type(ty.as_str(), location, item))
                } else {
                    Some(Err(ValidationError::single_type_error(
                        location.clone(),
                        location,
                        Location::new(),
                        item,
                        JsonType::String,
                    )))
                }
            } else {
                Some(MultipleTypesValidator::compile(items, location))
            }
        }
        _ => {
            let location = ctx.location().join("type");
            Some(Err(ValidationError::multiple_type_error(
                location.clone(),
                location,
                Location::new(),
                schema,
                JsonTypeSet::empty()
                    .insert(JsonType::String)
                    .insert(JsonType::Array),
            )))
        }
    }
}

fn compile_single_type<'a>(
    item: &str,
    location: Location,
    instance: &'a Value,
) -> CompilationResult<'a> {
    match JsonType::from_str(item) {
        Ok(JsonType::Array) => type_::ArrayTypeValidator::compile(location),
        Ok(JsonType::Boolean) => type_::BooleanTypeValidator::compile(location),
        Ok(JsonType::Integer) => IntegerTypeValidator::compile(location),
        Ok(JsonType::Null) => type_::NullTypeValidator::compile(location),
        Ok(JsonType::Number) => type_::NumberTypeValidator::compile(location),
        Ok(JsonType::Object) => type_::ObjectTypeValidator::compile(location),
        Ok(JsonType::String) => type_::StringTypeValidator::compile(location),
        Err(()) => Err(ValidationError::compile_error(
            location.clone(),
            location,
            Location::new(),
            instance,
            "Unexpected type",
        )),
    }
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use serde_json::Value;
    use test_case::test_case;

    fn parse_json(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    // Draft 4 is strict: floats like 1.0 are NOT integers
    #[test_case(r#"{"type": "integer"}"#, "42", true; "plain integer")]
    #[test_case(r#"{"type": "integer"}"#, "-42", true; "negative integer")]
    #[test_case(r#"{"type": "integer"}"#, "0", true; "zero")]
    #[test_case(r#"{"type": "integer"}"#, "1.0", false; "float with .0 is not integer in draft4")]
    #[test_case(r#"{"type": "integer"}"#, "42.0", false; "integer as float is not integer in draft4")]
    #[test_case(r#"{"type": "integer"}"#, "-42.0", false; "negative float with .0 is not integer in draft4")]
    #[test_case(r#"{"type": "integer"}"#, "1.5", false; "decimal")]
    #[test_case(r#"{"type": "integer"}"#, "0.1", false; "small decimal")]
    #[test_case(r#"{"type": "integer"}"#, "42.7", false; "float with decimal")]
    #[test_case(r#"{"type": "integer"}"#, "9223372036854775807", true; "i64::MAX")]
    #[test_case(r#"{"type": "integer"}"#, "-9223372036854775808", true; "i64::MIN")]
    #[test_case(r#"{"type": "integer"}"#, "18446744073709551615", true; "u64::MAX")]
    #[test_case(r#"{"type": "integer"}"#, "true", false; "boolean")]
    #[test_case(r#"{"type": "integer"}"#, r#""42""#, false; "string")]
    #[test_case(r#"{"type": "integer"}"#, "[]", false; "array")]
    #[test_case(r#"{"type": "integer"}"#, "{}", false; "object")]
    #[test_case(r#"{"type": "integer"}"#, "null", false; "null")]
    fn integer_type_validation_draft4(schema_json: &str, instance_json: &str, expected: bool) {
        let schema = parse_json(schema_json);
        let instance = parse_json(instance_json);
        if expected {
            tests_util::is_valid_with_draft4(&schema, &instance);
        } else {
            tests_util::is_not_valid_with_draft4(&schema, &instance);
        }
    }

    #[cfg(feature = "arbitrary-precision")]
    mod arbitrary_precision {
        use crate::tests_util;
        use serde_json::Value;
        use test_case::test_case;

        fn parse_json(s: &str) -> Value {
            serde_json::from_str(s).unwrap()
        }

        // Tests for huge integers beyond i64/u64 range - these must be parsed from JSON string
        // to avoid Python/Rust int conversion issues
        #[test_case(r#"{"type": "integer"}"#, "18446744073709551616", true; "u64_max_plus_1_plain")]
        #[test_case(r#"{"type": "integer"}"#, "-9223372036854775809", true; "i64_min_minus_1")]
        #[test_case(r#"{"type": "integer"}"#, "99999999999999999999", true; "huge_plain_integer")]
        #[test_case(
            r#"{"type": "integer"}"#,
            "999999999999999999999999999999999999999999999999999999999999999999999999999999",
            true;
            "very_huge_plain_integer"
        )]
        #[test_case(r#"{"type": "integer"}"#, "-18446744073709551616", true; "negative_huge_integer")]
        #[test_case(r#"{"type": "integer"}"#, "-99999999999999999999", true; "negative_huge_plain")]
        // Numbers with decimal points are NOT integers in Draft 4, even if fractional part is zero
        #[test_case(r#"{"type": "integer"}"#, "18446744073709551616.0", false; "huge_with_dot_0_not_integer")]
        #[test_case(r#"{"type": "integer"}"#, "99999999999999999999.0", false; "huge_integer_with_dot_0_not_integer")]
        #[test_case(r#"{"type": "integer"}"#, "-18446744073709551616.0", false; "negative_huge_with_dot_0_not_integer")]
        #[test_case(r#"{"type": "integer"}"#, "-99999999999999999999.0", false; "negative_very_huge_with_dot_0_not_integer")]
        #[test_case(r#"{"type": "integer"}"#, "18446744073709551616.5", false; "huge decimal")]
        #[test_case(r#"{"type": "integer"}"#, "99999999999999999999.5", false; "huge float")]
        #[test_case(
            r#"{"type": "integer"}"#,
            "999999999999999999999999999999999999999999999999999999999999999999999999999999.5",
            false;
            "very huge float"
        )]
        #[test_case(r#"{"type": "integer"}"#, "1e1000", true; "huge scientific notation integer")]
        #[test_case(r#"{"type": "integer"}"#, "1e1000001", false; "infinity positive")]
        #[test_case(r#"{"type": "integer"}"#, "-1e1000001", false; "infinity negative")]
        #[test_case(r#"{"type": ["integer", "string"]}"#, "18446744073709551616", true; "huge int in union")]
        #[test_case(r#"{"type": ["integer", "string"]}"#, "-9223372036854775809", true; "huge negative int in union")]
        #[test_case(r#"{"type": ["integer", "string"]}"#, "18446744073709551616.0", false; "huge .0 not integer in union")]
        #[test_case(r#"{"type": ["integer", "string"]}"#, "18446744073709551616.5", false; "huge float not in union")]
        fn huge_number_integer_validation_draft4(
            schema_json: &str,
            instance_json: &str,
            expected: bool,
        ) {
            let schema = parse_json(schema_json);
            let instance = parse_json(instance_json);
            if expected {
                tests_util::is_valid_with_draft4(&schema, &instance);
            } else {
                tests_util::is_not_valid_with_draft4(&schema, &instance);
            }
        }
    }
}
