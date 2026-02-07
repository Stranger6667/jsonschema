//! Ruby bindings for the jsonschema crate.
#![allow(unreachable_pub)]
#![allow(clippy::trivially_copy_pass_by_ref)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::unused_self)]
#![allow(clippy::struct_field_names)]

mod error_kind;
mod evaluation;
mod options;
mod registry;
mod retriever;
mod ser;
mod static_id;

use jsonschema::{paths::LocationSegment, ValidationOptions};
use magnus::{
    function,
    gc::{register_address, register_mark_object, unregister_address},
    method,
    prelude::*,
    scan_args::scan_args,
    value::{Lazy, ReprValue},
    DataTypeFunctions, Error, Exception, ExceptionClass, RClass, RModule, RObject, Ruby, Value,
};
use referencing::unescape_segment;
use std::{
    cell::RefCell,
    panic::{self, AssertUnwindSafe},
    sync::Arc,
};

use crate::{
    error_kind::ValidationErrorKind,
    evaluation::Evaluation,
    options::{
        extract_evaluate_kwargs, extract_kwargs, extract_kwargs_no_draft, make_options_from_kwargs,
        parse_draft_symbol, CallbackRoots, CompilationRoots, CompilationRootsRef, ExtractedKwargs,
        ParsedOptions,
    },
    registry::Registry,
    retriever::{retriever_error_message, RubyRetriever},
    ser::{to_schema_value, to_value},
    static_id::define_rb_intern,
};

// Report Rust heap usage to Ruby GC so it can account for native memory pressure.
rb_sys::set_global_tracking_allocator!();

define_rb_intern!(static ID_ALLOCATE: "allocate");
define_rb_intern!(static ID_AT_MESSAGE: "@message");
define_rb_intern!(static ID_AT_VERBOSE_MESSAGE: "@verbose_message");
define_rb_intern!(static ID_AT_INSTANCE_PATH: "@instance_path");
define_rb_intern!(static ID_AT_SCHEMA_PATH: "@schema_path");
define_rb_intern!(static ID_AT_EVALUATION_PATH: "@evaluation_path");
define_rb_intern!(static ID_AT_INSTANCE_PATH_POINTER: "@instance_path_pointer");
define_rb_intern!(static ID_AT_SCHEMA_PATH_POINTER: "@schema_path_pointer");
define_rb_intern!(static ID_AT_EVALUATION_PATH_POINTER: "@evaluation_path_pointer");
define_rb_intern!(static ID_AT_KIND: "@kind");
define_rb_intern!(static ID_AT_INSTANCE: "@instance");

define_rb_intern!(static ID_SYM_MESSAGE: "message");
define_rb_intern!(static ID_SYM_VERBOSE_MESSAGE: "verbose_message");
define_rb_intern!(static ID_SYM_INSTANCE_PATH: "instance_path");
define_rb_intern!(static ID_SYM_SCHEMA_PATH: "schema_path");
define_rb_intern!(static ID_SYM_EVALUATION_PATH: "evaluation_path");
define_rb_intern!(static ID_SYM_KIND: "kind");
define_rb_intern!(static ID_SYM_INSTANCE: "instance");
define_rb_intern!(static ID_SYM_INSTANCE_PATH_POINTER: "instance_path_pointer");
define_rb_intern!(static ID_SYM_SCHEMA_PATH_POINTER: "schema_path_pointer");
define_rb_intern!(static ID_SYM_EVALUATION_PATH_POINTER: "evaluation_path_pointer");

struct BuiltValidator {
    validator: jsonschema::Validator,
    callback_roots: CallbackRoots,
    compilation_roots: CompilationRootsRef,
}

fn build_validator(
    ruby: &Ruby,
    options: ValidationOptions,
    retriever: Option<RubyRetriever>,
    callback_roots: CallbackRoots,
    compilation_roots: Arc<CompilationRoots>,
    schema: &serde_json::Value,
) -> Result<BuiltValidator, Error> {
    let validator = match retriever {
        Some(ret) => options.with_retriever(ret).build(schema),
        None => options.build(schema),
    }
    .map_err(|error| {
        if let jsonschema::error::ValidationErrorKind::Referencing(err) = error.kind() {
            if let Some(message) = retriever_error_message(err) {
                Error::new(ruby.exception_arg_error(), message)
            } else {
                referencing_error(ruby, err.to_string())
            }
        } else {
            Error::new(ruby.exception_arg_error(), error.to_string())
        }
    })?;

    Ok(BuiltValidator {
        validator,
        callback_roots,
        compilation_roots,
    })
}

/// RAII guard that registers Ruby callback values as GC roots for the duration
/// of a one-off validation call (module-level `valid?`, `validate!`, etc.).
///
/// Persistent `Validator` instances protect their callbacks via the GC mark phase
/// (see `Validator::mark_callback_roots`). One-off calls have no such wrapper, so
/// this guard calls `register_address` on construction and `unregister_address` on
/// drop to keep callbacks alive while validation runs.
struct CallbackRootGuard {
    roots: Vec<Value>,
}

impl CallbackRootGuard {
    fn new(ruby: &Ruby, callback_roots: &CallbackRoots) -> Self {
        let roots = {
            let roots_guard = match callback_roots.lock() {
                Ok(roots) => roots,
                Err(poisoned) => poisoned.into_inner(),
            };
            roots_guard
                .iter()
                .map(|root| ruby.get_inner(*root))
                .collect::<Vec<_>>()
        };
        // We do not mutate `roots` after this point, so references used for GC address
        // registration remain valid for the lifetime of the guard.
        for root in &roots {
            register_address(root);
        }

        Self { roots }
    }
}

impl Drop for CallbackRootGuard {
    fn drop(&mut self) {
        for root in &self.roots {
            unregister_address(root);
        }
    }
}

fn build_parsed_options(
    ruby: &Ruby,
    kw: ExtractedKwargs,
    draft_override: Option<jsonschema::Draft>,
) -> Result<ParsedOptions, Error> {
    let (
        draft_val,
        validate_formats,
        ignore_unknown_formats,
        mask,
        base_uri,
        retriever,
        formats,
        keywords,
        registry,
    ) = kw.base;
    let parsed_draft = match draft_val {
        Some(val) => Some(parse_draft_symbol(ruby, val)?),
        None => None,
    };
    make_options_from_kwargs(
        ruby,
        draft_override.or(parsed_draft),
        validate_formats,
        ignore_unknown_formats,
        mask,
        base_uri,
        retriever,
        formats,
        keywords,
        registry,
        kw.pattern_options,
        kw.email_options,
        kw.http_options,
    )
}

thread_local! {
    static LAST_CALLBACK_ERROR: RefCell<Option<Error>> = const { RefCell::new(None) };
    /// When `true`, the custom panic hook suppresses output (inside `catch_unwind` blocks).
    static SUPPRESS_PANIC_OUTPUT: RefCell<bool> = const { RefCell::new(false) };
}

static VALIDATION_ERROR_CLASS: Lazy<ExceptionClass> = Lazy::new(|ruby| {
    let module: RModule = ruby
        .class_object()
        .const_get("JSONSchema")
        .expect("JSONSchema module must be defined before native extension is used");
    let cls: RClass = module
        .const_get("ValidationError")
        .expect("JSONSchema::ValidationError must be defined before native extension is used");
    let exc_cls = ExceptionClass::from_value(cls.as_value())
        .expect("JSONSchema::ValidationError must be an exception class");
    register_mark_object(exc_cls);
    exc_cls
});

static REFERENCING_ERROR_CLASS: Lazy<ExceptionClass> = Lazy::new(|ruby| {
    let module: RModule = ruby
        .class_object()
        .const_get("JSONSchema")
        .expect("JSONSchema module must be defined before native extension is used");
    let cls: RClass = module
        .const_get("ReferencingError")
        .expect("JSONSchema::ReferencingError must be defined before native extension is used");
    let exc_cls = ExceptionClass::from_value(cls.as_value())
        .expect("JSONSchema::ReferencingError must be an exception class");
    register_mark_object(exc_cls);
    exc_cls
});

struct StringWriter<'a>(&'a mut String);

#[allow(unsafe_code)]
impl std::io::Write for StringWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // SAFETY: `serde_json` always produces valid UTF-8
        self.0
            .push_str(unsafe { std::str::from_utf8_unchecked(buf) });
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Build a verbose error message with schema path, instance path, and instance value.
fn build_verbose_message(
    mut message: String,
    schema_path: &jsonschema::paths::Location,
    instance_path: &jsonschema::paths::Location,
    root_instance: Option<&serde_json::Value>,
    failing_instance: &serde_json::Value,
    mask: Option<&str>,
) -> String {
    let schema_path_str = schema_path.as_str();
    let instance_path_str = instance_path.as_str();

    let estimated_addition =
        150 + schema_path_str.len() + instance_path_str.len() + mask.map_or(100, str::len); // Mask length or ~100 for JSON serialization

    message.reserve(estimated_addition);
    message.push_str("\n\nFailed validating");

    let is_index_segment =
        |segment: &str| segment.bytes().all(|b| b.is_ascii_digit()) && !segment.is_empty();
    let is_schema_property_map = |segment: Option<&str>| {
        matches!(
            segment,
            Some(
                "properties"
                    | "patternProperties"
                    | "dependentSchemas"
                    | "$defs"
                    | "definitions"
                    | "dependencies",
            )
        )
    };
    let push_segment = |m: &mut String, segment: &str, is_index: bool| {
        if is_index {
            m.push_str(segment);
        } else {
            m.push('"');
            m.push_str(segment);
            m.push('"');
        }
    };

    let mut schema_segments = Vec::new();
    let mut previous_schema_segment: Option<String> = None;
    for segment in schema_path_str.split('/').skip(1) {
        let segment = unescape_segment(segment);
        let segment = segment.as_ref();
        let is_index = is_index_segment(segment)
            && !is_schema_property_map(previous_schema_segment.as_deref());
        schema_segments.push((segment.to_owned(), is_index));
        previous_schema_segment = Some(segment.to_owned());
    }

    if let Some((last, rest)) = schema_segments.split_last() {
        message.push(' ');
        push_segment(&mut message, &last.0, last.1);
        message.push_str(" in schema");
        for (segment, is_index) in rest {
            message.push('[');
            push_segment(&mut message, segment, *is_index);
            message.push(']');
        }
    } else {
        message.push_str(" in schema");
    }

    message.push_str("\n\nOn instance");
    let mut current = root_instance;
    for segment in instance_path_str.split('/').skip(1) {
        let segment = unescape_segment(segment);
        let segment = segment.as_ref();
        let is_index = match current {
            Some(serde_json::Value::Object(_)) => false,
            _ => is_index_segment(segment),
        };
        message.push('[');
        push_segment(&mut message, segment, is_index);
        message.push(']');

        current = match (current, is_index) {
            (Some(serde_json::Value::Array(values)), true) => segment
                .parse::<usize>()
                .ok()
                .and_then(|idx| values.get(idx)),
            (Some(serde_json::Value::Object(values)), false) => values.get(segment),
            _ => None,
        };
    }
    message.push_str(":\n    ");

    if let Some(mask) = mask {
        message.push_str(mask);
    } else {
        let mut writer = StringWriter(&mut message);
        serde_json::to_writer(&mut writer, failing_instance).expect("Failed to serialize JSON");
    }

    message
}

/// Compute the display message for a validation error, respecting the mask option.
fn error_message(error: &jsonschema::ValidationError<'_>, mask: Option<&str>) -> String {
    if let Some(mask) = mask {
        error.masked_with(mask).to_string()
    } else {
        error.to_string()
    }
}

/// Convert a jsonschema `ValidationError` to a Ruby `ValidationError`.
fn into_ruby_error(
    ruby: &Ruby,
    error: jsonschema::ValidationError<'_>,
    root_instance: Option<&serde_json::Value>,
    message: &str,
    mask: Option<&str>,
) -> Result<Value, Error> {
    let rb_message = ruby.into_value(message);
    let verbose_message = build_verbose_message(
        message.to_owned(),
        error.schema_path(),
        error.instance_path(),
        root_instance,
        error.instance(),
        mask,
    );

    let (instance, kind, instance_path, schema_path, evaluation_path) = error.into_parts();

    let instance_path_ptr = ruby.into_value(instance_path.as_str());
    let schema_path_ptr = ruby.into_value(schema_path.as_str());
    let evaluation_path_ptr = ruby.into_value(evaluation_path.as_str());

    let into_path_segment = |segment: LocationSegment<'_>| -> Value {
        match segment {
            LocationSegment::Property(property) => ruby.into_value(property.as_ref()),
            LocationSegment::Index(idx) => ruby.into_value(idx),
        }
    };

    let kind_obj = ValidationErrorKind::new(ruby, &kind, mask)?;
    let rb_instance = ser::value_to_ruby(ruby, instance.as_ref())?;

    let exc_class = ruby.get_inner(&VALIDATION_ERROR_CLASS);

    let exc: RObject = exc_class.funcall(*ID_ALLOCATE, ())?;

    exc.ivar_set(*ID_AT_MESSAGE, rb_message)?;
    exc.ivar_set(
        *ID_AT_VERBOSE_MESSAGE,
        ruby.into_value(verbose_message.as_str()),
    )?;
    exc.ivar_set(
        *ID_AT_INSTANCE_PATH,
        ruby.ary_from_iter(instance_path.into_iter().map(&into_path_segment)),
    )?;
    exc.ivar_set(
        *ID_AT_SCHEMA_PATH,
        ruby.ary_from_iter(schema_path.into_iter().map(&into_path_segment)),
    )?;
    exc.ivar_set(
        *ID_AT_EVALUATION_PATH,
        ruby.ary_from_iter(evaluation_path.into_iter().map(&into_path_segment)),
    )?;
    exc.ivar_set(*ID_AT_INSTANCE_PATH_POINTER, instance_path_ptr)?;
    exc.ivar_set(*ID_AT_SCHEMA_PATH_POINTER, schema_path_ptr)?;
    exc.ivar_set(*ID_AT_EVALUATION_PATH_POINTER, evaluation_path_ptr)?;
    exc.ivar_set(*ID_AT_KIND, ruby.into_value(kind_obj))?;
    exc.ivar_set(*ID_AT_INSTANCE, rb_instance)?;

    Ok(exc.as_value())
}

/// Convert a jsonschema `ValidationError` into a Ruby `ValidationError` value.
fn to_ruby_error_value(
    ruby: &Ruby,
    error: jsonschema::ValidationError<'_>,
    root_instance: Option<&serde_json::Value>,
    mask: Option<&str>,
) -> Result<Value, Error> {
    let message = error_message(&error, mask);
    into_ruby_error(ruby, error, root_instance, &message, mask)
}

fn referencing_error(ruby: &Ruby, message: String) -> Error {
    let exc_class = ruby.get_inner(&REFERENCING_ERROR_CLASS);
    Error::new(exc_class, message)
}

fn raise_validation_error(
    ruby: &Ruby,
    error: jsonschema::ValidationError<'_>,
    root_instance: Option<&serde_json::Value>,
    mask: Option<&str>,
) -> Error {
    let message = error_message(&error, mask);
    match into_ruby_error(ruby, error, root_instance, &message, mask) {
        Ok(exc_value) => {
            if let Some(exc) = Exception::from_value(exc_value) {
                exc.into()
            } else {
                let exc_class = ruby.get_inner(&VALIDATION_ERROR_CLASS);
                Error::new(exc_class, message)
            }
        }
        Err(e) => e,
    }
}

/// RAII guard that sets `SUPPRESS_PANIC_OUTPUT` to `true` on creation and
/// resets it to `false` on drop, ensuring the flag is always restored even
/// if `catch_unwind` itself encounters a double-panic.
struct SuppressPanicGuard;

impl SuppressPanicGuard {
    fn new() -> Self {
        SUPPRESS_PANIC_OUTPUT.with(|flag| *flag.borrow_mut() = true);
        SuppressPanicGuard
    }
}

impl Drop for SuppressPanicGuard {
    fn drop(&mut self) {
        SUPPRESS_PANIC_OUTPUT.with(|flag| *flag.borrow_mut() = false);
    }
}

/// Run a closure with panic output suppressed, catching panics.
fn catch_unwind_silent<F, R>(f: F) -> Result<R, Box<dyn std::any::Any + Send>>
where
    F: FnOnce() -> R + panic::UnwindSafe,
{
    let _guard = SuppressPanicGuard::new();
    panic::catch_unwind(f)
}

#[allow(clippy::needless_pass_by_value)]
fn handle_callback_panic(ruby: &Ruby, err: Box<dyn std::any::Any + Send>) -> Error {
    LAST_CALLBACK_ERROR.with(|last| {
        if let Some(err) = last.borrow_mut().take() {
            err
        } else {
            let msg = if let Some(s) = err.downcast_ref::<&str>() {
                format!("Validation callback panicked: {s}")
            } else if let Some(s) = err.downcast_ref::<String>() {
                format!("Validation callback panicked: {s}")
            } else {
                "Validation callback panicked".to_string()
            };
            Error::new(ruby.exception_runtime_error(), msg)
        }
    })
}

#[allow(clippy::needless_pass_by_value)]
fn handle_without_gvl_panic(ruby: &Ruby, err: Box<dyn std::any::Any + Send>) -> Error {
    let msg = if let Some(s) = err.downcast_ref::<&str>() {
        format!("Validation panicked: {s}")
    } else if let Some(s) = err.downcast_ref::<String>() {
        format!("Validation panicked: {s}")
    } else {
        "Validation panicked".to_string()
    };
    Error::new(ruby.exception_runtime_error(), msg)
}

/// Run a closure without holding the Ruby GVL.
///
/// The closure runs on the same thread, just without the GVL held,
/// allowing other Ruby threads to proceed. The closure MUST NOT
/// access any Ruby objects or call any Ruby API.
///
/// # Safety
/// Caller must ensure the closure does not interact with Ruby.
#[allow(unsafe_code)]
unsafe fn without_gvl<F, R>(f: F) -> Result<R, Box<dyn std::any::Any + Send>>
where
    F: FnMut() -> R,
{
    struct Payload<F, R> {
        f: F,
        result: std::mem::MaybeUninit<Result<R, Box<dyn std::any::Any + Send>>>,
    }

    unsafe extern "C" fn call<F: FnMut() -> R, R>(
        data: *mut std::ffi::c_void,
    ) -> *mut std::ffi::c_void {
        let payload = unsafe { &mut *data.cast::<Payload<F, R>>() };
        let result = panic::catch_unwind(AssertUnwindSafe(|| (payload.f)()));
        payload.result.write(result);
        std::ptr::null_mut()
    }

    let mut payload = Payload {
        f,
        result: std::mem::MaybeUninit::uninit(),
    };

    unsafe {
        rb_sys::rb_thread_call_without_gvl(
            Some(call::<F, R>),
            (&raw mut payload).cast::<std::ffi::c_void>(),
            None,
            std::ptr::null_mut(),
        )
    };

    unsafe { payload.result.assume_init() }
}

/// Wrapper around `jsonschema::Validator`.
///
/// Holds GC-protection state for Ruby callbacks (format checkers, custom keywords,
/// retrievers) that live inside the inner `jsonschema::Validator` as trait objects.
/// See the doc comment on `CallbackRoots` for the full picture.
#[derive(magnus::TypedData)]
#[magnus(class = "JSONSchema::Validator", free_immediately, size, mark)]
pub struct Validator {
    validator: jsonschema::Validator,
    mask: Option<String>,
    has_ruby_callbacks: bool,
    /// Marked during Ruby's GC mark phase to keep runtime callbacks alive.
    callback_roots: CallbackRoots,
    /// Protects callbacks via `register_address` during schema compilation —
    /// before this wrapper exists and `mark()` can run. Held so that its `Drop`
    /// impl calls `unregister_address` to balance the registrations.
    _compilation_roots: CompilationRootsRef,
}

impl DataTypeFunctions for Validator {
    fn mark(&self, marker: &magnus::gc::Marker) {
        self.mark_callback_roots(marker);
    }
}

impl Validator {
    fn mark_callback_roots(&self, marker: &magnus::gc::Marker) {
        // Avoid panicking in Ruby GC mark paths; preserving existing roots is safer than aborting.
        let roots = match self.callback_roots.lock() {
            Ok(roots) => roots,
            Err(poisoned) => poisoned.into_inner(),
        };
        for root in roots.iter().copied() {
            marker.mark(root);
        }
    }

    #[allow(unsafe_code)]
    fn is_valid(ruby: &Ruby, rb_self: &Self, instance: Value) -> Result<bool, Error> {
        let json_instance = to_value(ruby, instance)?;

        if rb_self.has_ruby_callbacks {
            let result = catch_unwind_silent(AssertUnwindSafe(|| {
                rb_self.validator.is_valid(&json_instance)
            }));
            match result {
                Ok(valid) => Ok(valid),
                Err(err) => Err(handle_callback_panic(ruby, err)),
            }
        } else {
            // SAFETY: validation is pure Rust with no Ruby callbacks
            match unsafe { without_gvl(|| rb_self.validator.is_valid(&json_instance)) } {
                Ok(valid) => Ok(valid),
                Err(err) => Err(handle_without_gvl_panic(ruby, err)),
            }
        }
    }

    #[allow(unsafe_code)]
    fn validate(ruby: &Ruby, rb_self: &Self, instance: Value) -> Result<(), Error> {
        let json_instance = to_value(ruby, instance)?;

        if rb_self.has_ruby_callbacks {
            let result = catch_unwind_silent(AssertUnwindSafe(|| {
                rb_self.validator.validate(&json_instance)
            }));
            match result {
                Ok(Ok(())) => Ok(()),
                Ok(Err(error)) => Err(raise_validation_error(
                    ruby,
                    error,
                    Some(&json_instance),
                    rb_self.mask.as_deref(),
                )),
                Err(err) => Err(handle_callback_panic(ruby, err)),
            }
        } else {
            // SAFETY: validation is pure Rust with no Ruby callbacks
            match unsafe { without_gvl(|| rb_self.validator.validate(&json_instance)) } {
                Ok(Ok(())) => Ok(()),
                Ok(Err(error)) => Err(raise_validation_error(
                    ruby,
                    error,
                    Some(&json_instance),
                    rb_self.mask.as_deref(),
                )),
                Err(err) => Err(handle_without_gvl_panic(ruby, err)),
            }
        }
    }

    #[allow(unsafe_code)]
    fn iter_errors(ruby: &Ruby, rb_self: &Self, instance: Value) -> Result<Value, Error> {
        let json_instance = to_value(ruby, instance)?;

        if ruby.block_given() {
            // Lazy path: yield errors one at a time to the block
            if rb_self.has_ruby_callbacks {
                let mut iter = rb_self.validator.iter_errors(&json_instance);
                loop {
                    let result = catch_unwind_silent(AssertUnwindSafe(|| iter.next()));
                    match result {
                        Ok(Some(error)) => {
                            let ruby_error = to_ruby_error_value(
                                ruby,
                                error,
                                Some(&json_instance),
                                rb_self.mask.as_deref(),
                            )?;
                            ruby.yield_value::<Value, Value>(ruby_error)?;
                        }
                        Ok(None) => break,
                        Err(err) => return Err(handle_callback_panic(ruby, err)),
                    }
                }
            } else {
                for error in rb_self.validator.iter_errors(&json_instance) {
                    let ruby_error = to_ruby_error_value(
                        ruby,
                        error,
                        Some(&json_instance),
                        rb_self.mask.as_deref(),
                    )?;
                    ruby.yield_value::<Value, Value>(ruby_error)?;
                }
            }
            Ok(ruby.qnil().as_value())
        } else if rb_self.has_ruby_callbacks {
            // Eager path with callbacks
            let result = catch_unwind_silent(AssertUnwindSafe(|| {
                rb_self
                    .validator
                    .iter_errors(&json_instance)
                    .collect::<Vec<_>>()
            }));
            match result {
                Ok(errors) => {
                    let arr = ruby.ary_new_capa(errors.len());
                    for e in errors {
                        arr.push(to_ruby_error_value(
                            ruby,
                            e,
                            Some(&json_instance),
                            rb_self.mask.as_deref(),
                        )?)?;
                    }
                    Ok(arr.as_value())
                }
                Err(err) => Err(handle_callback_panic(ruby, err)),
            }
        } else {
            // Eager path without callbacks — release GVL
            // SAFETY: validation is pure Rust with no Ruby callbacks
            let errors = match unsafe {
                without_gvl(|| {
                    rb_self
                        .validator
                        .iter_errors(&json_instance)
                        .collect::<Vec<_>>()
                })
            } {
                Ok(errors) => errors,
                Err(err) => return Err(handle_without_gvl_panic(ruby, err)),
            };
            let arr = ruby.ary_new_capa(errors.len());
            for e in errors {
                arr.push(to_ruby_error_value(
                    ruby,
                    e,
                    Some(&json_instance),
                    rb_self.mask.as_deref(),
                )?)?;
            }
            Ok(arr.as_value())
        }
    }

    #[allow(unsafe_code)]
    fn evaluate(ruby: &Ruby, rb_self: &Self, instance: Value) -> Result<Evaluation, Error> {
        let json_instance = to_value(ruby, instance)?;

        if rb_self.has_ruby_callbacks {
            let result = catch_unwind_silent(AssertUnwindSafe(|| {
                rb_self.validator.evaluate(&json_instance)
            }));
            match result {
                Ok(output) => Ok(Evaluation::new(output)),
                Err(err) => Err(handle_callback_panic(ruby, err)),
            }
        } else {
            // SAFETY: validation is pure Rust with no Ruby callbacks
            let output = match unsafe { without_gvl(|| rb_self.validator.evaluate(&json_instance)) }
            {
                Ok(output) => output,
                Err(err) => return Err(handle_without_gvl_panic(ruby, err)),
            };
            Ok(Evaluation::new(output))
        }
    }

    fn inspect(&self) -> String {
        let draft = match self.validator.draft() {
            jsonschema::Draft::Draft4 => "Draft4",
            jsonschema::Draft::Draft6 => "Draft6",
            jsonschema::Draft::Draft7 => "Draft7",
            jsonschema::Draft::Draft201909 => "Draft201909",
            jsonschema::Draft::Draft202012 => "Draft202012",
            _ => "Unknown",
        };
        format!("#<JSONSchema::{draft}Validator>")
    }
}

fn validator_for(ruby: &Ruby, args: &[Value]) -> Result<Validator, Error> {
    let parsed_args = scan_args::<(Value,), (), (), (), _, ()>(args)?;
    let (schema,) = parsed_args.required;
    let kw = extract_kwargs_no_draft(ruby, parsed_args.keywords)?;

    let json_schema = to_schema_value(ruby, schema)?;
    let parsed = build_parsed_options(ruby, kw, None)?;
    let has_ruby_callbacks = parsed.has_ruby_callbacks;
    let BuiltValidator {
        validator,
        callback_roots,
        compilation_roots,
    } = build_validator(
        ruby,
        parsed.options,
        parsed.retriever,
        parsed.callback_roots,
        parsed.compilation_roots,
        &json_schema,
    )?;
    Ok(Validator {
        validator,
        mask: parsed.mask,
        has_ruby_callbacks,
        callback_roots,
        _compilation_roots: compilation_roots,
    })
}

#[allow(unsafe_code)]
fn is_valid(ruby: &Ruby, args: &[Value]) -> Result<bool, Error> {
    let parsed_args = scan_args::<(Value, Value), (), (), (), _, ()>(args)?;
    let (schema, instance) = parsed_args.required;
    let kw = extract_kwargs(ruby, parsed_args.keywords)?;

    let json_schema = to_schema_value(ruby, schema)?;
    let json_instance = to_value(ruby, instance)?;
    let parsed = build_parsed_options(ruby, kw, None)?;
    let has_ruby_callbacks = parsed.has_ruby_callbacks;
    let BuiltValidator {
        validator,
        callback_roots,
        compilation_roots: _compilation_roots,
    } = build_validator(
        ruby,
        parsed.options,
        parsed.retriever,
        parsed.callback_roots,
        parsed.compilation_roots,
        &json_schema,
    )?;

    if has_ruby_callbacks {
        let _callback_roots = CallbackRootGuard::new(ruby, &callback_roots);
        let result = catch_unwind_silent(AssertUnwindSafe(|| validator.is_valid(&json_instance)));
        match result {
            Ok(valid) => Ok(valid),
            Err(err) => Err(handle_callback_panic(ruby, err)),
        }
    } else {
        // SAFETY: validation is pure Rust with no Ruby callbacks
        match unsafe { without_gvl(|| validator.is_valid(&json_instance)) } {
            Ok(valid) => Ok(valid),
            Err(err) => Err(handle_without_gvl_panic(ruby, err)),
        }
    }
}

#[allow(unsafe_code)]
fn validate(ruby: &Ruby, args: &[Value]) -> Result<(), Error> {
    let parsed_args = scan_args::<(Value, Value), (), (), (), _, ()>(args)?;
    let (schema, instance) = parsed_args.required;
    let kw = extract_kwargs(ruby, parsed_args.keywords)?;

    let json_schema = to_schema_value(ruby, schema)?;
    let json_instance = to_value(ruby, instance)?;
    let parsed = build_parsed_options(ruby, kw, None)?;
    let has_ruby_callbacks = parsed.has_ruby_callbacks;
    let BuiltValidator {
        validator,
        callback_roots,
        compilation_roots: _compilation_roots,
    } = build_validator(
        ruby,
        parsed.options,
        parsed.retriever,
        parsed.callback_roots,
        parsed.compilation_roots,
        &json_schema,
    )?;

    if has_ruby_callbacks {
        let _callback_roots = CallbackRootGuard::new(ruby, &callback_roots);
        let result = catch_unwind_silent(AssertUnwindSafe(|| validator.validate(&json_instance)));
        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(raise_validation_error(
                ruby,
                error,
                Some(&json_instance),
                parsed.mask.as_deref(),
            )),
            Err(err) => Err(handle_callback_panic(ruby, err)),
        }
    } else {
        // SAFETY: validation is pure Rust with no Ruby callbacks
        match unsafe { without_gvl(|| validator.validate(&json_instance)) } {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(raise_validation_error(
                ruby,
                error,
                Some(&json_instance),
                parsed.mask.as_deref(),
            )),
            Err(err) => Err(handle_without_gvl_panic(ruby, err)),
        }
    }
}

#[allow(unsafe_code)]
fn each_error(ruby: &Ruby, args: &[Value]) -> Result<Value, Error> {
    let parsed_args = scan_args::<(Value, Value), (), (), (), _, ()>(args)?;
    let (schema, instance) = parsed_args.required;
    let kw = extract_kwargs(ruby, parsed_args.keywords)?;

    let json_schema = to_schema_value(ruby, schema)?;
    let json_instance = to_value(ruby, instance)?;
    let parsed = build_parsed_options(ruby, kw, None)?;
    let has_ruby_callbacks = parsed.has_ruby_callbacks;
    let BuiltValidator {
        validator,
        callback_roots,
        compilation_roots: _compilation_roots,
    } = build_validator(
        ruby,
        parsed.options,
        parsed.retriever,
        parsed.callback_roots,
        parsed.compilation_roots,
        &json_schema,
    )?;

    if ruby.block_given() {
        // Lazy path: yield errors one at a time to the block
        if has_ruby_callbacks {
            let _callback_roots = CallbackRootGuard::new(ruby, &callback_roots);
            let mut iter = validator.iter_errors(&json_instance);
            loop {
                let result = catch_unwind_silent(AssertUnwindSafe(|| iter.next()));
                match result {
                    Ok(Some(error)) => {
                        let ruby_error = to_ruby_error_value(
                            ruby,
                            error,
                            Some(&json_instance),
                            parsed.mask.as_deref(),
                        )?;
                        ruby.yield_value::<Value, Value>(ruby_error)?;
                    }
                    Ok(None) => break,
                    Err(err) => return Err(handle_callback_panic(ruby, err)),
                }
            }
        } else {
            for error in validator.iter_errors(&json_instance) {
                let ruby_error =
                    to_ruby_error_value(ruby, error, Some(&json_instance), parsed.mask.as_deref())?;
                ruby.yield_value::<Value, Value>(ruby_error)?;
            }
        }
        Ok(ruby.qnil().as_value())
    } else if has_ruby_callbacks {
        // Eager path with callbacks
        let _callback_roots = CallbackRootGuard::new(ruby, &callback_roots);
        let result = catch_unwind_silent(AssertUnwindSafe(|| {
            validator.iter_errors(&json_instance).collect::<Vec<_>>()
        }));
        match result {
            Ok(errors) => {
                let arr = ruby.ary_new_capa(errors.len());
                for e in errors {
                    arr.push(to_ruby_error_value(
                        ruby,
                        e,
                        Some(&json_instance),
                        parsed.mask.as_deref(),
                    )?)?;
                }
                Ok(arr.as_value())
            }
            Err(err) => Err(handle_callback_panic(ruby, err)),
        }
    } else {
        // Eager path without callbacks — release GVL
        // SAFETY: validation is pure Rust with no Ruby callbacks
        let errors = match unsafe {
            without_gvl(|| validator.iter_errors(&json_instance).collect::<Vec<_>>())
        } {
            Ok(errors) => errors,
            Err(err) => return Err(handle_without_gvl_panic(ruby, err)),
        };
        let arr = ruby.ary_new_capa(errors.len());
        for e in errors {
            arr.push(to_ruby_error_value(
                ruby,
                e,
                Some(&json_instance),
                parsed.mask.as_deref(),
            )?)?;
        }
        Ok(arr.as_value())
    }
}

#[allow(unsafe_code)]
fn evaluate(ruby: &Ruby, args: &[Value]) -> Result<Evaluation, Error> {
    let parsed_args = scan_args::<(Value, Value), (), (), (), _, ()>(args)?;
    let (schema, instance) = parsed_args.required;
    let kw = extract_evaluate_kwargs(ruby, parsed_args.keywords)?;

    let json_schema = to_schema_value(ruby, schema)?;
    let json_instance = to_value(ruby, instance)?;
    let parsed = build_parsed_options(ruby, kw, None)?;
    let has_ruby_callbacks = parsed.has_ruby_callbacks;
    let BuiltValidator {
        validator,
        callback_roots,
        compilation_roots: _compilation_roots,
    } = build_validator(
        ruby,
        parsed.options,
        parsed.retriever,
        parsed.callback_roots,
        parsed.compilation_roots,
        &json_schema,
    )?;

    if has_ruby_callbacks {
        let _callback_roots = CallbackRootGuard::new(ruby, &callback_roots);
        let result = catch_unwind_silent(AssertUnwindSafe(|| validator.evaluate(&json_instance)));
        match result {
            Ok(output) => Ok(Evaluation::new(output)),
            Err(err) => Err(handle_callback_panic(ruby, err)),
        }
    } else {
        // SAFETY: validation is pure Rust with no Ruby callbacks
        let output = match unsafe { without_gvl(|| validator.evaluate(&json_instance)) } {
            Ok(output) => output,
            Err(err) => return Err(handle_without_gvl_panic(ruby, err)),
        };
        Ok(Evaluation::new(output))
    }
}

macro_rules! define_draft_validator {
    ($name:ident, $class_name:expr, $draft:expr) => {
        #[derive(magnus::TypedData)]
        #[magnus(class = $class_name, free_immediately, size, mark)]
        pub struct $name {
            inner: Validator,
        }

        impl DataTypeFunctions for $name {
            fn mark(&self, marker: &magnus::gc::Marker) {
                self.inner.mark_callback_roots(marker);
            }
        }

        impl $name {
            fn new_impl(ruby: &Ruby, args: &[Value]) -> Result<Self, Error> {
                let parsed_args = scan_args::<(Value,), (), (), (), _, ()>(args)?;
                let (schema,) = parsed_args.required;
                let kw = extract_kwargs_no_draft(ruby, parsed_args.keywords)?;

                let json_schema = to_schema_value(ruby, schema)?;
                let parsed = build_parsed_options(ruby, kw, Some($draft))?;
                let has_ruby_callbacks = parsed.has_ruby_callbacks;
                let BuiltValidator {
                    validator,
                    callback_roots,
                    compilation_roots,
                } = build_validator(
                    ruby,
                    parsed.options,
                    parsed.retriever,
                    parsed.callback_roots,
                    parsed.compilation_roots,
                    &json_schema,
                )?;
                Ok($name {
                    inner: Validator {
                        validator,
                        mask: parsed.mask,
                        has_ruby_callbacks,
                        callback_roots,
                        _compilation_roots: compilation_roots,
                    },
                })
            }

            fn is_valid(ruby: &Ruby, rb_self: &Self, instance: Value) -> Result<bool, Error> {
                Validator::is_valid(ruby, &rb_self.inner, instance)
            }

            fn validate(ruby: &Ruby, rb_self: &Self, instance: Value) -> Result<(), Error> {
                Validator::validate(ruby, &rb_self.inner, instance)
            }

            fn iter_errors(ruby: &Ruby, rb_self: &Self, instance: Value) -> Result<Value, Error> {
                Validator::iter_errors(ruby, &rb_self.inner, instance)
            }

            fn evaluate(ruby: &Ruby, rb_self: &Self, instance: Value) -> Result<Evaluation, Error> {
                Validator::evaluate(ruby, &rb_self.inner, instance)
            }

            fn inspect(&self) -> String {
                self.inner.inspect()
            }
        }
    };
}

define_draft_validator!(
    Draft4Validator,
    "JSONSchema::Draft4Validator",
    jsonschema::Draft::Draft4
);
define_draft_validator!(
    Draft6Validator,
    "JSONSchema::Draft6Validator",
    jsonschema::Draft::Draft6
);
define_draft_validator!(
    Draft7Validator,
    "JSONSchema::Draft7Validator",
    jsonschema::Draft::Draft7
);
define_draft_validator!(
    Draft201909Validator,
    "JSONSchema::Draft201909Validator",
    jsonschema::Draft::Draft201909
);
define_draft_validator!(
    Draft202012Validator,
    "JSONSchema::Draft202012Validator",
    jsonschema::Draft::Draft202012
);

fn meta_is_valid(ruby: &Ruby, args: &[Value]) -> Result<bool, Error> {
    use magnus::scan_args::get_kwargs;
    let parsed_args = scan_args::<(Value,), (), (), (), _, ()>(args)?;
    let (schema,) = parsed_args.required;
    let kw: magnus::scan_args::KwArgs<(), (Option<Option<&Registry>>,), ()> =
        get_kwargs(parsed_args.keywords, &[], &[*options::KW_REGISTRY])?;
    let registry = kw.optional.0.flatten();

    let json_schema = to_schema_value(ruby, schema)?;

    let result = if let Some(reg) = registry {
        jsonschema::meta::options()
            .with_registry(reg.inner.clone())
            .validate(&json_schema)
    } else {
        jsonschema::meta::validate(&json_schema)
    };

    match result {
        Ok(()) => Ok(true),
        Err(error) => {
            if let jsonschema::error::ValidationErrorKind::Referencing(err) = error.kind() {
                return Err(referencing_error(ruby, err.to_string()));
            }
            Ok(false)
        }
    }
}

fn meta_validate(ruby: &Ruby, args: &[Value]) -> Result<(), Error> {
    use magnus::scan_args::get_kwargs;
    let parsed_args = scan_args::<(Value,), (), (), (), _, ()>(args)?;
    let (schema,) = parsed_args.required;
    let kw: magnus::scan_args::KwArgs<(), (Option<Option<&Registry>>,), ()> =
        get_kwargs(parsed_args.keywords, &[], &[*options::KW_REGISTRY])?;
    let registry = kw.optional.0.flatten();

    let json_schema = to_schema_value(ruby, schema)?;

    let result = if let Some(reg) = registry {
        jsonschema::meta::options()
            .with_registry(reg.inner.clone())
            .validate(&json_schema)
    } else {
        jsonschema::meta::validate(&json_schema)
    };

    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            if let jsonschema::error::ValidationErrorKind::Referencing(err) = error.kind() {
                return Err(referencing_error(ruby, err.to_string()));
            }
            Err(raise_validation_error(
                ruby,
                error,
                Some(&json_schema),
                None,
            ))
        }
    }
}

// ValidationError instance methods (defined from Rust, called on exception instances)

fn validation_error_to_s(ruby: &Ruby, rb_self: Value) -> Result<Value, Error> {
    let obj = RObject::from_value(rb_self).ok_or_else(|| {
        Error::new(
            Ruby::get().expect("Ruby").exception_type_error(),
            "expected object",
        )
    })?;
    let message: Value = obj.ivar_get(*ID_AT_MESSAGE)?;
    if message.is_nil() {
        ruby.call_super(())
    } else {
        Ok(message)
    }
}

fn validation_error_inspect(_ruby: &Ruby, rb_self: Value) -> Result<String, Error> {
    let msg: String = rb_self.funcall("to_s", ())?;
    Ok(format!("#<JSONSchema::ValidationError: {msg}>"))
}

fn validation_error_eq(ruby: &Ruby, rb_self: Value, other: Value) -> Result<bool, Error> {
    let exc_class = ruby.get_inner(&VALIDATION_ERROR_CLASS);
    let other_obj = match RObject::from_value(other) {
        Some(obj) if obj.is_kind_of(exc_class) => obj,
        _ => return Ok(false),
    };
    let self_obj = RObject::from_value(rb_self)
        .ok_or_else(|| Error::new(ruby.exception_type_error(), "expected object"))?;

    let self_key = ruby.ary_new_capa(3);
    self_key.push(self_obj.ivar_get::<_, Value>(*ID_AT_MESSAGE)?)?;
    self_key.push(self_obj.ivar_get::<_, Value>(*ID_AT_SCHEMA_PATH)?)?;
    self_key.push(self_obj.ivar_get::<_, Value>(*ID_AT_INSTANCE_PATH)?)?;

    let other_key = ruby.ary_new_capa(3);
    other_key.push(other_obj.ivar_get::<_, Value>(*ID_AT_MESSAGE)?)?;
    other_key.push(other_obj.ivar_get::<_, Value>(*ID_AT_SCHEMA_PATH)?)?;
    other_key.push(other_obj.ivar_get::<_, Value>(*ID_AT_INSTANCE_PATH)?)?;

    self_key.funcall("==", (other_key,))
}

fn validation_error_hash(ruby: &Ruby, rb_self: Value) -> Result<Value, Error> {
    let obj = RObject::from_value(rb_self)
        .ok_or_else(|| Error::new(ruby.exception_type_error(), "expected object"))?;
    let arr = ruby.ary_new_capa(3);
    arr.push(obj.ivar_get::<_, Value>(*ID_AT_MESSAGE)?)?;
    arr.push(obj.ivar_get::<_, Value>(*ID_AT_SCHEMA_PATH)?)?;
    arr.push(obj.ivar_get::<_, Value>(*ID_AT_INSTANCE_PATH)?)?;
    arr.funcall("hash", ())
}

#[magnus::init(name = "jsonschema_rb")]
fn init(ruby: &Ruby) -> Result<(), Error> {
    // Conditionally suppress panic output — only when inside `catch_unwind`
    // blocks used for Ruby callback panics (format checkers, custom keywords).
    // Other panics pass through to the default handler to preserve debugging output.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let suppress = SUPPRESS_PANIC_OUTPUT.with(|flag| *flag.borrow());
        if !suppress {
            default_hook(info);
        }
    }));

    let module = ruby.define_module("JSONSchema")?;

    // ValidationError < StandardError
    let validation_error_class =
        module.define_error("ValidationError", ruby.exception_standard_error())?;
    let validation_error_rclass =
        RClass::from_value(validation_error_class.as_value()).expect("ExceptionClass is an RClass");
    for sym_id in [
        &*ID_SYM_MESSAGE,
        &*ID_SYM_VERBOSE_MESSAGE,
        &*ID_SYM_INSTANCE_PATH,
        &*ID_SYM_SCHEMA_PATH,
        &*ID_SYM_EVALUATION_PATH,
        &*ID_SYM_KIND,
        &*ID_SYM_INSTANCE,
        &*ID_SYM_INSTANCE_PATH_POINTER,
        &*ID_SYM_SCHEMA_PATH_POINTER,
        &*ID_SYM_EVALUATION_PATH_POINTER,
    ] {
        let _: Value = validation_error_rclass.funcall("attr_reader", (sym_id.to_symbol(),))?;
    }
    validation_error_rclass.define_method("message", method!(validation_error_to_s, 0))?;
    validation_error_rclass.define_method("to_s", method!(validation_error_to_s, 0))?;
    validation_error_rclass.define_method("inspect", method!(validation_error_inspect, 0))?;
    validation_error_rclass.define_method("==", method!(validation_error_eq, 1))?;
    validation_error_rclass.define_alias("eql?", "==")?;
    validation_error_rclass.define_method("hash", method!(validation_error_hash, 0))?;

    // ReferencingError < StandardError
    module.define_error("ReferencingError", ruby.exception_standard_error())?;

    // Module-level functions
    module.define_singleton_method("validator_for", function!(validator_for, -1))?;
    module.define_singleton_method("valid?", function!(is_valid, -1))?;
    module.define_singleton_method("validate!", function!(validate, -1))?;
    module.define_singleton_method("each_error", function!(each_error, -1))?;
    module.define_singleton_method("evaluate", function!(evaluate, -1))?;

    // Validator class
    let validator_class = module.define_class("Validator", ruby.class_object())?;
    validator_class.define_method("valid?", method!(Validator::is_valid, 1))?;
    validator_class.define_method("validate!", method!(Validator::validate, 1))?;
    validator_class.define_method("each_error", method!(Validator::iter_errors, 1))?;
    validator_class.define_method("evaluate", method!(Validator::evaluate, 1))?;
    validator_class.define_method("inspect", method!(Validator::inspect, 0))?;

    // Draft-specific validators
    macro_rules! define_draft_class {
        ($ruby:expr, $module:expr, $name:ident, $class_str:expr, $superclass:expr) => {
            let cls = $module.define_class($class_str, $superclass)?;
            cls.define_singleton_method("new", function!($name::new_impl, -1))?;
            cls.define_method("valid?", method!($name::is_valid, 1))?;
            cls.define_method("validate!", method!($name::validate, 1))?;
            cls.define_method("each_error", method!($name::iter_errors, 1))?;
            cls.define_method("evaluate", method!($name::evaluate, 1))?;
            cls.define_method("inspect", method!($name::inspect, 0))?;
        };
    }

    define_draft_class!(
        ruby,
        module,
        Draft4Validator,
        "Draft4Validator",
        validator_class
    );
    define_draft_class!(
        ruby,
        module,
        Draft6Validator,
        "Draft6Validator",
        validator_class
    );
    define_draft_class!(
        ruby,
        module,
        Draft7Validator,
        "Draft7Validator",
        validator_class
    );
    define_draft_class!(
        ruby,
        module,
        Draft201909Validator,
        "Draft201909Validator",
        validator_class
    );
    define_draft_class!(
        ruby,
        module,
        Draft202012Validator,
        "Draft202012Validator",
        validator_class
    );

    // Internal implementation detail for shared validator behavior.
    let _: Value = module.funcall("private_constant", ("Validator",))?;

    evaluation::define_class(ruby, &module)?;
    registry::define_class(ruby, &module)?;
    error_kind::define_class(ruby, &module)?;
    options::define_classes(ruby, &module)?;

    let meta_module = module.define_module("Meta")?;
    meta_module.define_singleton_method("valid?", function!(meta_is_valid, -1))?;
    meta_module.define_singleton_method("validate!", function!(meta_validate, -1))?;

    Ok(())
}
