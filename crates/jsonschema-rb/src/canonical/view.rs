use jsonschema::{canonical::CanonicalView, JsonType};
use magnus::{method, prelude::*, DataTypeFunctions, Error, RHash, RModule, Ruby, Value};
use serde_json::Number;

use crate::{canonical::RbCanonicalSchema, ser::value_to_ruby};

fn wrap_schema(ruby: &Ruby, schema: jsonschema::canonical::CanonicalSchema) -> Value {
    ruby.obj_wrap(RbCanonicalSchema { inner: schema })
        .as_value()
}

fn number_to_ruby(ruby: &Ruby, number: Option<Number>) -> Result<Value, Error> {
    match number {
        Some(number) => value_to_ruby(ruby, &serde_json::Value::Number(number)),
        None => Ok(ruby.qnil().as_value()),
    }
}

fn required_number_to_ruby(ruby: &Ruby, number: &Number) -> Result<Value, Error> {
    value_to_ruby(ruby, &serde_json::Value::Number(number.clone()))
}

fn numbers_to_ruby(ruby: &Ruby, numbers: &[Number]) -> Result<Value, Error> {
    let array = ruby.ary_new_capa(numbers.len());
    for number in numbers {
        array.push(value_to_ruby(
            ruby,
            &serde_json::Value::Number(number.clone()),
        )?)?;
    }
    Ok(array.as_value())
}

fn schemas_to_ruby(
    ruby: &Ruby,
    schemas: &[jsonschema::canonical::CanonicalSchema],
) -> Result<Value, Error> {
    let array = ruby.ary_new_capa(schemas.len());
    for schema in schemas {
        array.push(wrap_schema(ruby, schema.clone()))?;
    }
    Ok(array.as_value())
}

#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Canonical::NullView", free_immediately)]
pub struct NullView;
impl DataTypeFunctions for NullView {}
impl NullView {
    fn inspect(_: &Self) -> &'static str {
        "#<JSONSchema::Canonical::NullView>"
    }
    fn deconstruct_keys(ruby: &Ruby, _: &Self, _keys: Value) -> RHash {
        ruby.hash_new()
    }
}

#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Canonical::TrueView", free_immediately)]
pub struct TrueView;
impl DataTypeFunctions for TrueView {}
impl TrueView {
    fn inspect(_: &Self) -> &'static str {
        "#<JSONSchema::Canonical::TrueView>"
    }
    fn deconstruct_keys(ruby: &Ruby, _: &Self, _keys: Value) -> RHash {
        ruby.hash_new()
    }
}

#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Canonical::FalseView", free_immediately)]
pub struct FalseView;
impl DataTypeFunctions for FalseView {}
impl FalseView {
    fn inspect(_: &Self) -> &'static str {
        "#<JSONSchema::Canonical::FalseView>"
    }
    fn deconstruct_keys(ruby: &Ruby, _: &Self, _keys: Value) -> RHash {
        ruby.hash_new()
    }
}

#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Canonical::BooleanView", free_immediately)]
pub struct BooleanView {
    variant: &'static str,
}
impl DataTypeFunctions for BooleanView {}
impl BooleanView {
    fn variant(ruby: &Ruby, rb_self: &Self) -> Value {
        ruby.sym_new(rb_self.variant).as_value()
    }
    fn inspect(rb_self: &Self) -> String {
        format!(
            "#<JSONSchema::Canonical::BooleanView variant=:{}>",
            rb_self.variant
        )
    }
    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(ruby.sym_new("variant"), ruby.sym_new(rb_self.variant))?;
        Ok(hash)
    }
}

#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Canonical::ConstView", free_immediately)]
pub struct ConstView {
    value: serde_json::Value,
}
impl DataTypeFunctions for ConstView {}
impl ConstView {
    fn value(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        value_to_ruby(ruby, &rb_self.value)
    }
    fn inspect(rb_self: &Self) -> String {
        format!(
            "#<JSONSchema::Canonical::ConstView value={}>",
            rb_self.value
        )
    }
    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(ruby.sym_new("value"), value_to_ruby(ruby, &rb_self.value)?)?;
        Ok(hash)
    }
}

#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Canonical::EnumView", free_immediately)]
pub struct EnumView {
    values: Vec<serde_json::Value>,
}
impl DataTypeFunctions for EnumView {}
impl EnumView {
    fn values(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        let array = ruby.ary_new_capa(rb_self.values.len());
        for value in &rb_self.values {
            array.push(value_to_ruby(ruby, value)?)?;
        }
        Ok(array.as_value())
    }
    fn inspect(rb_self: &Self) -> String {
        format!(
            "#<JSONSchema::Canonical::EnumView values={:?}>",
            rb_self.values
        )
    }
    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(ruby.sym_new("values"), EnumView::values(ruby, rb_self)?)?;
        Ok(hash)
    }
}

#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Canonical::MultiTypeView", free_immediately)]
pub struct MultiTypeView {
    types: Vec<&'static str>,
}
impl DataTypeFunctions for MultiTypeView {}
impl MultiTypeView {
    fn types(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        let array = ruby.ary_new_capa(rb_self.types.len());
        for type_name in &rb_self.types {
            array.push(ruby.sym_new(*type_name).as_value())?;
        }
        Ok(array.as_value())
    }
    fn inspect(rb_self: &Self) -> String {
        format!(
            "#<JSONSchema::Canonical::MultiTypeView types={:?}>",
            rb_self.types
        )
    }
    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(ruby.sym_new("types"), Self::types(ruby, rb_self)?)?;
        Ok(hash)
    }
}

#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Canonical::ReferenceView", free_immediately)]
pub struct ReferenceView {
    uri: String,
}
impl DataTypeFunctions for ReferenceView {}
impl ReferenceView {
    fn uri(rb_self: &Self) -> &str {
        &rb_self.uri
    }
    fn inspect(rb_self: &Self) -> String {
        format!(
            "#<JSONSchema::Canonical::ReferenceView uri={:?}>",
            rb_self.uri
        )
    }
    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(ruby.sym_new("uri"), rb_self.uri.as_str())?;
        Ok(hash)
    }
}

#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Canonical::RecursiveView", free_immediately)]
pub struct RecursiveView {
    uri: String,
}
impl DataTypeFunctions for RecursiveView {}
impl RecursiveView {
    fn uri(rb_self: &Self) -> &str {
        &rb_self.uri
    }
    fn inspect(rb_self: &Self) -> String {
        format!(
            "#<JSONSchema::Canonical::RecursiveView uri={:?}>",
            rb_self.uri
        )
    }
    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(ruby.sym_new("uri"), rb_self.uri.as_str())?;
        Ok(hash)
    }
}

#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Canonical::DynamicRefView", free_immediately)]
pub struct DynamicRefView {
    name: String,
}
impl DataTypeFunctions for DynamicRefView {}
impl DynamicRefView {
    fn name(rb_self: &Self) -> &str {
        &rb_self.name
    }
    fn inspect(rb_self: &Self) -> String {
        format!(
            "#<JSONSchema::Canonical::DynamicRefView name={:?}>",
            rb_self.name
        )
    }
    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(ruby.sym_new("name"), rb_self.name.as_str())?;
        Ok(hash)
    }
}

#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Canonical::RawView", free_immediately)]
pub struct RawView {
    schema: serde_json::Value,
}
impl DataTypeFunctions for RawView {}
impl RawView {
    fn schema(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        value_to_ruby(ruby, &rb_self.schema)
    }
    fn inspect(rb_self: &Self) -> String {
        format!(
            "#<JSONSchema::Canonical::RawView schema={}>",
            rb_self.schema
        )
    }
    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(
            ruby.sym_new("schema"),
            value_to_ruby(ruby, &rb_self.schema)?,
        )?;
        Ok(hash)
    }
}

#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Canonical::IntegerView", free_immediately)]
pub struct IntegerView {
    pub minimum: Option<Number>,
    pub maximum: Option<Number>,
    pub exclusive_minimum: Option<Number>,
    pub exclusive_maximum: Option<Number>,
    pub multiple_of: Option<Number>,
    pub not_multiple_of: Vec<Number>,
}
impl DataTypeFunctions for IntegerView {}
impl IntegerView {
    fn minimum(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        number_to_ruby(ruby, rb_self.minimum.clone())
    }
    fn maximum(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        number_to_ruby(ruby, rb_self.maximum.clone())
    }
    fn exclusive_minimum(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        number_to_ruby(ruby, rb_self.exclusive_minimum.clone())
    }
    fn exclusive_maximum(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        number_to_ruby(ruby, rb_self.exclusive_maximum.clone())
    }
    fn multiple_of(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        number_to_ruby(ruby, rb_self.multiple_of.clone())
    }
    fn not_multiple_of(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        numbers_to_ruby(ruby, &rb_self.not_multiple_of)
    }
    fn inspect(rb_self: &Self) -> String {
        format!(
            "#<JSONSchema::Canonical::IntegerView minimum={:?} maximum={:?}>",
            rb_self.minimum, rb_self.maximum
        )
    }
    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(ruby.sym_new("minimum"), Self::minimum(ruby, rb_self)?)?;
        hash.aset(ruby.sym_new("maximum"), Self::maximum(ruby, rb_self)?)?;
        hash.aset(
            ruby.sym_new("exclusive_minimum"),
            Self::exclusive_minimum(ruby, rb_self)?,
        )?;
        hash.aset(
            ruby.sym_new("exclusive_maximum"),
            Self::exclusive_maximum(ruby, rb_self)?,
        )?;
        hash.aset(
            ruby.sym_new("multiple_of"),
            Self::multiple_of(ruby, rb_self)?,
        )?;
        hash.aset(
            ruby.sym_new("not_multiple_of"),
            Self::not_multiple_of(ruby, rb_self)?,
        )?;
        Ok(hash)
    }
}

#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Canonical::NumberView", free_immediately)]
pub struct NumberView {
    pub minimum: Option<Number>,
    pub maximum: Option<Number>,
    pub exclusive_minimum: Option<Number>,
    pub exclusive_maximum: Option<Number>,
    pub multiple_of: Option<Number>,
    pub not_multiple_of: Vec<Number>,
}
impl DataTypeFunctions for NumberView {}
impl NumberView {
    fn minimum(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        number_to_ruby(ruby, rb_self.minimum.clone())
    }
    fn maximum(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        number_to_ruby(ruby, rb_self.maximum.clone())
    }
    fn exclusive_minimum(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        number_to_ruby(ruby, rb_self.exclusive_minimum.clone())
    }
    fn exclusive_maximum(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        number_to_ruby(ruby, rb_self.exclusive_maximum.clone())
    }
    fn multiple_of(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        number_to_ruby(ruby, rb_self.multiple_of.clone())
    }
    fn not_multiple_of(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        numbers_to_ruby(ruby, &rb_self.not_multiple_of)
    }
    fn inspect(rb_self: &Self) -> String {
        format!(
            "#<JSONSchema::Canonical::NumberView minimum={:?} maximum={:?}>",
            rb_self.minimum, rb_self.maximum
        )
    }
    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(ruby.sym_new("minimum"), Self::minimum(ruby, rb_self)?)?;
        hash.aset(ruby.sym_new("maximum"), Self::maximum(ruby, rb_self)?)?;
        hash.aset(
            ruby.sym_new("exclusive_minimum"),
            Self::exclusive_minimum(ruby, rb_self)?,
        )?;
        hash.aset(
            ruby.sym_new("exclusive_maximum"),
            Self::exclusive_maximum(ruby, rb_self)?,
        )?;
        hash.aset(
            ruby.sym_new("multiple_of"),
            Self::multiple_of(ruby, rb_self)?,
        )?;
        hash.aset(
            ruby.sym_new("not_multiple_of"),
            Self::not_multiple_of(ruby, rb_self)?,
        )?;
        Ok(hash)
    }
}

#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Canonical::ContentFacetView", free_immediately)]
pub struct ContentFacetView {
    content_encoding: Option<String>,
    content_media_type: Option<String>,
    content_schema: Option<serde_json::Value>,
}
impl DataTypeFunctions for ContentFacetView {}
impl ContentFacetView {
    fn content_encoding(ruby: &Ruby, rb_self: &Self) -> Value {
        match &rb_self.content_encoding {
            Some(encoding) => ruby.into_value(encoding.as_str()),
            None => ruby.qnil().as_value(),
        }
    }
    fn content_media_type(ruby: &Ruby, rb_self: &Self) -> Value {
        match &rb_self.content_media_type {
            Some(media_type) => ruby.into_value(media_type.as_str()),
            None => ruby.qnil().as_value(),
        }
    }
    fn content_schema(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        match &rb_self.content_schema {
            Some(schema) => value_to_ruby(ruby, schema),
            None => Ok(ruby.qnil().as_value()),
        }
    }
    fn inspect(_: &Self) -> &'static str {
        "#<JSONSchema::Canonical::ContentFacetView>"
    }
    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(
            ruby.sym_new("content_encoding"),
            Self::content_encoding(ruby, rb_self),
        )?;
        hash.aset(
            ruby.sym_new("content_media_type"),
            Self::content_media_type(ruby, rb_self),
        )?;
        hash.aset(
            ruby.sym_new("content_schema"),
            Self::content_schema(ruby, rb_self)?,
        )?;
        Ok(hash)
    }
}

#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Canonical::StringView", free_immediately)]
pub struct StringView {
    min_length: Option<Number>,
    max_length: Option<Number>,
    patterns: Vec<String>,
    not_patterns: Vec<String>,
    format: Option<String>,
    content: Vec<jsonschema::canonical::ContentFacetView>,
    extended_regex: bool,
}
impl DataTypeFunctions for StringView {}
impl StringView {
    fn min_length(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        number_to_ruby(ruby, rb_self.min_length.clone())
    }
    fn max_length(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        number_to_ruby(ruby, rb_self.max_length.clone())
    }
    fn patterns(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        let array = ruby.ary_new_capa(rb_self.patterns.len());
        for pattern in &rb_self.patterns {
            array.push(ruby.into_value(pattern.as_str()))?;
        }
        Ok(array.as_value())
    }
    fn not_patterns(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        let array = ruby.ary_new_capa(rb_self.not_patterns.len());
        for pattern in &rb_self.not_patterns {
            array.push(ruby.into_value(pattern.as_str()))?;
        }
        Ok(array.as_value())
    }
    fn format(ruby: &Ruby, rb_self: &Self) -> Value {
        match &rb_self.format {
            Some(format) => ruby.into_value(format.as_str()),
            None => ruby.qnil().as_value(),
        }
    }
    fn content(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        let array = ruby.ary_new_capa(rb_self.content.len());
        for facet in &rb_self.content {
            let wrapped = ruby.obj_wrap(ContentFacetView {
                content_encoding: facet.content_encoding.clone(),
                content_media_type: facet.content_media_type.clone(),
                content_schema: facet.content_schema.clone(),
            });
            array.push(wrapped)?;
        }
        Ok(array.as_value())
    }
    fn extended_regex_p(rb_self: &Self) -> bool {
        rb_self.extended_regex
    }
    fn inspect(rb_self: &Self) -> String {
        format!(
            "#<JSONSchema::Canonical::StringView min_length={:?} max_length={:?}>",
            rb_self.min_length, rb_self.max_length
        )
    }
    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(ruby.sym_new("min_length"), Self::min_length(ruby, rb_self)?)?;
        hash.aset(ruby.sym_new("max_length"), Self::max_length(ruby, rb_self)?)?;
        hash.aset(ruby.sym_new("patterns"), Self::patterns(ruby, rb_self)?)?;
        hash.aset(
            ruby.sym_new("not_patterns"),
            Self::not_patterns(ruby, rb_self)?,
        )?;
        hash.aset(ruby.sym_new("format"), Self::format(ruby, rb_self))?;
        hash.aset(ruby.sym_new("content"), Self::content(ruby, rb_self)?)?;
        hash.aset(ruby.sym_new("extended_regex"), rb_self.extended_regex)?;
        Ok(hash)
    }
}

#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Canonical::ContainsView", free_immediately)]
pub struct ContainsView {
    schema: jsonschema::canonical::CanonicalSchema,
    min_contains: Number,
    max_contains: Option<Number>,
}
impl DataTypeFunctions for ContainsView {}
impl ContainsView {
    fn schema(ruby: &Ruby, rb_self: &Self) -> Value {
        wrap_schema(ruby, rb_self.schema.clone())
    }
    fn min_contains(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        required_number_to_ruby(ruby, &rb_self.min_contains)
    }
    fn max_contains(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        number_to_ruby(ruby, rb_self.max_contains.clone())
    }
    fn inspect(rb_self: &Self) -> String {
        format!(
            "#<JSONSchema::Canonical::ContainsView min_contains={}>",
            rb_self.min_contains
        )
    }
    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(ruby.sym_new("schema"), Self::schema(ruby, rb_self))?;
        hash.aset(
            ruby.sym_new("min_contains"),
            Self::min_contains(ruby, rb_self)?,
        )?;
        hash.aset(
            ruby.sym_new("max_contains"),
            Self::max_contains(ruby, rb_self)?,
        )?;
        Ok(hash)
    }
}

#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Canonical::ArrayView", free_immediately)]
pub struct ArrayView {
    prefix: Vec<jsonschema::canonical::CanonicalSchema>,
    tail: Option<jsonschema::canonical::CanonicalSchema>,
    min_items: Number,
    max_items: Option<Number>,
    unique_items: bool,
    repeated_items: bool,
    contains: Vec<(
        jsonschema::canonical::CanonicalSchema,
        Number,
        Option<Number>,
    )>,
}
impl DataTypeFunctions for ArrayView {}
impl ArrayView {
    fn prefix(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        schemas_to_ruby(ruby, &rb_self.prefix)
    }
    fn tail(ruby: &Ruby, rb_self: &Self) -> Value {
        match &rb_self.tail {
            Some(schema) => wrap_schema(ruby, schema.clone()),
            None => ruby.qnil().as_value(),
        }
    }
    fn min_items(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        required_number_to_ruby(ruby, &rb_self.min_items)
    }
    fn max_items(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        number_to_ruby(ruby, rb_self.max_items.clone())
    }
    fn unique_items_p(rb_self: &Self) -> bool {
        rb_self.unique_items
    }
    fn repeated_items_p(rb_self: &Self) -> bool {
        rb_self.repeated_items
    }
    fn contains(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        let array = ruby.ary_new_capa(rb_self.contains.len());
        for (schema, min_contains, max_contains) in &rb_self.contains {
            let wrapped = ruby.obj_wrap(ContainsView {
                schema: schema.clone(),
                min_contains: min_contains.clone(),
                max_contains: max_contains.clone(),
            });
            array.push(wrapped)?;
        }
        Ok(array.as_value())
    }
    fn inspect(_: &Self) -> &'static str {
        "#<JSONSchema::Canonical::ArrayView>"
    }
    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(ruby.sym_new("prefix"), Self::prefix(ruby, rb_self)?)?;
        hash.aset(ruby.sym_new("tail"), Self::tail(ruby, rb_self))?;
        hash.aset(ruby.sym_new("min_items"), Self::min_items(ruby, rb_self)?)?;
        hash.aset(ruby.sym_new("max_items"), Self::max_items(ruby, rb_self)?)?;
        hash.aset(ruby.sym_new("unique_items"), Self::unique_items_p(rb_self))?;
        hash.aset(
            ruby.sym_new("repeated_items"),
            Self::repeated_items_p(rb_self),
        )?;
        hash.aset(ruby.sym_new("contains"), Self::contains(ruby, rb_self)?)?;
        Ok(hash)
    }
}

#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Canonical::RequiredProperty", free_immediately)]
pub struct RequiredProperty {
    name: String,
}
impl DataTypeFunctions for RequiredProperty {}
impl RequiredProperty {
    fn name(rb_self: &Self) -> &str {
        &rb_self.name
    }
    fn inspect(rb_self: &Self) -> String {
        format!(
            "#<JSONSchema::Canonical::RequiredProperty name={:?}>",
            rb_self.name
        )
    }
    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(ruby.sym_new("name"), rb_self.name.as_str())?;
        Ok(hash)
    }
}

#[derive(magnus::TypedData)]
#[magnus(
    class = "JSONSchema::Canonical::PatternPropertyRequirement",
    free_immediately
)]
pub struct PatternPropertyRequirement {
    pattern: String,
    schema: jsonschema::canonical::CanonicalSchema,
}
impl DataTypeFunctions for PatternPropertyRequirement {}
impl PatternPropertyRequirement {
    fn pattern(rb_self: &Self) -> &str {
        &rb_self.pattern
    }
    fn schema(ruby: &Ruby, rb_self: &Self) -> Value {
        wrap_schema(ruby, rb_self.schema.clone())
    }
    fn inspect(rb_self: &Self) -> String {
        format!(
            "#<JSONSchema::Canonical::PatternPropertyRequirement pattern={:?}>",
            rb_self.pattern
        )
    }
    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(ruby.sym_new("pattern"), rb_self.pattern.as_str())?;
        hash.aset(ruby.sym_new("schema"), Self::schema(ruby, rb_self))?;
        Ok(hash)
    }
}

#[derive(magnus::TypedData)]
#[magnus(
    class = "JSONSchema::Canonical::AdditionalPropertiesRequirement",
    free_immediately
)]
pub struct AdditionalPropertiesRequirement {
    schema: jsonschema::canonical::CanonicalSchema,
}
impl DataTypeFunctions for AdditionalPropertiesRequirement {}
impl AdditionalPropertiesRequirement {
    fn schema(ruby: &Ruby, rb_self: &Self) -> Value {
        wrap_schema(ruby, rb_self.schema.clone())
    }
    fn inspect(_: &Self) -> &'static str {
        "#<JSONSchema::Canonical::AdditionalPropertiesRequirement>"
    }
    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(ruby.sym_new("schema"), Self::schema(ruby, rb_self))?;
        Ok(hash)
    }
}

#[derive(magnus::TypedData)]
#[magnus(
    class = "JSONSchema::Canonical::DependentPropertiesRequirement",
    free_immediately
)]
pub struct DependentPropertiesRequirement {
    property: String,
    required_properties: Vec<String>,
}
impl DataTypeFunctions for DependentPropertiesRequirement {}
impl DependentPropertiesRequirement {
    fn property(rb_self: &Self) -> &str {
        &rb_self.property
    }
    fn required_properties(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        let array = ruby.ary_new_capa(rb_self.required_properties.len());
        for property in &rb_self.required_properties {
            array.push(ruby.into_value(property.as_str()))?;
        }
        Ok(array.as_value())
    }
    fn inspect(rb_self: &Self) -> String {
        format!(
            "#<JSONSchema::Canonical::DependentPropertiesRequirement property={:?}>",
            rb_self.property
        )
    }
    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(ruby.sym_new("property"), rb_self.property.as_str())?;
        hash.aset(
            ruby.sym_new("required_properties"),
            Self::required_properties(ruby, rb_self)?,
        )?;
        Ok(hash)
    }
}

#[derive(magnus::TypedData)]
#[magnus(
    class = "JSONSchema::Canonical::DependentSchemaRequirement",
    free_immediately
)]
pub struct DependentSchemaRequirement {
    property: String,
    schema: jsonschema::canonical::CanonicalSchema,
}
impl DataTypeFunctions for DependentSchemaRequirement {}
impl DependentSchemaRequirement {
    fn property(rb_self: &Self) -> &str {
        &rb_self.property
    }
    fn schema(ruby: &Ruby, rb_self: &Self) -> Value {
        wrap_schema(ruby, rb_self.schema.clone())
    }
    fn inspect(rb_self: &Self) -> String {
        format!(
            "#<JSONSchema::Canonical::DependentSchemaRequirement property={:?}>",
            rb_self.property
        )
    }
    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(ruby.sym_new("property"), rb_self.property.as_str())?;
        hash.aset(ruby.sym_new("schema"), Self::schema(ruby, rb_self))?;
        Ok(hash)
    }
}

#[derive(magnus::TypedData)]
#[magnus(
    class = "JSONSchema::Canonical::NamedPropertyConstraint",
    free_immediately
)]
pub struct NamedPropertyConstraint {
    name: String,
    schema: jsonschema::canonical::CanonicalSchema,
}
impl DataTypeFunctions for NamedPropertyConstraint {}
impl NamedPropertyConstraint {
    fn name(rb_self: &Self) -> &str {
        &rb_self.name
    }
    fn schema(ruby: &Ruby, rb_self: &Self) -> Value {
        wrap_schema(ruby, rb_self.schema.clone())
    }
    fn inspect(rb_self: &Self) -> String {
        format!(
            "#<JSONSchema::Canonical::NamedPropertyConstraint name={:?}>",
            rb_self.name
        )
    }
    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(ruby.sym_new("name"), rb_self.name.as_str())?;
        hash.aset(ruby.sym_new("schema"), Self::schema(ruby, rb_self))?;
        Ok(hash)
    }
}

#[derive(magnus::TypedData)]
#[magnus(
    class = "JSONSchema::Canonical::PatternPropertyConstraint",
    free_immediately
)]
pub struct PatternPropertyConstraint {
    pattern: String,
    schema: jsonschema::canonical::CanonicalSchema,
}
impl DataTypeFunctions for PatternPropertyConstraint {}
impl PatternPropertyConstraint {
    fn pattern(rb_self: &Self) -> &str {
        &rb_self.pattern
    }
    fn schema(ruby: &Ruby, rb_self: &Self) -> Value {
        wrap_schema(ruby, rb_self.schema.clone())
    }
    fn inspect(rb_self: &Self) -> String {
        format!(
            "#<JSONSchema::Canonical::PatternPropertyConstraint pattern={:?}>",
            rb_self.pattern
        )
    }
    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(ruby.sym_new("pattern"), rb_self.pattern.as_str())?;
        hash.aset(ruby.sym_new("schema"), Self::schema(ruby, rb_self))?;
        Ok(hash)
    }
}

#[derive(magnus::TypedData)]
#[magnus(
    class = "JSONSchema::Canonical::AdditionalPropertiesConstraint",
    free_immediately
)]
pub struct AdditionalPropertiesConstraint {
    schema: jsonschema::canonical::CanonicalSchema,
}
impl DataTypeFunctions for AdditionalPropertiesConstraint {}
impl AdditionalPropertiesConstraint {
    fn schema(ruby: &Ruby, rb_self: &Self) -> Value {
        wrap_schema(ruby, rb_self.schema.clone())
    }
    fn inspect(_: &Self) -> &'static str {
        "#<JSONSchema::Canonical::AdditionalPropertiesConstraint>"
    }
    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(ruby.sym_new("schema"), Self::schema(ruby, rb_self))?;
        Ok(hash)
    }
}

#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Canonical::ObjectView", free_immediately)]
pub struct ObjectView {
    requirements: Vec<jsonschema::canonical::ObjectRequirementView>,
    constraints: Vec<jsonschema::canonical::ObjectConstraintView>,
    property_names: Option<jsonschema::canonical::CanonicalSchema>,
    min_properties: Option<Number>,
    max_properties: Option<Number>,
}
impl DataTypeFunctions for ObjectView {}
impl ObjectView {
    fn requirements(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        let array = ruby.ary_new_capa(rb_self.requirements.len());
        for requirement in &rb_self.requirements {
            array.push(requirement_to_ruby(ruby, requirement))?;
        }
        Ok(array.as_value())
    }
    fn constraints(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        let array = ruby.ary_new_capa(rb_self.constraints.len());
        for constraint in &rb_self.constraints {
            array.push(constraint_to_ruby(ruby, constraint))?;
        }
        Ok(array.as_value())
    }
    fn property_names(ruby: &Ruby, rb_self: &Self) -> Value {
        match &rb_self.property_names {
            Some(schema) => wrap_schema(ruby, schema.clone()),
            None => ruby.qnil().as_value(),
        }
    }
    fn min_properties(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        number_to_ruby(ruby, rb_self.min_properties.clone())
    }
    fn max_properties(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        number_to_ruby(ruby, rb_self.max_properties.clone())
    }
    fn inspect(_: &Self) -> &'static str {
        "#<JSONSchema::Canonical::ObjectView>"
    }
    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(
            ruby.sym_new("requirements"),
            Self::requirements(ruby, rb_self)?,
        )?;
        hash.aset(
            ruby.sym_new("constraints"),
            Self::constraints(ruby, rb_self)?,
        )?;
        hash.aset(
            ruby.sym_new("property_names"),
            Self::property_names(ruby, rb_self),
        )?;
        hash.aset(
            ruby.sym_new("min_properties"),
            Self::min_properties(ruby, rb_self)?,
        )?;
        hash.aset(
            ruby.sym_new("max_properties"),
            Self::max_properties(ruby, rb_self)?,
        )?;
        Ok(hash)
    }
}

fn requirement_to_ruby(
    ruby: &Ruby,
    requirement: &jsonschema::canonical::ObjectRequirementView,
) -> Value {
    match requirement {
        jsonschema::canonical::ObjectRequirementView::RequiredProperty { name } => ruby
            .obj_wrap(RequiredProperty { name: name.clone() })
            .as_value(),
        jsonschema::canonical::ObjectRequirementView::PatternPropertyRequirement {
            pattern,
            schema,
        } => ruby
            .obj_wrap(PatternPropertyRequirement {
                pattern: pattern.clone(),
                schema: schema.clone(),
            })
            .as_value(),
        jsonschema::canonical::ObjectRequirementView::AdditionalPropertiesRequirement {
            schema,
        } => ruby
            .obj_wrap(AdditionalPropertiesRequirement {
                schema: schema.clone(),
            })
            .as_value(),
        jsonschema::canonical::ObjectRequirementView::DependentPropertiesRequirement {
            property,
            required_properties,
        } => ruby
            .obj_wrap(DependentPropertiesRequirement {
                property: property.clone(),
                required_properties: required_properties.clone(),
            })
            .as_value(),
        jsonschema::canonical::ObjectRequirementView::DependentSchemaRequirement {
            property,
            schema,
        } => ruby
            .obj_wrap(DependentSchemaRequirement {
                property: property.clone(),
                schema: schema.clone(),
            })
            .as_value(),
    }
}

fn constraint_to_ruby(
    ruby: &Ruby,
    constraint: &jsonschema::canonical::ObjectConstraintView,
) -> Value {
    match constraint {
        jsonschema::canonical::ObjectConstraintView::NamedProperty { name, schema } => ruby
            .obj_wrap(NamedPropertyConstraint {
                name: name.clone(),
                schema: schema.clone(),
            })
            .as_value(),
        jsonschema::canonical::ObjectConstraintView::PatternProperty { pattern, schema } => ruby
            .obj_wrap(PatternPropertyConstraint {
                pattern: pattern.clone(),
                schema: schema.clone(),
            })
            .as_value(),
        jsonschema::canonical::ObjectConstraintView::AdditionalProperties { schema } => ruby
            .obj_wrap(AdditionalPropertiesConstraint {
                schema: schema.clone(),
            })
            .as_value(),
    }
}

#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Canonical::AllOfView", free_immediately)]
pub struct AllOfView {
    schemas: Vec<jsonschema::canonical::CanonicalSchema>,
}
impl DataTypeFunctions for AllOfView {}
impl AllOfView {
    fn schemas(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        schemas_to_ruby(ruby, &rb_self.schemas)
    }
    fn inspect(_: &Self) -> &'static str {
        "#<JSONSchema::Canonical::AllOfView>"
    }
    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(ruby.sym_new("schemas"), Self::schemas(ruby, rb_self)?)?;
        Ok(hash)
    }
}

#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Canonical::AnyOfView", free_immediately)]
pub struct AnyOfView {
    schemas: Vec<jsonschema::canonical::CanonicalSchema>,
}
impl DataTypeFunctions for AnyOfView {}
impl AnyOfView {
    fn schemas(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        schemas_to_ruby(ruby, &rb_self.schemas)
    }
    fn inspect(_: &Self) -> &'static str {
        "#<JSONSchema::Canonical::AnyOfView>"
    }
    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(ruby.sym_new("schemas"), Self::schemas(ruby, rb_self)?)?;
        Ok(hash)
    }
}

#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Canonical::OneOfView", free_immediately)]
pub struct OneOfView {
    schemas: Vec<jsonschema::canonical::CanonicalSchema>,
}
impl DataTypeFunctions for OneOfView {}
impl OneOfView {
    fn schemas(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        schemas_to_ruby(ruby, &rb_self.schemas)
    }
    fn inspect(_: &Self) -> &'static str {
        "#<JSONSchema::Canonical::OneOfView>"
    }
    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(ruby.sym_new("schemas"), Self::schemas(ruby, rb_self)?)?;
        Ok(hash)
    }
}

#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Canonical::NotView", free_immediately)]
pub struct NotView {
    schema: jsonschema::canonical::CanonicalSchema,
}
impl DataTypeFunctions for NotView {}
impl NotView {
    fn schema(ruby: &Ruby, rb_self: &Self) -> Value {
        wrap_schema(ruby, rb_self.schema.clone())
    }
    fn inspect(_: &Self) -> &'static str {
        "#<JSONSchema::Canonical::NotView>"
    }
    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(ruby.sym_new("schema"), Self::schema(ruby, rb_self))?;
        Ok(hash)
    }
}

#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Canonical::IfThenElseView", free_immediately)]
pub struct IfThenElseView {
    condition: jsonschema::canonical::CanonicalSchema,
    then_branch: Option<jsonschema::canonical::CanonicalSchema>,
    else_branch: Option<jsonschema::canonical::CanonicalSchema>,
}
impl DataTypeFunctions for IfThenElseView {}
impl IfThenElseView {
    fn condition(ruby: &Ruby, rb_self: &Self) -> Value {
        wrap_schema(ruby, rb_self.condition.clone())
    }
    fn then_branch(ruby: &Ruby, rb_self: &Self) -> Value {
        match &rb_self.then_branch {
            Some(schema) => wrap_schema(ruby, schema.clone()),
            None => ruby.qnil().as_value(),
        }
    }
    fn else_branch(ruby: &Ruby, rb_self: &Self) -> Value {
        match &rb_self.else_branch {
            Some(schema) => wrap_schema(ruby, schema.clone()),
            None => ruby.qnil().as_value(),
        }
    }
    fn inspect(_: &Self) -> &'static str {
        "#<JSONSchema::Canonical::IfThenElseView>"
    }
    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(ruby.sym_new("condition"), Self::condition(ruby, rb_self))?;
        hash.aset(
            ruby.sym_new("then_branch"),
            Self::then_branch(ruby, rb_self),
        )?;
        hash.aset(
            ruby.sym_new("else_branch"),
            Self::else_branch(ruby, rb_self),
        )?;
        Ok(hash)
    }
}

/// A value matches iff its JSON type is `type_name` *and* it satisfies `body`; other types do not match.
#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Canonical::TypedGroupView", free_immediately)]
pub struct TypedGroupView {
    type_name: String,
    body: jsonschema::canonical::CanonicalSchema,
}
impl DataTypeFunctions for TypedGroupView {}
impl TypedGroupView {
    fn type_name(ruby: &Ruby, rb_self: &Self) -> Value {
        ruby.sym_new(rb_self.type_name.as_str()).as_value()
    }
    fn body(ruby: &Ruby, rb_self: &Self) -> Value {
        wrap_schema(ruby, rb_self.body.clone())
    }
    fn inspect(rb_self: &Self) -> String {
        format!(
            "#<JSONSchema::Canonical::TypedGroupView type_name={:?}>",
            rb_self.type_name
        )
    }
    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(
            ruby.sym_new("type_name"),
            ruby.sym_new(rb_self.type_name.as_str()),
        )?;
        hash.aset(ruby.sym_new("body"), Self::body(ruby, rb_self))?;
        Ok(hash)
    }
}

/// Constrains only values of JSON type `type_name` (they must satisfy `body`); any other type matches unconditionally.
#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Canonical::TypeGuardView", free_immediately)]
pub struct TypeGuardView {
    type_name: String,
    body: jsonschema::canonical::CanonicalSchema,
}
impl DataTypeFunctions for TypeGuardView {}
impl TypeGuardView {
    fn type_name(ruby: &Ruby, rb_self: &Self) -> Value {
        ruby.sym_new(rb_self.type_name.as_str()).as_value()
    }
    fn body(ruby: &Ruby, rb_self: &Self) -> Value {
        wrap_schema(ruby, rb_self.body.clone())
    }
    fn inspect(rb_self: &Self) -> String {
        format!(
            "#<JSONSchema::Canonical::TypeGuardView type_name={:?}>",
            rb_self.type_name
        )
    }
    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(
            ruby.sym_new("type_name"),
            ruby.sym_new(rb_self.type_name.as_str()),
        )?;
        hash.aset(ruby.sym_new("body"), Self::body(ruby, rb_self))?;
        Ok(hash)
    }
}

pub(crate) fn view_to_ruby(ruby: &Ruby, view: CanonicalView) -> Value {
    use jsonschema::canonical::BooleanVariant;

    match view {
        CanonicalView::Null => ruby.obj_wrap(NullView).as_value(),
        CanonicalView::True => ruby.obj_wrap(TrueView).as_value(),
        CanonicalView::False => ruby.obj_wrap(FalseView).as_value(),
        CanonicalView::Boolean(boolean) => {
            let variant = match boolean.variant {
                BooleanVariant::Any => "any",
                BooleanVariant::JustTrue => "just_true",
                BooleanVariant::JustFalse => "just_false",
            };
            ruby.obj_wrap(BooleanView { variant }).as_value()
        }
        CanonicalView::Integer(integer) => ruby
            .obj_wrap(IntegerView {
                minimum: integer.minimum,
                maximum: integer.maximum,
                exclusive_minimum: integer.exclusive_minimum,
                exclusive_maximum: integer.exclusive_maximum,
                multiple_of: integer.multiple_of,
                not_multiple_of: integer.not_multiple_of,
            })
            .as_value(),
        CanonicalView::Number(number) => ruby
            .obj_wrap(NumberView {
                minimum: number.minimum,
                maximum: number.maximum,
                exclusive_minimum: number.exclusive_minimum,
                exclusive_maximum: number.exclusive_maximum,
                multiple_of: number.multiple_of,
                not_multiple_of: number.not_multiple_of,
            })
            .as_value(),
        CanonicalView::String(string) => ruby
            .obj_wrap(StringView {
                min_length: string.min_length,
                max_length: string.max_length,
                patterns: string.patterns,
                not_patterns: string.not_patterns,
                format: string.format,
                content: string.content,
                extended_regex: string.extended_regex,
            })
            .as_value(),
        CanonicalView::Array(array) => ruby
            .obj_wrap(ArrayView {
                prefix: array.prefix,
                tail: array.tail,
                min_items: array.min_items,
                max_items: array.max_items,
                unique_items: array.unique_items,
                repeated_items: array.repeated_items,
                contains: array
                    .contains
                    .into_iter()
                    .map(|contains| {
                        (
                            contains.schema,
                            contains.min_contains,
                            contains.max_contains,
                        )
                    })
                    .collect(),
            })
            .as_value(),
        CanonicalView::Object(object) => ruby
            .obj_wrap(ObjectView {
                requirements: object.requirements,
                constraints: object.constraints,
                property_names: object.property_names,
                min_properties: object.min_properties,
                max_properties: object.max_properties,
            })
            .as_value(),
        CanonicalView::MultiType(types) => ruby
            .obj_wrap(MultiTypeView {
                types: types.iter().map(JsonType::as_str).collect(),
            })
            .as_value(),
        CanonicalView::AllOf(schemas) => ruby.obj_wrap(AllOfView { schemas }).as_value(),
        CanonicalView::AnyOf(schemas) => ruby.obj_wrap(AnyOfView { schemas }).as_value(),
        CanonicalView::OneOf(schemas) => ruby.obj_wrap(OneOfView { schemas }).as_value(),
        CanonicalView::Not(schema) => ruby.obj_wrap(NotView { schema }).as_value(),
        CanonicalView::IfThenElse(if_then_else) => ruby
            .obj_wrap(IfThenElseView {
                condition: if_then_else.condition,
                then_branch: if_then_else.then_branch,
                else_branch: if_then_else.else_branch,
            })
            .as_value(),
        CanonicalView::TypedGroup(group) => ruby
            .obj_wrap(TypedGroupView {
                type_name: group.ty.as_str().to_string(),
                body: group.body,
            })
            .as_value(),
        CanonicalView::TypeGuard(guard) => ruby
            .obj_wrap(TypeGuardView {
                type_name: guard.ty.as_str().to_string(),
                body: guard.body,
            })
            .as_value(),
        CanonicalView::Const(value) => ruby.obj_wrap(ConstView { value }).as_value(),
        CanonicalView::Enum(values) => ruby.obj_wrap(EnumView { values }).as_value(),
        CanonicalView::Reference(uri) => ruby.obj_wrap(ReferenceView { uri }).as_value(),
        CanonicalView::Recursive(uri) => ruby.obj_wrap(RecursiveView { uri }).as_value(),
        CanonicalView::DynamicRef(name) => ruby.obj_wrap(DynamicRefView { name }).as_value(),
        CanonicalView::Raw(schema) => ruby.obj_wrap(RawView { schema }).as_value(),
    }
}

pub(crate) fn register_view_classes(ruby: &Ruby, module: &RModule) -> Result<(), Error> {
    let class = module.define_class("NullView", ruby.class_object())?;
    class.define_method("inspect", method!(NullView::inspect, 0))?;
    class.define_method("deconstruct_keys", method!(NullView::deconstruct_keys, 1))?;

    let class = module.define_class("TrueView", ruby.class_object())?;
    class.define_method("inspect", method!(TrueView::inspect, 0))?;
    class.define_method("deconstruct_keys", method!(TrueView::deconstruct_keys, 1))?;

    let class = module.define_class("FalseView", ruby.class_object())?;
    class.define_method("inspect", method!(FalseView::inspect, 0))?;
    class.define_method("deconstruct_keys", method!(FalseView::deconstruct_keys, 1))?;

    let class = module.define_class("BooleanView", ruby.class_object())?;
    class.define_method("variant", method!(BooleanView::variant, 0))?;
    class.define_method("inspect", method!(BooleanView::inspect, 0))?;
    class.define_method(
        "deconstruct_keys",
        method!(BooleanView::deconstruct_keys, 1),
    )?;

    let class = module.define_class("IntegerView", ruby.class_object())?;
    class.define_method("minimum", method!(IntegerView::minimum, 0))?;
    class.define_method("maximum", method!(IntegerView::maximum, 0))?;
    class.define_method(
        "exclusive_minimum",
        method!(IntegerView::exclusive_minimum, 0),
    )?;
    class.define_method(
        "exclusive_maximum",
        method!(IntegerView::exclusive_maximum, 0),
    )?;
    class.define_method("multiple_of", method!(IntegerView::multiple_of, 0))?;
    class.define_method("not_multiple_of", method!(IntegerView::not_multiple_of, 0))?;
    class.define_method("inspect", method!(IntegerView::inspect, 0))?;
    class.define_method(
        "deconstruct_keys",
        method!(IntegerView::deconstruct_keys, 1),
    )?;

    let class = module.define_class("NumberView", ruby.class_object())?;
    class.define_method("minimum", method!(NumberView::minimum, 0))?;
    class.define_method("maximum", method!(NumberView::maximum, 0))?;
    class.define_method(
        "exclusive_minimum",
        method!(NumberView::exclusive_minimum, 0),
    )?;
    class.define_method(
        "exclusive_maximum",
        method!(NumberView::exclusive_maximum, 0),
    )?;
    class.define_method("multiple_of", method!(NumberView::multiple_of, 0))?;
    class.define_method("not_multiple_of", method!(NumberView::not_multiple_of, 0))?;
    class.define_method("inspect", method!(NumberView::inspect, 0))?;
    class.define_method("deconstruct_keys", method!(NumberView::deconstruct_keys, 1))?;

    let class = module.define_class("ContentFacetView", ruby.class_object())?;
    class.define_method(
        "content_encoding",
        method!(ContentFacetView::content_encoding, 0),
    )?;
    class.define_method(
        "content_media_type",
        method!(ContentFacetView::content_media_type, 0),
    )?;
    class.define_method(
        "content_schema",
        method!(ContentFacetView::content_schema, 0),
    )?;
    class.define_method("inspect", method!(ContentFacetView::inspect, 0))?;
    class.define_method(
        "deconstruct_keys",
        method!(ContentFacetView::deconstruct_keys, 1),
    )?;

    let class = module.define_class("StringView", ruby.class_object())?;
    class.define_method("min_length", method!(StringView::min_length, 0))?;
    class.define_method("max_length", method!(StringView::max_length, 0))?;
    class.define_method("patterns", method!(StringView::patterns, 0))?;
    class.define_method("not_patterns", method!(StringView::not_patterns, 0))?;
    class.define_method("format", method!(StringView::format, 0))?;
    class.define_method("content", method!(StringView::content, 0))?;
    class.define_method("extended_regex?", method!(StringView::extended_regex_p, 0))?;
    class.define_method("inspect", method!(StringView::inspect, 0))?;
    class.define_method("deconstruct_keys", method!(StringView::deconstruct_keys, 1))?;

    let class = module.define_class("ContainsView", ruby.class_object())?;
    class.define_method("schema", method!(ContainsView::schema, 0))?;
    class.define_method("min_contains", method!(ContainsView::min_contains, 0))?;
    class.define_method("max_contains", method!(ContainsView::max_contains, 0))?;
    class.define_method("inspect", method!(ContainsView::inspect, 0))?;
    class.define_method(
        "deconstruct_keys",
        method!(ContainsView::deconstruct_keys, 1),
    )?;

    let class = module.define_class("ArrayView", ruby.class_object())?;
    class.define_method("prefix", method!(ArrayView::prefix, 0))?;
    class.define_method("tail", method!(ArrayView::tail, 0))?;
    class.define_method("min_items", method!(ArrayView::min_items, 0))?;
    class.define_method("max_items", method!(ArrayView::max_items, 0))?;
    class.define_method("unique_items?", method!(ArrayView::unique_items_p, 0))?;
    class.define_method("repeated_items?", method!(ArrayView::repeated_items_p, 0))?;
    class.define_method("contains", method!(ArrayView::contains, 0))?;
    class.define_method("inspect", method!(ArrayView::inspect, 0))?;
    class.define_method("deconstruct_keys", method!(ArrayView::deconstruct_keys, 1))?;

    let class = module.define_class("RequiredProperty", ruby.class_object())?;
    class.define_method("name", method!(RequiredProperty::name, 0))?;
    class.define_method("inspect", method!(RequiredProperty::inspect, 0))?;
    class.define_method(
        "deconstruct_keys",
        method!(RequiredProperty::deconstruct_keys, 1),
    )?;

    let class = module.define_class("PatternPropertyRequirement", ruby.class_object())?;
    class.define_method("pattern", method!(PatternPropertyRequirement::pattern, 0))?;
    class.define_method("schema", method!(PatternPropertyRequirement::schema, 0))?;
    class.define_method("inspect", method!(PatternPropertyRequirement::inspect, 0))?;
    class.define_method(
        "deconstruct_keys",
        method!(PatternPropertyRequirement::deconstruct_keys, 1),
    )?;

    let class = module.define_class("AdditionalPropertiesRequirement", ruby.class_object())?;
    class.define_method(
        "schema",
        method!(AdditionalPropertiesRequirement::schema, 0),
    )?;
    class.define_method(
        "inspect",
        method!(AdditionalPropertiesRequirement::inspect, 0),
    )?;
    class.define_method(
        "deconstruct_keys",
        method!(AdditionalPropertiesRequirement::deconstruct_keys, 1),
    )?;

    let class = module.define_class("DependentPropertiesRequirement", ruby.class_object())?;
    class.define_method(
        "property",
        method!(DependentPropertiesRequirement::property, 0),
    )?;
    class.define_method(
        "required_properties",
        method!(DependentPropertiesRequirement::required_properties, 0),
    )?;
    class.define_method(
        "inspect",
        method!(DependentPropertiesRequirement::inspect, 0),
    )?;
    class.define_method(
        "deconstruct_keys",
        method!(DependentPropertiesRequirement::deconstruct_keys, 1),
    )?;

    let class = module.define_class("DependentSchemaRequirement", ruby.class_object())?;
    class.define_method("property", method!(DependentSchemaRequirement::property, 0))?;
    class.define_method("schema", method!(DependentSchemaRequirement::schema, 0))?;
    class.define_method("inspect", method!(DependentSchemaRequirement::inspect, 0))?;
    class.define_method(
        "deconstruct_keys",
        method!(DependentSchemaRequirement::deconstruct_keys, 1),
    )?;

    let class = module.define_class("NamedPropertyConstraint", ruby.class_object())?;
    class.define_method("name", method!(NamedPropertyConstraint::name, 0))?;
    class.define_method("schema", method!(NamedPropertyConstraint::schema, 0))?;
    class.define_method("inspect", method!(NamedPropertyConstraint::inspect, 0))?;
    class.define_method(
        "deconstruct_keys",
        method!(NamedPropertyConstraint::deconstruct_keys, 1),
    )?;

    let class = module.define_class("PatternPropertyConstraint", ruby.class_object())?;
    class.define_method("pattern", method!(PatternPropertyConstraint::pattern, 0))?;
    class.define_method("schema", method!(PatternPropertyConstraint::schema, 0))?;
    class.define_method("inspect", method!(PatternPropertyConstraint::inspect, 0))?;
    class.define_method(
        "deconstruct_keys",
        method!(PatternPropertyConstraint::deconstruct_keys, 1),
    )?;

    let class = module.define_class("AdditionalPropertiesConstraint", ruby.class_object())?;
    class.define_method("schema", method!(AdditionalPropertiesConstraint::schema, 0))?;
    class.define_method(
        "inspect",
        method!(AdditionalPropertiesConstraint::inspect, 0),
    )?;
    class.define_method(
        "deconstruct_keys",
        method!(AdditionalPropertiesConstraint::deconstruct_keys, 1),
    )?;

    let class = module.define_class("ObjectView", ruby.class_object())?;
    class.define_method("requirements", method!(ObjectView::requirements, 0))?;
    class.define_method("constraints", method!(ObjectView::constraints, 0))?;
    class.define_method("property_names", method!(ObjectView::property_names, 0))?;
    class.define_method("min_properties", method!(ObjectView::min_properties, 0))?;
    class.define_method("max_properties", method!(ObjectView::max_properties, 0))?;
    class.define_method("inspect", method!(ObjectView::inspect, 0))?;
    class.define_method("deconstruct_keys", method!(ObjectView::deconstruct_keys, 1))?;

    let class = module.define_class("MultiTypeView", ruby.class_object())?;
    class.define_method("types", method!(MultiTypeView::types, 0))?;
    class.define_method("inspect", method!(MultiTypeView::inspect, 0))?;
    class.define_method(
        "deconstruct_keys",
        method!(MultiTypeView::deconstruct_keys, 1),
    )?;

    let class = module.define_class("AllOfView", ruby.class_object())?;
    class.define_method("schemas", method!(AllOfView::schemas, 0))?;
    class.define_method("inspect", method!(AllOfView::inspect, 0))?;
    class.define_method("deconstruct_keys", method!(AllOfView::deconstruct_keys, 1))?;

    let class = module.define_class("AnyOfView", ruby.class_object())?;
    class.define_method("schemas", method!(AnyOfView::schemas, 0))?;
    class.define_method("inspect", method!(AnyOfView::inspect, 0))?;
    class.define_method("deconstruct_keys", method!(AnyOfView::deconstruct_keys, 1))?;

    let class = module.define_class("OneOfView", ruby.class_object())?;
    class.define_method("schemas", method!(OneOfView::schemas, 0))?;
    class.define_method("inspect", method!(OneOfView::inspect, 0))?;
    class.define_method("deconstruct_keys", method!(OneOfView::deconstruct_keys, 1))?;

    let class = module.define_class("NotView", ruby.class_object())?;
    class.define_method("schema", method!(NotView::schema, 0))?;
    class.define_method("inspect", method!(NotView::inspect, 0))?;
    class.define_method("deconstruct_keys", method!(NotView::deconstruct_keys, 1))?;

    let class = module.define_class("IfThenElseView", ruby.class_object())?;
    class.define_method("condition", method!(IfThenElseView::condition, 0))?;
    class.define_method("then_branch", method!(IfThenElseView::then_branch, 0))?;
    class.define_method("else_branch", method!(IfThenElseView::else_branch, 0))?;
    class.define_method("inspect", method!(IfThenElseView::inspect, 0))?;
    class.define_method(
        "deconstruct_keys",
        method!(IfThenElseView::deconstruct_keys, 1),
    )?;

    let class = module.define_class("TypedGroupView", ruby.class_object())?;
    class.define_method("type_name", method!(TypedGroupView::type_name, 0))?;
    class.define_method("body", method!(TypedGroupView::body, 0))?;
    class.define_method("inspect", method!(TypedGroupView::inspect, 0))?;
    class.define_method(
        "deconstruct_keys",
        method!(TypedGroupView::deconstruct_keys, 1),
    )?;

    let class = module.define_class("TypeGuardView", ruby.class_object())?;
    class.define_method("type_name", method!(TypeGuardView::type_name, 0))?;
    class.define_method("body", method!(TypeGuardView::body, 0))?;
    class.define_method("inspect", method!(TypeGuardView::inspect, 0))?;
    class.define_method(
        "deconstruct_keys",
        method!(TypeGuardView::deconstruct_keys, 1),
    )?;

    let class = module.define_class("ConstView", ruby.class_object())?;
    class.define_method("value", method!(ConstView::value, 0))?;
    class.define_method("inspect", method!(ConstView::inspect, 0))?;
    class.define_method("deconstruct_keys", method!(ConstView::deconstruct_keys, 1))?;

    let class = module.define_class("EnumView", ruby.class_object())?;
    class.define_method("values", method!(EnumView::values, 0))?;
    class.define_method("inspect", method!(EnumView::inspect, 0))?;
    class.define_method("deconstruct_keys", method!(EnumView::deconstruct_keys, 1))?;

    let class = module.define_class("ReferenceView", ruby.class_object())?;
    class.define_method("uri", method!(ReferenceView::uri, 0))?;
    class.define_method("inspect", method!(ReferenceView::inspect, 0))?;
    class.define_method(
        "deconstruct_keys",
        method!(ReferenceView::deconstruct_keys, 1),
    )?;

    let class = module.define_class("RecursiveView", ruby.class_object())?;
    class.define_method("uri", method!(RecursiveView::uri, 0))?;
    class.define_method("inspect", method!(RecursiveView::inspect, 0))?;
    class.define_method(
        "deconstruct_keys",
        method!(RecursiveView::deconstruct_keys, 1),
    )?;

    let class = module.define_class("DynamicRefView", ruby.class_object())?;
    class.define_method("name", method!(DynamicRefView::name, 0))?;
    class.define_method("inspect", method!(DynamicRefView::inspect, 0))?;
    class.define_method(
        "deconstruct_keys",
        method!(DynamicRefView::deconstruct_keys, 1),
    )?;

    let class = module.define_class("RawView", ruby.class_object())?;
    class.define_method("schema", method!(RawView::schema, 0))?;
    class.define_method("inspect", method!(RawView::inspect, 0))?;
    class.define_method("deconstruct_keys", method!(RawView::deconstruct_keys, 1))?;

    Ok(())
}
