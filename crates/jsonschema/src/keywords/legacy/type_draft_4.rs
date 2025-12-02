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
        self.types.contains_value_type(instance)
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

#[inline]
fn is_integer(num: &Number) -> bool {
    num.is_u64() || num.is_i64()
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
