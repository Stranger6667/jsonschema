use crate::{
    compiler,
    error::ValidationError,
    keywords::CompilationResult,
    paths::Location,
    primitive_type::{PrimitiveType, PrimitiveTypesBitMap},
    validator::Validate,
};
use serde_json::{json, Map, Number, Value};
use std::convert::TryFrom;

use crate::paths::LazyLocation;

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
                        location,
                        Location::new(),
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

pub(crate) struct NullTypeValidator {
    location: Location,
}

impl NullTypeValidator {
    #[inline]
    pub(crate) fn compile<'a>(location: Location) -> CompilationResult<'a> {
        Ok(Box::new(NullTypeValidator { location }))
    }
}

impl Validate for NullTypeValidator {
    fn is_valid(&self, instance: &Value) -> bool {
        instance.is_null()
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
                PrimitiveType::Null,
            ))
        }
    }
}

pub(crate) struct BooleanTypeValidator {
    location: Location,
}

impl BooleanTypeValidator {
    #[inline]
    pub(crate) fn compile<'a>(location: Location) -> CompilationResult<'a> {
        Ok(Box::new(BooleanTypeValidator { location }))
    }
}

impl Validate for BooleanTypeValidator {
    fn is_valid(&self, instance: &Value) -> bool {
        instance.is_boolean()
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
                PrimitiveType::Boolean,
            ))
        }
    }
}

pub(crate) struct StringTypeValidator {
    location: Location,
}

impl StringTypeValidator {
    #[inline]
    pub(crate) fn compile<'a>(location: Location) -> CompilationResult<'a> {
        Ok(Box::new(StringTypeValidator { location }))
    }
}

impl Validate for StringTypeValidator {
    fn is_valid(&self, instance: &Value) -> bool {
        instance.is_string()
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
                PrimitiveType::String,
            ))
        }
    }
}

pub(crate) struct ArrayTypeValidator {
    location: Location,
}

impl ArrayTypeValidator {
    #[inline]
    pub(crate) fn compile<'a>(location: Location) -> CompilationResult<'a> {
        Ok(Box::new(ArrayTypeValidator { location }))
    }
}

impl Validate for ArrayTypeValidator {
    fn is_valid(&self, instance: &Value) -> bool {
        instance.is_array()
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
                PrimitiveType::Array,
            ))
        }
    }
}

pub(crate) struct ObjectTypeValidator {
    location: Location,
}

impl ObjectTypeValidator {
    #[inline]
    pub(crate) fn compile<'a>(location: Location) -> CompilationResult<'a> {
        Ok(Box::new(ObjectTypeValidator { location }))
    }
}

impl Validate for ObjectTypeValidator {
    fn is_valid(&self, instance: &Value) -> bool {
        instance.is_object()
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
                PrimitiveType::Object,
            ))
        }
    }
}

pub(crate) struct NumberTypeValidator {
    location: Location,
}

impl NumberTypeValidator {
    #[inline]
    pub(crate) fn compile<'a>(location: Location) -> CompilationResult<'a> {
        Ok(Box::new(NumberTypeValidator { location }))
    }
}

impl Validate for NumberTypeValidator {
    fn is_valid(&self, instance: &Value) -> bool {
        instance.is_number()
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
                PrimitiveType::Number,
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
    num.is_u64() || num.is_i64() || num.as_f64().expect("Always valid").fract() == 0.
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
        Ok(PrimitiveType::Array) => ArrayTypeValidator::compile(location),
        Ok(PrimitiveType::Boolean) => BooleanTypeValidator::compile(location),
        Ok(PrimitiveType::Integer) => IntegerTypeValidator::compile(location),
        Ok(PrimitiveType::Null) => NullTypeValidator::compile(location),
        Ok(PrimitiveType::Number) => NumberTypeValidator::compile(location),
        Ok(PrimitiveType::Object) => ObjectTypeValidator::compile(location),
        Ok(PrimitiveType::String) => StringTypeValidator::compile(location),
        Err(()) => Err(ValidationError::custom(
            Location::new(),
            location,
            instance,
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
        tests_util::assert_schema_location(schema, instance, expected)
    }
}
