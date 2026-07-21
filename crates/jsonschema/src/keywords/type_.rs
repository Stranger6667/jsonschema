use crate::{
    compiler,
    error::ValidationError,
    evaluation::ErrorDescription,
    keywords::CompilationResult,
    paths::Location,
    types::{JsonType, JsonTypeSet},
    validator::{EvaluationResult, Validate, ValidationContext},
    Json, JsonNode,
};
use serde_json::{json, Map, Number, Value};
use std::{borrow::Cow, str::FromStr};

use crate::paths::{LazyLocation, RefTracker};

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
                            Cow::Borrowed(item),
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
                        Cow::Borrowed(item),
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
        self.types.contains_value_type::<F>(instance)
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
                instance.to_value(),
                self.types,
            ))
        }
    }
    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        _location: &LazyLocation,
        _tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if Validate::<F>::is_valid(self, instance, ctx) {
            EvaluationResult::valid_empty()
        } else {
            let message = format!("{} is not of types {:?}", instance.to_value(), self.types);
            EvaluationResult::invalid_empty(vec![ErrorDescription::new("type", message)])
        }
    }
}

pub(crate) struct NullTypeValidator {
    location: Location,
}

impl NullTypeValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(location: Location) -> CompilationResult<'a, F> {
        Ok(Box::new(NullTypeValidator { location }))
    }
}

impl<F: Json> Validate<F> for NullTypeValidator {
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        instance.is_null()
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
                instance.to_value(),
                JsonType::Null,
            ))
        }
    }
    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        _location: &LazyLocation,
        _tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if Validate::<F>::is_valid(self, instance, ctx) {
            EvaluationResult::valid_empty()
        } else {
            EvaluationResult::invalid_empty(vec![ErrorDescription::new(
                "type",
                format!(r#"{} is not of type "null""#, instance.to_value()),
            )])
        }
    }
}

pub(crate) struct BooleanTypeValidator {
    location: Location,
}

impl BooleanTypeValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(location: Location) -> CompilationResult<'a, F> {
        Ok(Box::new(BooleanTypeValidator { location }))
    }
}

impl<F: Json> Validate<F> for BooleanTypeValidator {
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        instance.as_boolean().is_some()
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
                instance.to_value(),
                JsonType::Boolean,
            ))
        }
    }
    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        _location: &LazyLocation,
        _tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if Validate::<F>::is_valid(self, instance, ctx) {
            EvaluationResult::valid_empty()
        } else {
            EvaluationResult::invalid_empty(vec![ErrorDescription::new(
                "type",
                format!(r#"{} is not of type "boolean""#, instance.to_value()),
            )])
        }
    }
}

pub(crate) struct StringTypeValidator {
    location: Location,
}

impl StringTypeValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(location: Location) -> CompilationResult<'a, F> {
        Ok(Box::new(StringTypeValidator { location }))
    }
}

impl<F: Json> Validate<F> for StringTypeValidator {
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        instance.is_string()
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
                instance.to_value(),
                JsonType::String,
            ))
        }
    }
    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        _location: &LazyLocation,
        _tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if Validate::<F>::is_valid(self, instance, ctx) {
            EvaluationResult::valid_empty()
        } else {
            EvaluationResult::invalid_empty(vec![ErrorDescription::new(
                "type",
                format!(r#"{} is not of type "string""#, instance.to_value()),
            )])
        }
    }
}

pub(crate) struct ArrayTypeValidator {
    location: Location,
}

impl ArrayTypeValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(location: Location) -> CompilationResult<'a, F> {
        Ok(Box::new(ArrayTypeValidator { location }))
    }
}

impl<F: Json> Validate<F> for ArrayTypeValidator {
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        instance.as_array().is_some()
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
                instance.to_value(),
                JsonType::Array,
            ))
        }
    }
    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        _location: &LazyLocation,
        _tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if Validate::<F>::is_valid(self, instance, ctx) {
            EvaluationResult::valid_empty()
        } else {
            EvaluationResult::invalid_empty(vec![ErrorDescription::new(
                "type",
                format!(r#"{} is not of type "array""#, instance.to_value()),
            )])
        }
    }
}

pub(crate) struct ObjectTypeValidator {
    location: Location,
}

impl ObjectTypeValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(location: Location) -> CompilationResult<'a, F> {
        Ok(Box::new(ObjectTypeValidator { location }))
    }
}

impl<F: Json> Validate<F> for ObjectTypeValidator {
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        instance.as_object().is_some()
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
                instance.to_value(),
                JsonType::Object,
            ))
        }
    }
    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        _location: &LazyLocation,
        _tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if Validate::<F>::is_valid(self, instance, ctx) {
            EvaluationResult::valid_empty()
        } else {
            EvaluationResult::invalid_empty(vec![ErrorDescription::new(
                "type",
                format!(r#"{} is not of type "object""#, instance.to_value()),
            )])
        }
    }
}

pub(crate) struct NumberTypeValidator {
    location: Location,
}

impl NumberTypeValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(location: Location) -> CompilationResult<'a, F> {
        Ok(Box::new(NumberTypeValidator { location }))
    }
}

impl<F: Json> Validate<F> for NumberTypeValidator {
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        instance.is_number()
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
                instance.to_value(),
                JsonType::Number,
            ))
        }
    }
    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        _location: &LazyLocation,
        _tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if Validate::<F>::is_valid(self, instance, ctx) {
            EvaluationResult::valid_empty()
        } else {
            EvaluationResult::invalid_empty(vec![ErrorDescription::new(
                "type",
                format!(r#"{} is not of type "number""#, instance.to_value()),
            )])
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
                instance.to_value(),
                JsonType::Integer,
            ))
        }
    }
    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        _location: &LazyLocation,
        _tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if Validate::<F>::is_valid(self, instance, ctx) {
            EvaluationResult::valid_empty()
        } else {
            EvaluationResult::invalid_empty(vec![ErrorDescription::new(
                "type",
                format!(r#"{} is not of type "integer""#, instance.to_value()),
            )])
        }
    }
}

pub(crate) fn is_integer(num: &Number) -> bool {
    if num.is_u64() || num.is_i64() {
        return true;
    }
    #[cfg(feature = "arbitrary-precision")]
    {
        use crate::ext::numeric::bignum;
        use num_traits::One;

        // Check huge plain integers first (no decimal point)
        if bignum::try_parse_bigint(num).is_some() {
            return true;
        }

        // Check huge decimals - must do this BEFORE as_f64() to avoid precision loss
        if let Some(bigfrac) = bignum::try_parse_bigfraction(num) {
            return bigfrac.denom().is_none_or(One::is_one);
        }

        // For numbers that fit in f64 range
        if let Some(f) = num.as_f64() {
            return f.fract() == 0.;
        }

        // Numbers that overflow to infinity (as_f64() returns None) are not integers
        false
    }
    #[cfg(not(feature = "arbitrary-precision"))]
    {
        if let Some(f) = num.as_f64() {
            return f.fract() == 0.;
        }
        unreachable!("Numbers always fit in u64/i64/f64 without arbitrary-precision")
    }
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
                        Cow::Borrowed(item),
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
                Cow::Borrowed(schema),
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
        Ok(JsonType::Array) => ArrayTypeValidator::compile(location),
        Ok(JsonType::Boolean) => BooleanTypeValidator::compile(location),
        Ok(JsonType::Integer) => IntegerTypeValidator::compile(location),
        Ok(JsonType::Null) => NullTypeValidator::compile(location),
        Ok(JsonType::Number) => NumberTypeValidator::compile(location),
        Ok(JsonType::Object) => ObjectTypeValidator::compile(location),
        Ok(JsonType::String) => StringTypeValidator::compile(location),
        Err(()) => Err(ValidationError::compile_error(
            location.clone(),
            location,
            Location::new(),
            Cow::Borrowed(instance),
            "Unexpected type",
        )),
    }
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use serde_json::{json, Value};
    use test_case::test_case;

    #[test_case(&json!({"type": "array"}), &json!(1), "/type")]
    #[test_case(&json!({"type": "boolean"}), &json!(1), "/type")]
    #[test_case(&json!({"type": "integer"}), &json!("f"), "/type")]
    #[test_case(&json!({"type": "null"}), &json!(1), "/type")]
    #[test_case(&json!({"type": "number"}), &json!("f"), "/type")]
    #[test_case(&json!({"type": "object"}), &json!(1), "/type")]
    #[test_case(&json!({"type": "string"}), &json!(1), "/type")]
    #[test_case(&json!({"type": ["string", "object"]}), &json!(1), "/type")]
    fn location(schema: &Value, instance: &Value, expected: &str) {
        tests_util::assert_schema_location(schema, instance, expected);
    }

    fn parse_json(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    #[test_case(r#"{"type": "integer"}"#, "42", true; "plain integer")]
    #[test_case(r#"{"type": "integer"}"#, "-42", true; "negative integer")]
    #[test_case(r#"{"type": "integer"}"#, "0", true; "zero")]
    #[test_case(r#"{"type": "integer"}"#, "1.0", true; "float with .0")]
    #[test_case(r#"{"type": "integer"}"#, "42.0", true; "integer as float")]
    #[test_case(r#"{"type": "integer"}"#, "-42.0", true; "negative float with .0")]
    #[test_case(r#"{"type": "integer"}"#, "1.5", false; "decimal")]
    #[test_case(r#"{"type": "integer"}"#, "0.1", false; "small decimal")]
    #[test_case(r#"{"type": "integer"}"#, "42.7", false; "float with decimal")]
    #[test_case(r#"{"type": "integer"}"#, "9223372036854775807", true; "i64::MAX")]
    #[test_case(r#"{"type": "integer"}"#, "-9223372036854775808", true; "i64::MIN")]
    #[test_case(r#"{"type": "integer"}"#, "18446744073709551615", true; "u64::MAX")]
    #[test_case(r#"{"type": "integer"}"#, "1e10", true; "scientific int 1e10")]
    #[test_case(r#"{"type": "integer"}"#, "1.0e10", true; "scientific 1.0e10")]
    #[test_case(r#"{"type": "integer"}"#, "1.5e10", true; "scientific 1.5e10 equals 15000000000")]
    #[test_case(r#"{"type": "integer"}"#, "1e-10", false; "scientific small 1e-10")]
    #[test_case(r#"{"type": "integer"}"#, "true", false; "boolean")]
    #[test_case(r#"{"type": "integer"}"#, r#""42""#, false; "string")]
    #[test_case(r#"{"type": "integer"}"#, "[]", false; "array")]
    #[test_case(r#"{"type": "integer"}"#, "{}", false; "object")]
    #[test_case(r#"{"type": "integer"}"#, "null", false; "null")]
    fn integer_type_validation(schema_json: &str, instance_json: &str, expected: bool) {
        let schema = parse_json(schema_json);
        let instance = parse_json(instance_json);
        if expected {
            tests_util::is_valid(&schema, &instance);
        } else {
            tests_util::is_not_valid(&schema, &instance);
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

        #[test_case(r#"{"type": "integer"}"#, "18446744073709551616", true; "u64_max_plus_1_plain")]
        #[test_case(r#"{"type": "integer"}"#, "18446744073709551616.0", true; "u64_max_plus_1_with_dot_0")]
        #[test_case(r#"{"type": "integer"}"#, "99999999999999999999", true; "huge_plain_integer")]
        #[test_case(r#"{"type": "integer"}"#, "99999999999999999999.0", true; "huge_integer_with_dot_0")]
        #[test_case(
            r#"{"type": "integer"}"#,
            "999999999999999999999999999999999999999999999999999999999999999999999999999999",
            true;
            "very_huge_plain_integer"
        )]
        #[test_case(
            r#"{"type": "integer"}"#,
            "999999999999999999999999999999999999999999999999999999999999999999999999999999.0",
            true;
            "very_huge_integer_with_dot_0"
        )]
        #[test_case(r#"{"type": "integer"}"#, "-18446744073709551616", true; "negative_huge_integer")]
        #[test_case(r#"{"type": "integer"}"#, "-18446744073709551616.0", true; "negative_huge_integer_with_dot_0")]
        #[test_case(r#"{"type": "integer"}"#, "-99999999999999999999", true; "negative_huge_plain")]
        #[test_case(r#"{"type": "integer"}"#, "-99999999999999999999.0", true; "negative_huge_plain_with_dot_0")]
        #[test_case(r#"{"type": "integer"}"#, "18446744073709551616.5", false; "huge decimal")]
        #[test_case(r#"{"type": "integer"}"#, "99999999999999999999.5", false; "huge float")]
        #[test_case(
            r#"{"type": "integer"}"#,
            "999999999999999999999999999999999999999999999999999999999999999999999999999999.5",
            false;
            "very huge float"
        )]
        #[test_case(r#"{"type": "integer"}"#, "0.3", false; "bigfrac 0.3")]
        #[test_case(r#"{"type": "integer"}"#, "0.1", false; "bigfrac 0.1")]
        #[test_case(r#"{"type": "integer"}"#, "123.456", false; "bigfrac 123.456")]
        #[test_case(r#"{"type": "integer"}"#, "1e1000", true; "huge scientific notation integer")]
        #[test_case(r#"{"type": "integer"}"#, "1e1000001", false; "infinity positive")]
        #[test_case(r#"{"type": "integer"}"#, "-1e1000001", false; "infinity negative")]
        #[test_case(r#"{"type": "integer"}"#, "1.5e-1000", false; "huge scientific float")]
        #[test_case(r#"{"type": "number"}"#, "18446744073709551616", true; "huge int valid as number")]
        #[test_case(r#"{"type": "number"}"#, "18446744073709551616.0", true; "huge .0 valid as number")]
        #[test_case(r#"{"type": "number"}"#, "18446744073709551616.5", true; "huge float valid as number")]
        #[test_case(r#"{"type": "number"}"#, "1e10000", true; "infinity valid as number")]
        #[test_case(r#"{"type": ["integer", "string"]}"#, "18446744073709551616", true; "huge int in union")]
        #[test_case(r#"{"type": ["integer", "string"]}"#, "18446744073709551616.0", true; "huge .0 in union")]
        #[test_case(r#"{"type": ["integer", "string"]}"#, "18446744073709551616.5", false; "huge float not in union")]
        #[test_case(r#"{"type": ["integer", "string"]}"#, r#""foo""#, true; "string in union")]
        #[test_case(r#"{"type": ["number", "string"]}"#, "18446744073709551616.5", true; "huge float with number")]
        #[test_case(r#"{"type": "integer"}"#, "true", false; "boolean not integer")]
        #[test_case(r#"{"type": "integer"}"#, r#""42""#, false; "string not integer")]
        #[test_case(r#"{"type": "integer"}"#, "[]", false; "array not integer")]
        #[test_case(r#"{"type": "integer"}"#, "{}", false; "object not integer")]
        #[test_case(r#"{"type": "integer"}"#, "null", false; "null not integer")]
        fn huge_number_integer_validation(schema_json: &str, instance_json: &str, expected: bool) {
            let schema = parse_json(schema_json);
            let instance = parse_json(instance_json);
            if expected {
                tests_util::is_valid(&schema, &instance);
            } else {
                tests_util::is_not_valid(&schema, &instance);
            }
        }
    }
}
