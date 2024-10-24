use crate::{
    compiler,
    error::ValidationError,
    keywords::{type_, CompilationResult},
    paths::{LazyLocation, Location},
    primitive_type::{PrimitiveType, PrimitiveTypesBitMap},
    validator::Validate,
};
use serde_json::{json, Map, Number, Value};
use std::convert::TryFrom;

pub(crate) struct MultipleTypesValidator {
    types: PrimitiveTypesBitMap,
    location: Location,
}

impl MultipleTypesValidator {
    #[inline]
    pub(crate) fn compile(items: &[Value], location: Location) -> CompilationResult {
        let mut types = PrimitiveTypesBitMap::new();
        for item in items {
            match item {
                Value::String(string) => {
                    if let Ok(primitive_type) = PrimitiveType::try_from(string.as_str()) {
                        types |= primitive_type;
                    } else {
                        return Err(ValidationError::enumeration(
                            Location::new(),
                            location,
                            item,
                            &json!([
                                "array", "boolean", "integer", "null", "number", "object", "string"
                            ]),
                        ));
                    }
                }
                _ => {
                    return Err(ValidationError::single_type_error(
                        Location::new(),
                        location,
                        item,
                        PrimitiveType::String,
                    ))
                }
            }
        }
        Ok(Box::new(MultipleTypesValidator { types, location }))
    }
}

impl Validate for MultipleTypesValidator {
    fn is_valid(&self, instance: &Value) -> bool {
        match instance {
            Value::Array(_) => self.types.contains_type(PrimitiveType::Array),
            Value::Bool(_) => self.types.contains_type(PrimitiveType::Boolean),
            Value::Null => self.types.contains_type(PrimitiveType::Null),
            Value::Number(num) => {
                self.types.contains_type(PrimitiveType::Number)
                    || (self.types.contains_type(PrimitiveType::Integer) && is_integer(num))
            }
            Value::Object(_) => self.types.contains_type(PrimitiveType::Object),
            Value::String(_) => self.types.contains_type(PrimitiveType::String),
        }
    }
    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> Result<(), ValidationError<'i>> {
        if self.is_valid(instance) {
            Ok(())
        } else {
            Err(ValidationError::multiple_type_error(
                self.location.clone(),
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
    fn is_valid(&self, instance: &Value) -> bool {
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
    ) -> Result<(), ValidationError<'i>> {
        if self.is_valid(instance) {
            Ok(())
        } else {
            Err(ValidationError::single_type_error(
                self.location.clone(),
                location.into(),
                instance,
                PrimitiveType::Integer,
            ))
        }
    }
}

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
                        Location::new(),
                        location,
                        item,
                        PrimitiveType::String,
                    )))
                }
            } else {
                Some(MultipleTypesValidator::compile(items, location))
            }
        }
        _ => Some(Err(ValidationError::multiple_type_error(
            Location::new(),
            ctx.location().clone(),
            schema,
            PrimitiveTypesBitMap::new()
                .add_type(PrimitiveType::String)
                .add_type(PrimitiveType::Array),
        ))),
    }
}

fn compile_single_type<'a>(
    item: &str,
    location: Location,
    instance: &'a Value,
) -> CompilationResult<'a> {
    match PrimitiveType::try_from(item) {
        Ok(PrimitiveType::Array) => type_::ArrayTypeValidator::compile(location),
        Ok(PrimitiveType::Boolean) => type_::BooleanTypeValidator::compile(location),
        Ok(PrimitiveType::Integer) => IntegerTypeValidator::compile(location),
        Ok(PrimitiveType::Null) => type_::NullTypeValidator::compile(location),
        Ok(PrimitiveType::Number) => type_::NumberTypeValidator::compile(location),
        Ok(PrimitiveType::Object) => type_::ObjectTypeValidator::compile(location),
        Ok(PrimitiveType::String) => type_::StringTypeValidator::compile(location),
        Err(()) => Err(ValidationError::custom(
            Location::new(),
            location,
            instance,
            "Unexpected type",
        )),
    }
}
