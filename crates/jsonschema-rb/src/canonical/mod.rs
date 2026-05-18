pub(crate) mod view;

use jsonschema::{
    canonical::{CanonicalSchema, CanonicalizationError},
    Draft,
};
use magnus::{
    function, method,
    prelude::*,
    scan_args::{get_kwargs, scan_args},
    typed_data,
    value::Lazy,
    DataTypeFunctions, Error, Exception, ExceptionClass, RClass, RHash, RModule, RObject, Ruby,
    TryConvert, Value,
};

use crate::{
    options::{parse_draft_symbol, FancyRegexOptions, RegexOptions, KW_VALIDATE_FORMATS},
    registry::{Registry, RetrieverBuildRootGuard},
    retriever::make_retriever,
    ser::{to_schema_value, value_to_ruby},
    static_id::{define_rb_intern, StaticId},
};

fn extract_and_delete(hash: &RHash, key: StaticId) -> Result<Option<Value>, Error> {
    let value: Option<Value> = hash.delete(key.to_symbol())?;
    match value {
        Some(value) if value.is_nil() => Ok(None),
        other => Ok(other),
    }
}

define_rb_intern!(static CANONICAL_KW_DRAFT: "draft");
define_rb_intern!(static CANONICAL_KW_RETRIEVER: "retriever");
define_rb_intern!(static CANONICAL_KW_REGISTRY: "registry");
define_rb_intern!(static CANONICAL_KW_BASE_URI: "base_uri");
define_rb_intern!(static CANONICAL_KW_PATTERN_OPTIONS: "pattern_options");
define_rb_intern!(static CANONICAL_KW_INLINE_BUDGET: "inline_budget");
define_rb_intern!(static CANONICAL_ALLOCATE: "allocate");
define_rb_intern!(static CANONICAL_INITIALIZE: "initialize");
define_rb_intern!(static CANONICAL_AT_LOCATION: "@location");

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
canonical_error_class!(INVALID_JSON_VALUE_CLASS, "InvalidJsonValue");
canonical_error_class!(UNGUARDED_RECURSION_CLASS, "UnguardedRecursion");
canonical_error_class!(INFINITE_RECURSION_CLASS, "InfiniteRecursion");
canonical_error_class!(INVALID_PATTERN_CLASS, "InvalidPattern");

fn canonicalization_error(ruby: &Ruby, error: CanonicalizationError) -> Error {
    if let CanonicalizationError::ValidationError(validation_error) = error {
        return crate::raise_validation_error(ruby, validation_error, None, None);
    }
    let message = error.to_string();
    match error {
        CanonicalizationError::InvalidSchemaType(_) => {
            Error::new(ruby.get_inner(&INVALID_SCHEMA_TYPE_CLASS), message)
        }
        CanonicalizationError::InvalidJsonValue(_) => {
            Error::new(ruby.get_inner(&INVALID_JSON_VALUE_CLASS), message)
        }
        CanonicalizationError::UnguardedRecursion(_) => {
            Error::new(ruby.get_inner(&UNGUARDED_RECURSION_CLASS), message)
        }
        CanonicalizationError::InfiniteRecursion(_) => {
            Error::new(ruby.get_inner(&INFINITE_RECURSION_CLASS), message)
        }
        CanonicalizationError::InvalidPattern { pointer, .. } => {
            let error_class = ruby.get_inner(&INVALID_PATTERN_CLASS);
            if let Ok(exception) = error_class.funcall::<_, _, RObject>(*CANONICAL_ALLOCATE, ()) {
                let _ =
                    exception.funcall::<_, _, Value>(*CANONICAL_INITIALIZE, (message.as_str(),));
                let _ = exception.ivar_set(*CANONICAL_AT_LOCATION, pointer.as_str());
                if let Some(exception) = Exception::from_value(exception.as_value()) {
                    return exception.into();
                }
            }
            Error::new(error_class, message)
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
    pub(crate) inner: CanonicalSchema,
}

impl DataTypeFunctions for RbCanonicalSchema {}

impl RbCanonicalSchema {
    fn to_json_schema(ruby: &Ruby, rb_self: &Self) -> Result<Value, Error> {
        value_to_ruby(ruby, &rb_self.inner.to_json_schema())
    }

    fn satisfiable(rb_self: &Self) -> bool {
        rb_self.inner.is_satisfiable()
    }

    fn draft(ruby: &Ruby, rb_self: &Self) -> Value {
        let name = match rb_self.inner.draft() {
            Draft::Draft4 => "draft4",
            Draft::Draft6 => "draft6",
            Draft::Draft7 => "draft7",
            Draft::Draft201909 => "draft201909",
            _ => "draft202012",
        };
        ruby.sym_new(name).as_value()
    }

    fn kind(ruby: &Ruby, rb_self: &Self) -> Value {
        ruby.sym_new(rb_self.inner.kind().as_str()).as_value()
    }

    fn inspect(rb_self: &Self) -> String {
        format!(
            "#<JSONSchema::Canonical::CanonicalSchema {}>",
            rb_self.inner.to_json_schema()
        )
    }

    fn eq(rb_self: &Self, other: Value) -> bool {
        let Ok(other_ref) = <&RbCanonicalSchema>::try_convert(other) else {
            return false;
        };
        rb_self.inner == other_ref.inner
    }

    fn view(ruby: &Ruby, rb_self: &Self) -> Value {
        view::view_to_ruby(ruby, rb_self.inner.view())
    }

    fn definitions(ruby: &Ruby, rb_self: &Self) -> Result<RHash, Error> {
        let hash = ruby.hash_new();
        for (uri, target) in rb_self.inner.definitions() {
            let wrapped = ruby.obj_wrap(RbCanonicalSchema { inner: target });
            hash.aset(uri, wrapped)?;
        }
        Ok(hash)
    }

    fn intersect(ruby: &Ruby, rb_self: &Self, other: Value) -> Result<Value, Error> {
        let other_ref = <&RbCanonicalSchema>::try_convert(other)?;
        let result = rb_self.inner.intersect(&other_ref.inner);
        Ok(ruby
            .obj_wrap(RbCanonicalSchema { inner: result })
            .as_value())
    }

    fn union(ruby: &Ruby, rb_self: &Self, other: Value) -> Result<Value, Error> {
        let other_ref = <&RbCanonicalSchema>::try_convert(other)?;
        let result = rb_self.inner.union(&other_ref.inner);
        Ok(ruby
            .obj_wrap(RbCanonicalSchema { inner: result })
            .as_value())
    }

    fn negate(ruby: &Ruby, rb_self: &Self) -> Value {
        ruby.obj_wrap(RbCanonicalSchema {
            inner: rb_self.inner.negate(),
        })
        .as_value()
    }

    fn subtract(ruby: &Ruby, rb_self: &Self, other: Value) -> Result<Value, Error> {
        let other_ref = <&RbCanonicalSchema>::try_convert(other)?;
        let result = rb_self.inner.subtract(&other_ref.inner);
        Ok(ruby
            .obj_wrap(RbCanonicalSchema { inner: result })
            .as_value())
    }

    fn subschema_of(ruby: &Ruby, rb_self: &Self, other: Value) -> Result<Value, Error> {
        let other_ref = <&RbCanonicalSchema>::try_convert(other)?;
        match rb_self.inner.is_subschema_of(&other_ref.inner) {
            Some(true) => Ok(ruby.qtrue().as_value()),
            Some(false) => Ok(ruby.qfalse().as_value()),
            None => Ok(ruby.qnil().as_value()),
        }
    }
}

fn canonicalize(ruby: &Ruby, args: &[Value]) -> Result<Value, Error> {
    let parsed = scan_args::<(Value,), (), (), (), _, ()>(args)?;
    let (schema_arg,) = parsed.required;
    let keywords: RHash = parsed.keywords;

    let pattern_options_val = extract_and_delete(&keywords, *CANONICAL_KW_PATTERN_OPTIONS)?;

    let keyword_ids = [
        *CANONICAL_KW_DRAFT,
        *KW_VALIDATE_FORMATS,
        *CANONICAL_KW_RETRIEVER,
        *CANONICAL_KW_REGISTRY,
        *CANONICAL_KW_BASE_URI,
        *CANONICAL_KW_INLINE_BUDGET,
    ];
    #[allow(clippy::type_complexity)]
    let base_kwargs: magnus::scan_args::KwArgs<
        (),
        (
            Option<Value>,
            Option<bool>,
            Option<Value>,
            Option<Value>,
            Option<String>,
            Option<usize>,
        ),
        (),
    > = get_kwargs(keywords, &[], &keyword_ids)?;
    let (draft_val, validate_formats, retriever_val, registry_val, base_uri, inline_budget) =
        base_kwargs.optional;
    let retriever_was_provided = retriever_val.is_some();

    let schema_value = to_schema_value(ruby, schema_arg)?;
    let mut options = jsonschema::canonical::options();

    if let Some(budget) = inline_budget {
        options = options.with_inline_budget(budget);
    }
    if let Some(draft) = draft_val {
        options = options.with_draft(parse_draft_symbol(ruby, draft)?);
    }
    if let Some(validate_formats) = validate_formats {
        options = options.should_validate_formats(validate_formats);
    }
    let mut retriever_root: Option<Value> = None;
    if let Some(retriever_arg) = retriever_val {
        if let Some(retriever) = make_retriever(ruby, retriever_arg)? {
            retriever_root = Some(retriever_arg);
            options = options.with_retriever(retriever);
        }
    }
    if let Some(registry_arg) = registry_val {
        if !registry_arg.is_nil() {
            let registry: &Registry = TryConvert::try_convert(registry_arg)?;
            options = options.with_registry(registry.inner.as_ref());
            if !retriever_was_provided {
                if let Some(registry_retriever_value) = registry.retriever_value(ruby) {
                    if let Some(retriever) = make_retriever(ruby, registry_retriever_value)? {
                        retriever_root = Some(registry_retriever_value);
                        options = options.with_retriever(retriever);
                    }
                }
            }
        }
    }
    if let Some(uri) = base_uri {
        options = options.with_base_uri(uri);
    }
    if let Some(pattern_options) = pattern_options_val {
        if let Ok(fancy) = <&FancyRegexOptions>::try_convert(pattern_options) {
            let mut compiled = jsonschema::PatternOptions::fancy_regex();
            if let Some(limit) = fancy.backtrack_limit {
                compiled = compiled.backtrack_limit(limit);
            }
            if let Some(limit) = fancy.size_limit {
                compiled = compiled.size_limit(limit);
            }
            if let Some(limit) = fancy.dfa_size_limit {
                compiled = compiled.dfa_size_limit(limit);
            }
            options = options.with_pattern_options(&compiled);
        } else if let Ok(regex) = <&RegexOptions>::try_convert(pattern_options) {
            let mut compiled = jsonschema::PatternOptions::regex();
            if let Some(limit) = regex.size_limit {
                compiled = compiled.size_limit(limit);
            }
            if let Some(limit) = regex.dfa_size_limit {
                compiled = compiled.dfa_size_limit(limit);
            }
            options = options.with_pattern_options(&compiled);
        } else {
            return Err(Error::new(
                ruby.exception_type_error(),
                "pattern_options must be a RegexOptions or FancyRegexOptions instance",
            ));
        }
    }

    // The retriever proc is reachable only through the options `Arc`, invisible to the GC;
    // root it for the duration of the call.
    let _retriever_build_guard = RetrieverBuildRootGuard::new(retriever_root);
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
    canonical_module.define_error("InvalidJsonValue", base_error)?;
    canonical_module.define_error("UnguardedRecursion", base_error)?;
    canonical_module.define_error("InfiniteRecursion", base_error)?;
    canonical_module.define_error("InvalidPattern", base_error)?;

    let canonical_schema = canonical_module.define_class("CanonicalSchema", ruby.class_object())?;
    canonical_schema.define_method(
        "to_json_schema",
        method!(RbCanonicalSchema::to_json_schema, 0),
    )?;
    canonical_schema.define_method("satisfiable?", method!(RbCanonicalSchema::satisfiable, 0))?;
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
    canonical_schema.define_method("intersect", method!(RbCanonicalSchema::intersect, 1))?;
    canonical_schema.define_method("union", method!(RbCanonicalSchema::union, 1))?;
    canonical_schema.define_method("negate", method!(RbCanonicalSchema::negate, 0))?;
    canonical_schema.define_method("subtract", method!(RbCanonicalSchema::subtract, 1))?;
    canonical_schema.define_method("subschema_of?", method!(RbCanonicalSchema::subschema_of, 1))?;

    let json_module = canonical_module.define_module("JSON")?;
    json_module
        .define_singleton_method("to_string", function!(crate::canonical_json_to_string, 1))?;

    module.define_singleton_method("canonicalize", function!(canonicalize, -1))?;

    view::register_view_classes(ruby, &canonical_module)?;
    Ok(())
}
