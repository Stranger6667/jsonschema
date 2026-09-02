use crate::{
    compiler,
    error::ValidationError,
    keywords::{CompilationResult, NotNullable, Nullability, Nullable},
    paths::{LazyLocation, Location, RefTracker},
    types::{JsonType, JsonTypeSet},
    validator::{Validate, ValidationContext},
    Json, LazyInstance, Node,
};
use ahash::AHashSet;
use jsonschema_value::JsonNumber;
use serde_json::{Map, Value};
use std::{borrow::Cow, marker::PhantomData};

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
    pub(crate) fn compile<'a, F: Json>(
        schema: &'a Value,
        items: &'a [Value],
        location: Location,
    ) -> CompilationResult<'a, F> {
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

impl<F: Json> Validate<F> for EnumValidator {
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
            Err(ValidationError::enumeration(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance.lazy_value(),
                &self.options,
            ))
        }
    }

    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        // If the input value type is not in the types present among the enum options, then there
        // is no reason to compare it against all items - we know that
        // there are no items with such type at all
        if self.types.contains_value_type::<F>(instance) {
            self.items.iter().any(|item| instance.equals_value(item))
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
    pub(crate) fn compile<'a, F: Json>(
        schema: &'a Value,
        value: &'a Value,
        location: Location,
    ) -> CompilationResult<'a, F> {
        Ok(Box::new(SingleValueEnumValidator {
            options: schema.clone(),
            value: value.clone(),
            location,
        }))
    }
}

impl<F: Json> Validate<F> for SingleValueEnumValidator {
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
            Err(ValidationError::enumeration(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance.lazy_value(),
                &self.options,
            ))
        }
    }

    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        instance.equals_value(&self.value)
    }
}

#[derive(Debug)]
pub(crate) struct SmallStringEnumValidator<N> {
    options: Value,
    items: Vec<Box<str>>,
    location: Location,
    nullability: PhantomData<N>,
}

impl<N: Nullability> SmallStringEnumValidator<N> {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        schema: &'a Value,
        items: &'a [Value],
        location: Location,
    ) -> CompilationResult<'a, F> {
        Ok(Box::new(SmallStringEnumValidator::<N> {
            options: schema.clone(),
            items: items
                .iter()
                .filter_map(Value::as_str)
                .map(Into::into)
                .collect(),
            location,
            nullability: PhantomData,
        }))
    }
}

impl<F: Json, N: Nullability> Validate<F> for SmallStringEnumValidator<N> {
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
            Err(ValidationError::enumeration(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance.lazy_value(),
                &self.options,
            ))
        }
    }

    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        if let Some(s) = instance.as_string() {
            self.items.iter().any(|item| item.as_ref() == s.as_ref())
        } else {
            N::ACCEPTS_NULL && instance.is_null()
        }
    }
}

#[derive(Debug)]
pub(crate) struct BigStringEnumValidator<N> {
    options: Value,
    items: AHashSet<Box<str>>,
    location: Location,
    nullability: PhantomData<N>,
}

impl<N: Nullability> BigStringEnumValidator<N> {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        schema: &'a Value,
        items: &'a [Value],
        location: Location,
    ) -> CompilationResult<'a, F> {
        Ok(Box::new(BigStringEnumValidator::<N> {
            options: schema.clone(),
            items: items
                .iter()
                .filter_map(Value::as_str)
                .map(Into::into)
                .collect(),
            location,
            nullability: PhantomData,
        }))
    }
}

impl<F: Json, N: Nullability> Validate<F> for BigStringEnumValidator<N> {
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
            Err(ValidationError::enumeration(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance.lazy_value(),
                &self.options,
            ))
        }
    }

    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        if let Some(s) = instance.as_string() {
            self.items.contains(s.as_ref())
        } else {
            N::ACCEPTS_NULL && instance.is_null()
        }
    }
}

#[derive(Debug)]
pub(crate) struct IntegerEnumValidator<N> {
    options: Value,
    items: Vec<Value>,
    /// Sorted and deduplicated; answers for instances written as an `i64`.
    integers: Vec<i64>,
    location: Location,
    nullability: PhantomData<N>,
}

impl<N: Nullability> IntegerEnumValidator<N> {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        schema: &'a Value,
        items: &'a [Value],
        location: Location,
    ) -> CompilationResult<'a, F> {
        let mut integers: Vec<i64> = items.iter().filter_map(Value::as_i64).collect();
        integers.sort_unstable();
        integers.dedup();
        Ok(Box::new(IntegerEnumValidator::<N> {
            options: schema.clone(),
            items: items.iter().filter(|v| !v.is_null()).cloned().collect(),
            integers,
            location,
            nullability: PhantomData,
        }))
    }
}

impl<F: Json, N: Nullability> Validate<F> for IntegerEnumValidator<N> {
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
            Err(ValidationError::enumeration(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance.lazy_value(),
                &self.options,
            ))
        }
    }

    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        if let Some(number) = instance.as_number() {
            if let Some(value) = number.as_i64() {
                self.integers.binary_search(&value).is_ok()
            } else {
                // `2.0`, `1e2`, past `i64`: numeric equality across spellings and precisions
                self.items.iter().any(|item| instance.equals_value(item))
            }
        } else {
            N::ACCEPTS_NULL && instance.is_null()
        }
    }
}

#[inline]
pub(crate) fn compile<'a, F: Json>(
    ctx: &compiler::Context<F>,
    _: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a, F>> {
    if let Value::Array(items) = schema {
        let location = ctx.location().join("enum");
        if items.len() == 1 {
            let value = items.iter().next().expect("Vec is not empty");
            Some(SingleValueEnumValidator::compile(schema, value, location))
        } else if items
            .iter()
            .all(|v| matches!(v, Value::String(_) | Value::Null))
        {
            let small = items.len() <= STRING_ENUM_THRESHOLD;
            match (small, items.iter().any(Value::is_null)) {
                (true, true) => Some(SmallStringEnumValidator::<Nullable>::compile(
                    schema, items, location,
                )),
                (true, false) => Some(SmallStringEnumValidator::<NotNullable>::compile(
                    schema, items, location,
                )),
                (false, true) => Some(BigStringEnumValidator::<Nullable>::compile(
                    schema, items, location,
                )),
                (false, false) => Some(BigStringEnumValidator::<NotNullable>::compile(
                    schema, items, location,
                )),
            }
        } else if items.iter().all(|v| v.is_null() || v.as_i64().is_some()) {
            if items.iter().any(Value::is_null) {
                Some(IntegerEnumValidator::<Nullable>::compile(
                    schema, items, location,
                ))
            } else {
                Some(IntegerEnumValidator::<NotNullable>::compile(
                    schema, items, location,
                ))
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
            LazyInstance::Ready(Cow::Borrowed(schema)),
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

    // 11 strings plus null — the big string set with `null` allowed
    const BIG_STRING_OR_NULL_ENUM: &str = r#"{
        "enum": ["a","b","c","d","e","f","g","h","i","j","k",null]
    }"#;

    #[test_case(&json!({"enum": ["a", "b", null]}), &json!(null); "small accepts null")]
    #[test_case(&json!({"enum": ["a", "b", null]}), &json!("b"); "small accepts string")]
    #[test_case(&json!({"enum": [null, "a"]}), &json!("a"); "null first accepts string")]
    #[test_case(&serde_json::from_str(BIG_STRING_OR_NULL_ENUM).unwrap(), &json!(null); "big accepts null")]
    #[test_case(&serde_json::from_str(BIG_STRING_OR_NULL_ENUM).unwrap(), &json!("k"); "big accepts string")]
    fn string_or_null_enum_valid(schema: &Value, instance: &Value) {
        tests_util::is_valid(schema, instance);
    }

    #[test_case(&json!({"enum": ["a", "b", null]}), &json!("z"); "small rejects other string")]
    #[test_case(&json!({"enum": ["a", "b", null]}), &json!(1); "small rejects number")]
    #[test_case(&json!({"enum": ["a", "b"]}), &json!(null); "small without null rejects null")]
    #[test_case(&serde_json::from_str(BIG_STRING_OR_NULL_ENUM).unwrap(), &json!("z"); "big rejects other string")]
    #[test_case(&serde_json::from_str(BIG_STRING_OR_NULL_ENUM).unwrap(), &json!(false); "big rejects boolean")]
    #[test_case(&serde_json::from_str(BIG_STRING_ENUM).unwrap(), &json!(null); "big without null rejects null")]
    fn string_or_null_enum_invalid(schema: &Value, instance: &Value) {
        tests_util::is_not_valid(schema, instance);
    }

    #[test_case(&json!({"enum": [1, 2, 3]}), &json!(2); "integer accepts member")]
    #[test_case(&json!({"enum": [1, 2, 3]}), &json!(2.0); "integer accepts integral float")]
    #[test_case(&json!({"enum": [-1, 0]}), &json!(-0.0); "integer accepts negative zero")]
    #[test_case(&json!({"enum": [null, 10, 20]}), &json!(null); "integer with null accepts null")]
    #[test_case(&json!({"enum": [null, 10, 20]}), &json!(10); "null first accepts integer")]
    #[test_case(&json!({"enum": [9_007_199_254_740_993_i64]}), &json!(9_007_199_254_740_993_i64); "integer past 2^53 exact")]
    fn integer_enum_valid(schema: &Value, instance: &Value) {
        tests_util::is_valid(schema, instance);
    }

    #[test_case(&json!({"enum": [1, 2, 3]}), &json!(4); "integer rejects other integer")]
    #[test_case(&json!({"enum": [1, 2, 3]}), &json!("2"); "integer rejects string")]
    #[test_case(&json!({"enum": [1, 2, 3]}), &json!(2.5); "integer rejects fraction")]
    #[test_case(&json!({"enum": [1, 2, 3]}), &json!(null); "integer without null rejects null")]
    #[test_case(&json!({"enum": [1, 2, 3]}), &json!(true); "integer rejects boolean")]
    #[test_case(&json!({"enum": [1]}), &json!(18_446_744_073_709_551_615_u64); "integer rejects u64 past i64")]
    #[test_case(&json!({"enum": [9_007_199_254_740_993_i64]}), &json!(9_007_199_254_740_992.0); "integer rejects float neighbour past 2^53")]
    #[test_case(&json!({"enum": [null, 10, 20]}), &json!(15); "integer with null rejects other integer")]
    fn integer_enum_invalid(schema: &Value, instance: &Value) {
        tests_util::is_not_valid(schema, instance);
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
