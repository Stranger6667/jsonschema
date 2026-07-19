use jsonschema::canonical::{CanonicalKind, CanonicalSchema, CanonicalView, CanonicalizationError};
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

fn kind_label(kind: CanonicalKind) -> &'static str {
    match kind {
        CanonicalKind::Raw => "raw",
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
        ruby.sym_new(kind_label(rb_self.inner.kind())).as_value()
    }

    fn inspect(rb_self: &Self) -> String {
        // Bounded: `inspect` runs implicitly (IRB, error messages) and a full
        // `to_json_schema` re-emits the whole document.
        format!(
            "#<JSONSchema::Canonical::CanonicalSchema kind={} draft={:?}>",
            kind_label(rb_self.inner.kind()),
            rb_self.inner.draft()
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
            CanonicalView::Raw(schema) => ruby.obj_wrap(RawView { schema }).as_value(),
            other => unreachable!("unsupported canonical view: {other:?}"),
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
