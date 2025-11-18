use crate::{
    compiler,
    error::ValidationError,
    keywords::{type_, CompilationResult},
    paths::{LazyLocation, Location},
    types::{JsonType, JsonTypeSet},
    validator::Validate,
};
use referencing::Uri;
use serde_json::{json, Map, Number, Value};
use std::{str::FromStr, sync::Arc};

pub(crate) struct MultipleTypesValidator {
    types: JsonTypeSet,
    location: Location,
    absolute_path: Option<Arc<Uri<String>>>,
}

impl MultipleTypesValidator {
    #[inline]
    pub(crate) fn compile(
        items: &[Value],
        location: Location,
        absolute_path: Option<Arc<Uri<String>>>,
    ) -> CompilationResult<'_> {
        let mut types = JsonTypeSet::empty();
        for item in items {
            match item {
                Value::String(string) => {
                    if let Ok(ty) = JsonType::from_str(string.as_str()) {
                        types = types.insert(ty);
                    } else {
                        return Err(ValidationError::enumeration(
                            Location::new(),
                            location,
                            item,
                            &json!([
                                "array", "boolean", "integer", "null", "number", "object", "string"
                            ]),
                            absolute_path.clone(),
                        ));
                    }
                }
                _ => {
                    return Err(ValidationError::single_type_error(
                        Location::new(),
                        location,
                        item,
                        JsonType::String,
                        absolute_path.clone(),
                    ))
                }
            }
        }
        Ok(Box::new(MultipleTypesValidator {
            types,
            location,
            absolute_path,
        }))
    }
}

impl Validate for MultipleTypesValidator {
    fn is_valid(&self, instance: &Value) -> bool {
        self.types.contains_value_type(instance)
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
                self.absolute_path.clone(),
            ))
        }
    }
}

pub(crate) struct IntegerTypeValidator {
    location: Location,
    absolute_path: Option<Arc<Uri<String>>>,
}

impl IntegerTypeValidator {
    #[inline]
    pub(crate) fn compile<'a>(
        location: Location,
        absolute_path: Option<Arc<Uri<String>>>,
    ) -> CompilationResult<'a> {
        Ok(Box::new(IntegerTypeValidator {
            location,
            absolute_path,
        }))
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
                JsonType::Integer,
                self.absolute_path.clone(),
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
    let absolute_path = ctx.absolute_location(&location);
    match schema {
        Value::String(item) => Some(compile_single_type(
            item.as_str(),
            location,
            schema,
            absolute_path.as_ref(),
        )),
        Value::Array(items) => {
            if items.len() == 1 {
                let item = &items[0];
                if let Value::String(ty) = item {
                    Some(compile_single_type(
                        ty.as_str(),
                        location,
                        item,
                        absolute_path.as_ref(),
                    ))
                } else {
                    Some(Err(ValidationError::single_type_error(
                        Location::new(),
                        location,
                        item,
                        JsonType::String,
                        absolute_path,
                    )))
                }
            } else {
                Some(MultipleTypesValidator::compile(
                    items,
                    location,
                    absolute_path,
                ))
            }
        }
        _ => Some(Err(ValidationError::multiple_type_error(
            Location::new(),
            ctx.location().clone(),
            schema,
            JsonTypeSet::empty()
                .insert(JsonType::String)
                .insert(JsonType::Array),
            ctx.base_uri(),
        ))),
    }
}

fn compile_single_type<'a>(
    item: &str,
    location: Location,
    instance: &'a Value,
    absolute_path: Option<&Arc<Uri<String>>>,
) -> CompilationResult<'a> {
    match JsonType::from_str(item) {
        Ok(JsonType::Array) => type_::ArrayTypeValidator::compile(location, absolute_path.cloned()),
        Ok(JsonType::Boolean) => {
            type_::BooleanTypeValidator::compile(location, absolute_path.cloned())
        }
        Ok(JsonType::Integer) => IntegerTypeValidator::compile(location, absolute_path.cloned()),
        Ok(JsonType::Null) => type_::NullTypeValidator::compile(location, absolute_path.cloned()),
        Ok(JsonType::Number) => {
            type_::NumberTypeValidator::compile(location, absolute_path.cloned())
        }
        Ok(JsonType::Object) => {
            type_::ObjectTypeValidator::compile(location, absolute_path.cloned())
        }
        Ok(JsonType::String) => {
            type_::StringTypeValidator::compile(location, absolute_path.cloned())
        }
        Err(()) => Err(ValidationError::custom(
            Location::new(),
            location,
            instance,
            "Unexpected type",
        )),
    }
}
