use crate::{
    compiler,
    error::ValidationError,
    keywords::{type_, CompilationResult},
    paths::{LazyLocation, Location, RefTracker},
    types::{JsonType, JsonTypeSet},
    validator::{Validate, ValidationContext},
    Json, LazyInstance, Node,
};
use serde_json::{json, Map, Value};
use std::{borrow::Cow, str::FromStr};

pub(crate) struct MultipleTypesValidator {
    types: JsonTypeSet,
    location: Location,
}

impl MultipleTypesValidator {
    #[inline]
    pub(crate) fn compile<F: Json>(
        items: &[Value],
        location: Location,
    ) -> CompilationResult<'_, F> {
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
                            LazyInstance::Ready(Cow::Borrowed(item)),
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
                        LazyInstance::Ready(Cow::Borrowed(item)),
                        JsonType::String,
                    ))
                }
            }
        }
        Ok(Box::new(MultipleTypesValidator { types, location }))
    }
}

impl<F: Json> Validate<F> for MultipleTypesValidator {
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        match instance.json_type() {
            JsonType::Number => {
                let Some(n) = instance.as_number() else {
                    return false;
                };
                if is_integer(&n) {
                    self.types.contains(JsonType::Integer) || self.types.contains(JsonType::Number)
                } else {
                    self.types.contains(JsonType::Number)
                }
            }
            other => self.types.contains(other),
        }
    }
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
            Err(ValidationError::multiple_type_error(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance.lazy_value(),
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
    pub(crate) fn compile<'a, F: Json>(location: Location) -> CompilationResult<'a, F> {
        Ok(Box::new(IntegerTypeValidator { location }))
    }
}

impl<F: Json> Validate<F> for IntegerTypeValidator {
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        if let Some(num) = instance.as_number() {
            is_integer(&num)
        } else {
            false
        }
    }
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
            Err(ValidationError::single_type_error(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance.lazy_value(),
                JsonType::Integer,
            ))
        }
    }
}

// Draft 4: "a JSON number without a fraction or exponent part", so `1.0` and `1e2` are not
// integers here, unlike drafts 6+.
pub(crate) fn is_integer<N: jsonschema_value::JsonNumber>(num: &N) -> bool {
    num.is_written_as_integer()
}

#[inline]
pub(crate) fn compile<'a, F: Json>(
    ctx: &compiler::Context<F>,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a, F>> {
    // Absorbed by the fused array-shape validator emitted from `items`.
    if crate::keywords::items::array_shape_fusion(ctx, parent) {
        return None;
    }
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
                        LazyInstance::Ready(Cow::Borrowed(item)),
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
                LazyInstance::Ready(Cow::Borrowed(schema)),
                JsonTypeSet::from(JsonType::String).insert(JsonType::Array),
            )))
        }
    }
}

fn compile_single_type<'a, F: Json>(
    item: &str,
    location: Location,
    instance: &'a Value,
) -> CompilationResult<'a, F> {
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
            LazyInstance::Ready(Cow::Borrowed(instance)),
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
    #[test_case(r#"{"type": "integer"}"#, "1e2", false; "exponent notation is not an integer in draft4")]
    #[test_case(r#"{"type": "integer"}"#, "1E2", false; "capital exponent notation is not an integer in draft4")]
    #[test_case(r#"{"type": "integer"}"#, "1e0", false; "zero exponent is not an integer in draft4")]
    #[test_case(r#"{"type": "integer"}"#, "-1e2", false; "negative exponent notation is not an integer in draft4")]
    #[test_case(r#"{"type": "integer"}"#, "1e16", false; "exponent past the f64 fixed-notation cutoff is not an integer in draft4")]
    #[test_case(r#"{"type": "integer"}"#, "1.0e2", false; "fraction and exponent together is not an integer in draft4")]
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
        // Only decidable while the literal survives; otherwise these come back in exponent form.
        #[test_case(r#"{"type": "integer"}"#, "1e300", false; "large exponent is not an integer in draft4")]
        #[test_case(r#"{"type": "integer"}"#, "100000000000000000000.0", false; "large float with a fraction part is not an integer in draft4")]
        #[test_case(r#"{"type": "integer"}"#, "1e1000", false; "huge scientific notation is not an integer in draft4")]
        #[test_case(r#"{"type": "integer"}"#, "1e1000001", false; "scientific notation past f64 is not an integer in draft4")]
        #[test_case(r#"{"type": "integer"}"#, "-1e1000001", false; "negative scientific notation past f64 is not an integer in draft4")]
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
