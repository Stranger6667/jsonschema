//! ValidationErrorKind enum for Ruby.
use magnus::{
    gc, method,
    prelude::*,
    value::{Opaque, StaticSymbol},
    Error, RModule, Ruby, TypedData, Value,
};

use crate::{ser::value_to_ruby, static_id::define_rb_intern};

define_rb_intern!(static ID_NAME: "name");
define_rb_intern!(static ID_VALUE: "value");
define_rb_intern!(static ID_LIMIT: "limit");
define_rb_intern!(static ID_UNEXPECTED: "unexpected");
define_rb_intern!(static ID_CONTEXT: "context");
define_rb_intern!(static ID_ERROR: "error");
define_rb_intern!(static ID_EXPECTED_VALUE: "expected_value");
define_rb_intern!(static ID_CONTENT_ENCODING: "content_encoding");
define_rb_intern!(static ID_CONTENT_MEDIA_TYPE: "content_media_type");
define_rb_intern!(static ID_MESSAGE: "message");
define_rb_intern!(static ID_OPTIONS: "options");
define_rb_intern!(static ID_FORMAT: "format");
define_rb_intern!(static ID_MULTIPLE_OF: "multiple_of");
define_rb_intern!(static ID_SCHEMA: "schema");
define_rb_intern!(static ID_REASON: "reason");
define_rb_intern!(static ID_PROPERTY: "property");
define_rb_intern!(static ID_TYPES: "types");
define_rb_intern!(static ID_PATTERN: "pattern");
define_rb_intern!(static ID_CTX_INSTANCE_PATH: "instance_path");
define_rb_intern!(static ID_CTX_SCHEMA_PATH: "schema_path");
define_rb_intern!(static ID_CTX_EVALUATION_PATH: "evaluation_path");
define_rb_intern!(static ID_CTX_KIND: "kind");

#[derive(TypedData)]
#[magnus(
    class = "JSONSchema::ValidationErrorKind",
    free_immediately,
    size,
    mark
)]
pub struct ValidationErrorKind {
    name: String,
    data: Opaque<Value>,
}

impl magnus::typed_data::DataTypeFunctions for ValidationErrorKind {
    fn mark(&self, marker: &gc::Marker) {
        marker.mark(self.data);
    }
}

#[inline]
fn rb_hash1(ruby: &Ruby, k1: StaticSymbol, v1: Value) -> Result<Value, Error> {
    let hash = ruby.hash_new_capa(1);
    hash.aset(k1, v1)?;
    Ok(hash.as_value())
}

#[inline]
fn rb_hash2(
    ruby: &Ruby,
    k1: StaticSymbol,
    v1: Value,
    k2: StaticSymbol,
    v2: Value,
) -> Result<Value, Error> {
    let hash = ruby.hash_new_capa(2);
    hash.aset(k1, v1)?;
    hash.aset(k2, v2)?;
    Ok(hash.as_value())
}

/// Convert anyOf/oneOf context into a Ruby array of error branch arrays.
fn context_to_ruby(
    ruby: &Ruby,
    context: &[Vec<jsonschema::ValidationError<'static>>],
    mask: Option<&str>,
) -> Result<Value, Error> {
    let sym_message = ID_MESSAGE.to_symbol();
    let sym_instance_path = ID_CTX_INSTANCE_PATH.to_symbol();
    let sym_schema_path = ID_CTX_SCHEMA_PATH.to_symbol();
    let sym_evaluation_path = ID_CTX_EVALUATION_PATH.to_symbol();
    let sym_kind = ID_CTX_KIND.to_symbol();

    let branches = ruby.ary_new_capa(context.len());
    for branch in context {
        let errors = ruby.ary_new_capa(branch.len());
        for e in branch {
            let hash = ruby.hash_new_capa(5);
            let message = if let Some(mask) = mask {
                e.masked_with(mask).to_string()
            } else {
                e.to_string()
            };
            hash.aset(sym_message, ruby.into_value(message.as_str()))?;
            hash.aset(
                sym_instance_path,
                ruby.into_value(e.instance_path().as_str()),
            )?;
            hash.aset(sym_schema_path, ruby.into_value(e.schema_path().as_str()))?;
            hash.aset(
                sym_evaluation_path,
                ruby.into_value(e.evaluation_path().as_str()),
            )?;
            hash.aset(sym_kind, ruby.into_value(e.kind().keyword()))?;
            errors.push(hash)?;
        }
        branches.push(errors)?;
    }
    Ok(branches.as_value())
}

fn strings_to_ruby(ruby: &Ruby, strings: &[String]) -> Value {
    ruby.ary_from_iter(strings.iter().map(|s| ruby.into_value(s.as_str())))
        .as_value()
}

impl ValidationErrorKind {
    pub fn new(
        ruby: &Ruby,
        kind: &jsonschema::error::ValidationErrorKind,
        mask: Option<&str>,
    ) -> Result<Self, Error> {
        use jsonschema::error::ValidationErrorKind as K;

        let name = kind.keyword();

        let data = match kind {
            K::AdditionalItems { limit } => rb_hash1(
                ruby,
                ID_LIMIT.to_symbol(),
                ruby.integer_from_u64(*limit as u64).as_value(),
            )?,
            K::AdditionalProperties { unexpected }
            | K::UnevaluatedItems { unexpected }
            | K::UnevaluatedProperties { unexpected } => rb_hash1(
                ruby,
                ID_UNEXPECTED.to_symbol(),
                strings_to_ruby(ruby, unexpected),
            )?,
            K::AnyOf { context } => rb_hash1(
                ruby,
                ID_CONTEXT.to_symbol(),
                context_to_ruby(ruby, context, mask)?,
            )?,
            K::BacktrackLimitExceeded { error } => rb_hash1(
                ruby,
                ID_ERROR.to_symbol(),
                ruby.into_value(error.to_string().as_str()),
            )?,
            K::Constant { expected_value } => rb_hash1(
                ruby,
                ID_EXPECTED_VALUE.to_symbol(),
                value_to_ruby(ruby, expected_value)?,
            )?,
            K::Contains | K::FalseSchema | K::UniqueItems => ruby.hash_new().as_value(),
            K::ContentEncoding { content_encoding } => rb_hash1(
                ruby,
                ID_CONTENT_ENCODING.to_symbol(),
                ruby.into_value(content_encoding.as_str()),
            )?,
            K::ContentMediaType { content_media_type } => rb_hash1(
                ruby,
                ID_CONTENT_MEDIA_TYPE.to_symbol(),
                ruby.into_value(content_media_type.as_str()),
            )?,
            K::Custom { message, .. } => rb_hash1(
                ruby,
                ID_MESSAGE.to_symbol(),
                ruby.into_value(message.as_str()),
            )?,
            K::Enum { options } => {
                rb_hash1(ruby, ID_OPTIONS.to_symbol(), value_to_ruby(ruby, options)?)?
            }
            K::ExclusiveMaximum { limit }
            | K::ExclusiveMinimum { limit }
            | K::Maximum { limit }
            | K::Minimum { limit } => {
                rb_hash1(ruby, ID_LIMIT.to_symbol(), value_to_ruby(ruby, limit)?)?
            }
            K::Format { format } => rb_hash1(
                ruby,
                ID_FORMAT.to_symbol(),
                ruby.into_value(format.as_str()),
            )?,
            K::FromUtf8 { error } => rb_hash1(
                ruby,
                ID_ERROR.to_symbol(),
                ruby.into_value(error.to_string().as_str()),
            )?,
            K::MaxItems { limit }
            | K::MaxLength { limit }
            | K::MaxProperties { limit }
            | K::MinItems { limit }
            | K::MinLength { limit }
            | K::MinProperties { limit } => rb_hash1(
                ruby,
                ID_LIMIT.to_symbol(),
                ruby.integer_from_u64(*limit).as_value(),
            )?,
            K::MultipleOf { multiple_of } => rb_hash1(
                ruby,
                ID_MULTIPLE_OF.to_symbol(),
                value_to_ruby(ruby, multiple_of)?,
            )?,
            K::Not { schema } => {
                rb_hash1(ruby, ID_SCHEMA.to_symbol(), value_to_ruby(ruby, schema)?)?
            }
            K::OneOfMultipleValid { context } => rb_hash2(
                ruby,
                ID_REASON.to_symbol(),
                ruby.into_value("multipleValid"),
                ID_CONTEXT.to_symbol(),
                context_to_ruby(ruby, context, mask)?,
            )?,
            K::OneOfNotValid { context } => rb_hash2(
                ruby,
                ID_REASON.to_symbol(),
                ruby.into_value("notValid"),
                ID_CONTEXT.to_symbol(),
                context_to_ruby(ruby, context, mask)?,
            )?,
            K::Pattern { pattern } => rb_hash1(
                ruby,
                ID_PATTERN.to_symbol(),
                ruby.into_value(pattern.as_str()),
            )?,
            K::PropertyNames { error } => {
                let message = if let Some(mask) = mask {
                    error.masked_with(mask).to_string()
                } else {
                    error.to_string()
                };
                rb_hash1(
                    ruby,
                    ID_ERROR.to_symbol(),
                    ruby.into_value(message.as_str()),
                )?
            }
            K::Referencing(err) => rb_hash1(
                ruby,
                ID_ERROR.to_symbol(),
                ruby.into_value(err.to_string().as_str()),
            )?,
            K::Required { property } => rb_hash1(
                ruby,
                ID_PROPERTY.to_symbol(),
                value_to_ruby(ruby, property)?,
            )?,
            K::Type { kind } => {
                let types: Vec<Value> = match kind {
                    jsonschema::error::TypeKind::Single(ty) => vec![ruby.into_value(ty.as_str())],
                    jsonschema::error::TypeKind::Multiple(types) => types
                        .iter()
                        .map(|ty| ruby.into_value(ty.as_str()))
                        .collect(),
                };
                let rb_types = ruby.ary_from_iter(types);
                rb_hash1(ruby, ID_TYPES.to_symbol(), rb_types.as_value())?
            }
        };

        Ok(ValidationErrorKind {
            name: name.to_string(),
            data: data.into(),
        })
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn value(ruby: &Ruby, rb_self: &Self) -> Value {
        ruby.get_inner(rb_self.data)
    }

    fn as_hash(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        let hash = ruby.hash_new_capa(2);
        hash.aset(ID_NAME.to_symbol(), rb_self.name.as_str())?;
        hash.aset(ID_VALUE.to_symbol(), ruby.get_inner(rb_self.data))?;
        Ok(hash.as_value())
    }

    fn inspect(&self) -> String {
        format!("#<JSONSchema::ValidationErrorKind name={:?}>", self.name)
    }

    fn to_s(&self) -> &str {
        &self.name
    }
}

pub fn define_class(ruby: &Ruby, module: &RModule) -> Result<(), Error> {
    let class = module.define_class("ValidationErrorKind", ruby.class_object())?;
    class.define_method("name", method!(ValidationErrorKind::name, 0))?;
    class.define_method("value", method!(ValidationErrorKind::value, 0))?;
    class.define_method("to_h", method!(ValidationErrorKind::as_hash, 0))?;
    class.define_method("inspect", method!(ValidationErrorKind::inspect, 0))?;
    class.define_method("to_s", method!(ValidationErrorKind::to_s, 0))?;

    Ok(())
}
