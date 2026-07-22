use jsonschema::{
    canonical::{CanonicalSchema, CanonicalView, CanonicalizationError},
    JsonType,
};
use magnus::{
    function, method,
    prelude::*,
    scan_args::{get_kwargs, scan_args},
    typed_data,
    value::Lazy,
    DataTypeFunctions, Error, ExceptionClass, RClass, RHash, RModule, Ruby, TryConvert, Value,
};

use crate::{
    options::parse_draft_symbol,
    ser::{to_schema_value, value_to_ruby},
    static_id::define_rb_intern,
};

define_rb_intern!(static CANONICAL_KW_DRAFT: "draft");
define_rb_intern!(static CANONICAL_KW_VALIDATE_FORMATS: "validate_formats");

macro_rules! canonical_error_class {
    ($static_name:ident, $class_name:literal) => {
        static $static_name: Lazy<ExceptionClass> = Lazy::new(|ruby| {
            let json_schema: RModule = ruby
                .class_object()
                .const_get("JSONSchema")
                .expect("JSONSchema");
            let canonical: RModule = json_schema.const_get("Canonical").expect("Canonical");
            let error_class: RClass = canonical.const_get($class_name).expect($class_name);
            let exception_class =
                ExceptionClass::from_value(error_class.as_value()).expect("ExceptionClass");
            // The cached handle is invisible to Ruby's GC; registration pins it across compaction.
            magnus::gc::register_mark_object(exception_class);
            exception_class
        });
    };
}

canonical_error_class!(CANONICALIZATION_ERROR_CLASS, "CanonicalizationError");
canonical_error_class!(INVALID_SCHEMA_TYPE_CLASS, "InvalidSchemaType");

fn canonicalization_error(ruby: &Ruby, error: CanonicalizationError) -> Error {
    if let CanonicalizationError::ValidationError(validation_error) = error {
        return crate::raise_validation_error(ruby, validation_error, None, None);
    }
    let message = error.to_string();
    match error {
        CanonicalizationError::InvalidSchemaType(_) => {
            Error::new(ruby.get_inner(&INVALID_SCHEMA_TYPE_CLASS), message)
        }
        // `ValidationError` returns above; future variants fall back to the base canonical error.
        _ => Error::new(ruby.get_inner(&CANONICALIZATION_ERROR_CLASS), message),
    }
}

#[derive(magnus::TypedData, PartialEq, Eq, Hash)]
#[magnus(
    class = "JSONSchema::Canonical::CanonicalSchema",
    free_immediately,
    size
)]
pub struct RbCanonicalSchema {
    inner: CanonicalSchema,
}

impl DataTypeFunctions for RbCanonicalSchema {}

impl RbCanonicalSchema {
    fn to_json_schema(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        value_to_ruby(ruby, &rb_self.inner.to_json_schema())
    }

    fn draft(ruby: &Ruby, rb_self: &Self) -> Value {
        let name = match rb_self.inner.draft() {
            jsonschema::Draft::Draft4 => "draft4",
            jsonschema::Draft::Draft6 => "draft6",
            jsonschema::Draft::Draft7 => "draft7",
            jsonschema::Draft::Draft201909 => "draft201909",
            _ => "draft202012",
        };
        ruby.sym_new(name).as_value()
    }

    fn kind(ruby: &Ruby, rb_self: &Self) -> Value {
        ruby.sym_new(rb_self.inner.kind().as_str()).as_value()
    }

    fn satisfiable(rb_self: &Self) -> bool {
        rb_self.inner.is_satisfiable()
    }

    fn inspect(ruby: &Ruby, rb_self: &Self) -> String {
        // Bounded: `inspect` runs implicitly (IRB, error messages) and a full
        // `to_json_schema` re-emits the whole document.
        format!(
            "#<JSONSchema::Canonical::CanonicalSchema kind={} draft={}>",
            Self::kind(ruby, rb_self).inspect(),
            Self::draft(ruby, rb_self).inspect()
        )
    }

    fn eq(rb_self: &Self, other: Value) -> bool {
        let Ok(other_ref) = <&RbCanonicalSchema>::try_convert(other) else {
            return false;
        };
        rb_self.inner == other_ref.inner
    }

    fn view(ruby: &Ruby, rb_self: &Self) -> Value {
        match rb_self.inner.view() {
            CanonicalView::MultiType(set) => ruby
                .obj_wrap(MultiTypeView {
                    types: set.iter().map(JsonType::as_str).collect(),
                })
                .as_value(),
            CanonicalView::TypedGroup(group) => ruby
                .obj_wrap(TypedGroupView {
                    type_name: group.ty.as_str(),
                    body: group.body,
                })
                .as_value(),
            CanonicalView::Const(value) => ruby.obj_wrap(ConstView { value }).as_value(),
            CanonicalView::Enum(values) => ruby.obj_wrap(EnumView { values }).as_value(),
            CanonicalView::True => ruby.obj_wrap(TrueView).as_value(),
            CanonicalView::False => ruby.obj_wrap(FalseView).as_value(),
            CanonicalView::String(view) => ruby
                .obj_wrap(StringView {
                    min_length: view.min_length,
                    max_length: view.max_length,
                    patterns: view.patterns,
                    formats: view.formats,
                })
                .as_value(),
            CanonicalView::Integer(view) => ruby
                .obj_wrap(IntegerView {
                    minimum: view.minimum,
                    maximum: view.maximum,
                    multiple_of: view.multiple_of,
                })
                .as_value(),
            CanonicalView::AnyOf(branches) => ruby.obj_wrap(AnyOfView { branches }).as_value(),
            CanonicalView::Raw(schema) => ruby.obj_wrap(RawView { schema }).as_value(),
        }
    }

    fn definitions(ruby: &Ruby, rb_self: &Self) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        for (uri, target) in rb_self.inner.definitions() {
            let wrapped = ruby.obj_wrap(RbCanonicalSchema { inner: target });
            hash.aset(uri, wrapped)?;
        }
        Ok(hash)
    }
}

/// A schema the canonical form does not model structurally, kept verbatim.
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

    fn inspect(ruby: &Ruby, rb_self: &Self) -> Result<String, Error> {
        Ok(format!(
            "#<JSONSchema::Canonical::RawView schema={}>",
            Self::schema(ruby, rb_self)?.inspect()
        ))
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

/// Matches any value.
#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Canonical::TrueView", free_immediately)]
pub struct TrueView;

impl TrueView {
    fn inspect(_rb_self: &Self) -> String {
        "#<JSONSchema::Canonical::TrueView>".to_string()
    }
}

impl DataTypeFunctions for TrueView {}

/// Matches no value.
#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Canonical::FalseView", free_immediately)]
pub struct FalseView;

impl FalseView {
    fn inspect(_rb_self: &Self) -> String {
        "#<JSONSchema::Canonical::FalseView>".to_string()
    }
}

impl DataTypeFunctions for FalseView {}

/// A value matches iff its JSON type is in `types`.
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

    fn inspect(ruby: &Ruby, rb_self: &Self) -> Result<String, Error> {
        Ok(format!(
            "#<JSONSchema::Canonical::MultiTypeView types={}>",
            Self::types(ruby, rb_self)?.inspect()
        ))
    }

    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(ruby.sym_new("types"), Self::types(ruby, rb_self)?)?;
        Ok(hash)
    }
}

/// A value matches iff its JSON type is `type_name` *and* it satisfies `body`.
#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Canonical::TypedGroupView", free_immediately)]
pub struct TypedGroupView {
    type_name: &'static str,
    body: CanonicalSchema,
}

impl DataTypeFunctions for TypedGroupView {}

impl TypedGroupView {
    fn type_name(ruby: &Ruby, rb_self: &Self) -> Value {
        ruby.sym_new(rb_self.type_name).as_value()
    }

    fn body(ruby: &Ruby, rb_self: &Self) -> Value {
        ruby.obj_wrap(RbCanonicalSchema {
            inner: rb_self.body.clone(),
        })
        .as_value()
    }

    fn inspect(ruby: &Ruby, rb_self: &Self) -> String {
        format!(
            "#<JSONSchema::Canonical::TypedGroupView type_name={}>",
            Self::type_name(ruby, rb_self).inspect()
        )
    }

    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(ruby.sym_new("type_name"), Self::type_name(ruby, rb_self))?;
        hash.aset(ruby.sym_new("body"), Self::body(ruby, rb_self))?;
        Ok(hash)
    }
}

/// An absent bound is `nil`; a present one keeps its exact value.
fn bound_to_ruby(ruby: &Ruby, bound: Option<&serde_json::Number>) -> Result<Value, Error> {
    match bound {
        Some(number) => value_to_ruby(ruby, &serde_json::Value::Number(number.clone())),
        None => Ok(ruby.qnil().as_value()),
    }
}

fn strings_to_ruby(ruby: &Ruby, values: &[String]) -> Result<Value, Error> {
    let array = ruby.ary_new_capa(values.len());
    for value in values {
        array.push(ruby.str_new(value).as_value())?;
    }
    Ok(array.as_value())
}

/// A string value within a length window, matching every pattern and format.
#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Canonical::StringView", free_immediately)]
pub struct StringView {
    min_length: Option<serde_json::Number>,
    max_length: Option<serde_json::Number>,
    patterns: Vec<String>,
    formats: Vec<String>,
}

impl DataTypeFunctions for StringView {}

impl StringView {
    fn min_length(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        bound_to_ruby(ruby, rb_self.min_length.as_ref())
    }

    fn max_length(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        bound_to_ruby(ruby, rb_self.max_length.as_ref())
    }

    fn patterns(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        strings_to_ruby(ruby, &rb_self.patterns)
    }

    fn formats(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        strings_to_ruby(ruby, &rb_self.formats)
    }

    fn inspect(ruby: &Ruby, rb_self: &Self) -> Result<String, Error> {
        Ok(format!(
            "#<JSONSchema::Canonical::StringView min_length={} max_length={} patterns={} formats={}>",
            Self::min_length(ruby, rb_self)?.inspect(),
            Self::max_length(ruby, rb_self)?.inspect(),
            Self::patterns(ruby, rb_self)?.inspect(),
            Self::formats(ruby, rb_self)?.inspect()
        ))
    }

    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(ruby.sym_new("min_length"), Self::min_length(ruby, rb_self)?)?;
        hash.aset(ruby.sym_new("max_length"), Self::max_length(ruby, rb_self)?)?;
        hash.aset(ruby.sym_new("patterns"), Self::patterns(ruby, rb_self)?)?;
        hash.aset(ruby.sym_new("formats"), Self::formats(ruby, rb_self)?)?;
        Ok(hash)
    }
}

/// An integer value within a closed interval, optionally a multiple of a divisor.
#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Canonical::IntegerView", free_immediately)]
pub struct IntegerView {
    minimum: Option<serde_json::Number>,
    maximum: Option<serde_json::Number>,
    multiple_of: Option<serde_json::Number>,
}

impl DataTypeFunctions for IntegerView {}

impl IntegerView {
    fn minimum(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        bound_to_ruby(ruby, rb_self.minimum.as_ref())
    }

    fn maximum(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        bound_to_ruby(ruby, rb_self.maximum.as_ref())
    }

    fn multiple_of(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        bound_to_ruby(ruby, rb_self.multiple_of.as_ref())
    }

    fn inspect(ruby: &Ruby, rb_self: &Self) -> Result<String, Error> {
        Ok(format!(
            "#<JSONSchema::Canonical::IntegerView minimum={} maximum={} multiple_of={}>",
            Self::minimum(ruby, rb_self)?.inspect(),
            Self::maximum(ruby, rb_self)?.inspect(),
            Self::multiple_of(ruby, rb_self)?.inspect()
        ))
    }

    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(ruby.sym_new("minimum"), Self::minimum(ruby, rb_self)?)?;
        hash.aset(ruby.sym_new("maximum"), Self::maximum(ruby, rb_self)?)?;
        hash.aset(
            ruby.sym_new("multiple_of"),
            Self::multiple_of(ruby, rb_self)?,
        )?;
        Ok(hash)
    }
}

/// A value matches iff at least one branch matches.
#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Canonical::AnyOfView", free_immediately)]
pub struct AnyOfView {
    branches: Vec<CanonicalSchema>,
}

impl DataTypeFunctions for AnyOfView {}

impl AnyOfView {
    fn branches(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        let array = ruby.ary_new_capa(rb_self.branches.len());
        for branch in &rb_self.branches {
            array.push(
                ruby.obj_wrap(RbCanonicalSchema {
                    inner: branch.clone(),
                })
                .as_value(),
            )?;
        }
        Ok(array.as_value())
    }

    fn inspect(ruby: &Ruby, rb_self: &Self) -> Result<String, Error> {
        let kinds = ruby.ary_new_capa(rb_self.branches.len());
        for branch in &rb_self.branches {
            kinds.push(ruby.sym_new(branch.kind().as_str()).as_value())?;
        }
        Ok(format!(
            "#<JSONSchema::Canonical::AnyOfView branches={}>",
            kinds.as_value().inspect()
        ))
    }

    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(ruby.sym_new("branches"), Self::branches(ruby, rb_self)?)?;
        Ok(hash)
    }
}

/// Exactly one admitted value.
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

    fn inspect(ruby: &Ruby, rb_self: &Self) -> Result<String, Error> {
        Ok(format!(
            "#<JSONSchema::Canonical::ConstView value={}>",
            Self::value(ruby, rb_self)?.inspect()
        ))
    }

    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(ruby.sym_new("value"), Self::value(ruby, rb_self)?)?;
        Ok(hash)
    }
}

/// A sorted, deduplicated finite set of admitted values.
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

    fn inspect(ruby: &Ruby, rb_self: &Self) -> Result<String, Error> {
        Ok(format!(
            "#<JSONSchema::Canonical::EnumView values={}>",
            Self::values(ruby, rb_self)?.inspect()
        ))
    }

    fn deconstruct_keys(ruby: &Ruby, rb_self: &Self, _keys: Value) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        hash.aset(ruby.sym_new("values"), Self::values(ruby, rb_self)?)?;
        Ok(hash)
    }
}

fn canonicalize(ruby: &Ruby, args: &[Value]) -> Result<Value, Error> {
    let parsed = scan_args::<(Value,), (), (), (), _, ()>(args)?;
    let (schema_arg,) = parsed.required;
    let keywords: RHash = parsed.keywords;
    let base_kwargs: magnus::scan_args::KwArgs<(), (Option<Value>, Option<bool>), ()> = get_kwargs(
        keywords,
        &[],
        &[*CANONICAL_KW_DRAFT, *CANONICAL_KW_VALIDATE_FORMATS],
    )?;
    let (draft_val, validate_formats) = base_kwargs.optional;

    let schema_value = to_schema_value(ruby, schema_arg)?;
    let mut options = jsonschema::canonical::options();
    if let Some(draft) = draft_val {
        options = options.with_draft(parse_draft_symbol(ruby, draft)?);
    }
    if let Some(validate_formats) = validate_formats {
        options = options.should_validate_formats(validate_formats);
    }
    options
        .canonicalize(&schema_value)
        .map(|inner| ruby.obj_wrap(RbCanonicalSchema { inner }).as_value())
        .map_err(|error| canonicalization_error(ruby, error))
}

pub(crate) fn init_canonical(ruby: &Ruby, module: &RModule) -> Result<(), Error> {
    let canonical_module = module.define_module("Canonical")?;

    let base_error =
        canonical_module.define_error("CanonicalizationError", ruby.exception_standard_error())?;
    canonical_module.define_error("InvalidSchemaType", base_error)?;

    let canonical_schema = canonical_module.define_class("CanonicalSchema", ruby.class_object())?;
    canonical_schema.define_method(
        "to_json_schema",
        method!(RbCanonicalSchema::to_json_schema, 0),
    )?;
    canonical_schema.define_method("draft", method!(RbCanonicalSchema::draft, 0))?;
    canonical_schema.define_method("kind", method!(RbCanonicalSchema::kind, 0))?;
    canonical_schema.define_method("inspect", method!(RbCanonicalSchema::inspect, 0))?;
    canonical_schema.define_method("==", method!(RbCanonicalSchema::eq, 1))?;
    canonical_schema.define_method(
        "eql?",
        method!(<RbCanonicalSchema as typed_data::IsEql>::is_eql, 1),
    )?;
    canonical_schema.define_method(
        "hash",
        method!(<RbCanonicalSchema as typed_data::Hash>::hash, 0),
    )?;
    canonical_schema.define_method("view", method!(RbCanonicalSchema::view, 0))?;
    canonical_schema.define_method("definitions", method!(RbCanonicalSchema::definitions, 0))?;
    canonical_schema.define_method("satisfiable?", method!(RbCanonicalSchema::satisfiable, 0))?;

    let true_view = canonical_module.define_class("TrueView", ruby.class_object())?;
    true_view.define_method("inspect", method!(TrueView::inspect, 0))?;
    let false_view = canonical_module.define_class("FalseView", ruby.class_object())?;
    false_view.define_method("inspect", method!(FalseView::inspect, 0))?;

    let multi_type_view = canonical_module.define_class("MultiTypeView", ruby.class_object())?;
    multi_type_view.define_method("types", method!(MultiTypeView::types, 0))?;
    multi_type_view.define_method("inspect", method!(MultiTypeView::inspect, 0))?;
    multi_type_view.define_method(
        "deconstruct_keys",
        method!(MultiTypeView::deconstruct_keys, 1),
    )?;

    let typed_group_view = canonical_module.define_class("TypedGroupView", ruby.class_object())?;
    typed_group_view.define_method("type_name", method!(TypedGroupView::type_name, 0))?;
    typed_group_view.define_method("body", method!(TypedGroupView::body, 0))?;
    typed_group_view.define_method("inspect", method!(TypedGroupView::inspect, 0))?;
    typed_group_view.define_method(
        "deconstruct_keys",
        method!(TypedGroupView::deconstruct_keys, 1),
    )?;

    let const_view = canonical_module.define_class("ConstView", ruby.class_object())?;
    const_view.define_method("value", method!(ConstView::value, 0))?;
    const_view.define_method("inspect", method!(ConstView::inspect, 0))?;
    const_view.define_method("deconstruct_keys", method!(ConstView::deconstruct_keys, 1))?;

    let enum_view = canonical_module.define_class("EnumView", ruby.class_object())?;
    enum_view.define_method("values", method!(EnumView::values, 0))?;
    enum_view.define_method("inspect", method!(EnumView::inspect, 0))?;
    enum_view.define_method("deconstruct_keys", method!(EnumView::deconstruct_keys, 1))?;

    let string_view = canonical_module.define_class("StringView", ruby.class_object())?;
    string_view.define_method("min_length", method!(StringView::min_length, 0))?;
    string_view.define_method("max_length", method!(StringView::max_length, 0))?;
    string_view.define_method("patterns", method!(StringView::patterns, 0))?;
    string_view.define_method("formats", method!(StringView::formats, 0))?;
    string_view.define_method("inspect", method!(StringView::inspect, 0))?;
    string_view.define_method("deconstruct_keys", method!(StringView::deconstruct_keys, 1))?;

    let integer_view = canonical_module.define_class("IntegerView", ruby.class_object())?;
    integer_view.define_method("minimum", method!(IntegerView::minimum, 0))?;
    integer_view.define_method("maximum", method!(IntegerView::maximum, 0))?;
    integer_view.define_method("multiple_of", method!(IntegerView::multiple_of, 0))?;
    integer_view.define_method("inspect", method!(IntegerView::inspect, 0))?;
    integer_view.define_method(
        "deconstruct_keys",
        method!(IntegerView::deconstruct_keys, 1),
    )?;

    let any_of_view = canonical_module.define_class("AnyOfView", ruby.class_object())?;
    any_of_view.define_method("branches", method!(AnyOfView::branches, 0))?;
    any_of_view.define_method("inspect", method!(AnyOfView::inspect, 0))?;
    any_of_view.define_method("deconstruct_keys", method!(AnyOfView::deconstruct_keys, 1))?;

    let raw_view = canonical_module.define_class("RawView", ruby.class_object())?;
    raw_view.define_method("schema", method!(RawView::schema, 0))?;
    raw_view.define_method("inspect", method!(RawView::inspect, 0))?;
    raw_view.define_method("deconstruct_keys", method!(RawView::deconstruct_keys, 1))?;

    let json_module = canonical_module.define_module("JSON")?;
    json_module
        .define_singleton_method("to_string", function!(crate::canonical_json_to_string, 1))?;

    module.define_singleton_method("canonicalize", function!(canonicalize, -1))?;

    Ok(())
}
