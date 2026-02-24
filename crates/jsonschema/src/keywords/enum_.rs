use crate::{
    compiler,
    error::ValidationError,
    ext::cmp,
    keywords::CompilationResult,
    paths::{LazyLocation, Location, RefTracker},
    types::{JsonType, JsonTypeSet},
    validator::{Validate, ValidationContext},
};
use ahash::AHashSet;
use serde_json::{Map, Value};

const STRING_ENUM_THRESHOLD: usize = 10;

#[derive(Debug)]
pub(crate) struct EnumValidator {
    options: Value,
    // Types that occur in items
    types: JsonTypeSet,
    items: Vec<Value>,
    location: Location,
}

impl EnumValidator {
    #[inline]
    pub(crate) fn compile<'a>(
        schema: &'a Value,
        items: &'a [Value],
        location: Location,
    ) -> CompilationResult<'a> {
        let mut types = JsonTypeSet::empty();
        for item in items {
            types = types.insert(JsonType::from(item));
        }
        Ok(Box::new(EnumValidator {
            options: schema.clone(),
            items: items.to_vec(),
            types,
            location,
        }))
    }
}

impl Validate for EnumValidator {
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
            Err(ValidationError::enumeration(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance,
                &self.options,
            ))
        }
    }

    fn is_valid(&self, instance: &Value, _ctx: &mut ValidationContext) -> bool {
        // If the input value type is not in the types present among the enum options, then there
        // is no reason to compare it against all items - we know that
        // there are no items with such type at all
        if self.types.contains_value_type(instance) {
            self.items.iter().any(|item| cmp::equal(instance, item))
        } else {
            false
        }
    }
}

#[derive(Debug)]
pub(crate) struct SingleValueEnumValidator {
    value: Value,
    options: Value,
    location: Location,
}

impl SingleValueEnumValidator {
    #[inline]
    pub(crate) fn compile<'a>(
        schema: &'a Value,
        value: &'a Value,
        location: Location,
    ) -> CompilationResult<'a> {
        Ok(Box::new(SingleValueEnumValidator {
            options: schema.clone(),
            value: value.clone(),
            location,
        }))
    }
}

impl Validate for SingleValueEnumValidator {
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
            Err(ValidationError::enumeration(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance,
                &self.options,
            ))
        }
    }

    fn is_valid(&self, instance: &Value, _ctx: &mut ValidationContext) -> bool {
        cmp::equal(&self.value, instance)
    }
}

#[derive(Debug)]
pub(crate) struct SmallStringEnumValidator {
    options: Value,
    items: Vec<Box<str>>,
    location: Location,
}

impl SmallStringEnumValidator {
    #[inline]
    pub(crate) fn compile<'a>(
        schema: &'a Value,
        items: &'a [Value],
        location: Location,
    ) -> CompilationResult<'a> {
        let strings = items
            .iter()
            .map(|v| v.as_str().expect("all items are strings").into())
            .collect();
        Ok(Box::new(SmallStringEnumValidator {
            options: schema.clone(),
            items: strings,
            location,
        }))
    }
}

impl Validate for SmallStringEnumValidator {
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
            Err(ValidationError::enumeration(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance,
                &self.options,
            ))
        }
    }

    fn is_valid(&self, instance: &Value, _ctx: &mut ValidationContext) -> bool {
        if let Value::String(s) = instance {
            self.items.iter().any(|item| item.as_ref() == s.as_str())
        } else {
            false
        }
    }
}

#[derive(Debug)]
pub(crate) struct BigStringEnumValidator {
    options: Value,
    items: AHashSet<Box<str>>,
    location: Location,
}

impl BigStringEnumValidator {
    #[inline]
    pub(crate) fn compile<'a>(
        schema: &'a Value,
        items: &'a [Value],
        location: Location,
    ) -> CompilationResult<'a> {
        let strings = items
            .iter()
            .map(|v| v.as_str().expect("all items are strings").into())
            .collect();
        Ok(Box::new(BigStringEnumValidator {
            options: schema.clone(),
            items: strings,
            location,
        }))
    }
}

impl Validate for BigStringEnumValidator {
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
            Err(ValidationError::enumeration(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance,
                &self.options,
            ))
        }
    }

    fn is_valid(&self, instance: &Value, _ctx: &mut ValidationContext) -> bool {
        if let Value::String(s) = instance {
            self.items.contains(s.as_str())
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
    if let Value::Array(items) = schema {
        let location = ctx.location().join("enum");
        if items.len() == 1 {
            let value = items.iter().next().expect("Vec is not empty");
            Some(SingleValueEnumValidator::compile(schema, value, location))
        } else if items.iter().all(|v| matches!(v, Value::String(_))) {
            if items.len() <= STRING_ENUM_THRESHOLD {
                Some(SmallStringEnumValidator::compile(schema, items, location))
            } else {
                Some(BigStringEnumValidator::compile(schema, items, location))
            }
        } else {
            Some(EnumValidator::compile(schema, items, location))
        }
    } else {
        let location = ctx.location().join("enum");
        Some(Err(ValidationError::single_type_error(
            location.clone(),
            location,
            Location::new(),
            schema,
            JsonType::Array,
        )))
    }
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use serde_json::{json, Value};
    use test_case::test_case;

    #[test_case(&json!({"enum": [1]}), &json!(2), "/enum")]
    #[test_case(&json!({"enum": [1, 3]}), &json!(2), "/enum")]
    fn location(schema: &Value, instance: &Value, expected: &str) {
        tests_util::assert_schema_location(schema, instance, expected);
    }

    // 10 entries — exercises BigStringEnumValidator
    const BIG_STRING_ENUM: &str = r#"{
        "enum": ["a","b","c","d","e","f","g","h","i","j","k"]
    }"#;

    #[test]
    fn big_string_enum_valid() {
        let schema: Value = serde_json::from_str(BIG_STRING_ENUM).unwrap();
        for s in &["a", "e", "j"] {
            tests_util::is_valid(&schema, &json!(s));
        }
    }

    #[test]
    fn big_string_enum_invalid_string() {
        let schema: Value = serde_json::from_str(BIG_STRING_ENUM).unwrap();
        tests_util::is_not_valid(&schema, &json!("z"));
    }

    #[test]
    fn big_string_enum_invalid_type() {
        let schema: Value = serde_json::from_str(BIG_STRING_ENUM).unwrap();
        tests_util::is_not_valid(&schema, &json!(1));
        tests_util::is_not_valid(&schema, &json!(null));
    }

    #[test]
    fn big_string_enum_location() {
        let schema: Value = serde_json::from_str(BIG_STRING_ENUM).unwrap();
        tests_util::assert_schema_location(&schema, &json!("z"), "/enum");
    }

    #[test]
    fn big_string_enum_error_message() {
        let schema: Value = serde_json::from_str(BIG_STRING_ENUM).unwrap();
        tests_util::expect_errors(
            &schema,
            &json!("z"),
            &[r#""z" is not one of "a", "b" or 9 other candidates"#],
        );
    }
}
