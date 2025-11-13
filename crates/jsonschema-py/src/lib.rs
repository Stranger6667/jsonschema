#![allow(clippy::too_many_arguments)]
#![allow(unsafe_code)]
#![allow(unreachable_pub)]
#![allow(rustdoc::bare_urls)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_wrap)]
use std::{
    any::Any,
    cell::RefCell,
    io::Write,
    panic::{self, AssertUnwindSafe},
};

use jsonschema::{paths::LocationSegment, Draft};
use pyo3::{
    exceptions::{self, PyValueError},
    ffi::PyUnicode_AsUTF8AndSize,
    prelude::*,
    types::{PyAny, PyDict, PyList, PyString, PyType},
    wrap_pyfunction,
};
use regex::{FancyRegexOptions, RegexOptions};
use retriever::{into_retriever, Retriever};
use ser::to_value;
#[macro_use]
extern crate pyo3_built;

mod regex;
mod registry;
mod retriever;
mod ser;
mod types;

const DRAFT7: u8 = 7;
const DRAFT6: u8 = 6;
const DRAFT4: u8 = 4;
const DRAFT201909: u8 = 19;
const DRAFT202012: u8 = 20;

fn validation_error_type(py: Python<'_>) -> PyResult<Bound<'_, PyType>> {
    let module = py.import("jsonschema_rs")?;
    let exception_class = module.getattr("ValidationError")?;
    Ok(exception_class.cast_into::<PyType>()?)
}

fn referencing_error_type(py: Python<'_>) -> PyResult<Bound<'_, PyType>> {
    let module = py.import("jsonschema_rs")?;
    let exception_class = module.getattr("ReferencingError")?;
    Ok(exception_class.cast_into::<PyType>()?)
}

/// Convert a serde_json::Value to a Python object, properly handling arbitrary precision numbers
fn value_to_python(py: Python<'_>, value: &serde_json::Value) -> PyResult<Py<PyAny>> {
    match value {
        serde_json::Value::Null => Ok(py.None()),
        serde_json::Value::Bool(b) => Ok(pyo3::types::PyBool::new(py, *b)
            .to_owned()
            .into_any()
            .unbind()),
        serde_json::Value::Number(n) => {
            // With arbitrary precision, try to parse as integer first, then fall back to float
            if let Some(i) = n.as_i64() {
                Ok(pyo3::types::PyInt::new(py, i).into_any().unbind())
            } else if let Some(u) = n.as_u64() {
                Ok(pyo3::types::PyInt::new(py, u).into_any().unbind())
            } else {
                // For values that don't fit in i64/u64, check the string representation
                let s = n.as_str();
                let is_float = s.bytes().any(|b| b == b'.' || b == b'e' || b == b'E');
                if is_float {
                    if let Some(f) = n.as_f64() {
                        Ok(pyo3::types::PyFloat::new(py, f).into_any().unbind())
                    } else {
                        // Fall back to decimal.Decimal for values outside f64 range so Python callers
                        // observe the exact literal from JSON instead of ValueError/inf when numbers
                        // exceed binary64 limits.
                        let decimal = py.import("decimal")?.getattr("Decimal")?;
                        decimal.call1((s,)).map(|obj| obj.into_any().unbind())
                    }
                } else {
                    // Large integer - parse from string representation
                    let int_type = py.get_type::<pyo3::types::PyInt>();
                    int_type.call1((s,)).map(|obj| obj.into_any().unbind())
                }
            }
        }
        serde_json::Value::String(s) => Ok(PyString::new(py, s).into_any().unbind()),
        serde_json::Value::Array(arr) => {
            let py_list = PyList::empty(py);
            for item in arr {
                py_list.append(value_to_python(py, item)?)?;
            }
            Ok(py_list.into_any().unbind())
        }
        serde_json::Value::Object(obj) => {
            let py_dict = PyDict::new(py);
            for (k, v) in obj {
                py_dict.set_item(k, value_to_python(py, v)?)?;
            }
            Ok(py_dict.into_any().unbind())
        }
    }
}

struct ValidationErrorArgs {
    message: String,
    verbose_message: String,
    schema_path: Py<PyList>,
    instance_path: Py<PyList>,
    kind: ValidationErrorKind,
    instance: Py<PyAny>,
}

fn create_validation_error_object(
    py: Python<'_>,
    args: ValidationErrorArgs,
) -> PyResult<Py<PyAny>> {
    let ty = validation_error_type(py)?;
    let kind_obj = args.kind.into_pyobject(py)?.unbind();
    let obj = ty.call1((
        args.message,
        args.verbose_message,
        args.schema_path,
        args.instance_path,
        kind_obj,
        args.instance,
    ))?;
    Ok(obj.into())
}

fn validation_error_pyerr(py: Python<'_>, args: ValidationErrorArgs) -> PyResult<PyErr> {
    let obj = create_validation_error_object(py, args)?;
    Ok(PyErr::from_value(obj.into_bound(py)))
}

fn create_referencing_error_object(py: Python<'_>, message: String) -> PyResult<Py<PyAny>> {
    let ty = referencing_error_type(py)?;
    let obj = ty.call1((message,))?;
    Ok(obj.into())
}

fn referencing_error_pyerr(py: Python<'_>, message: String) -> PyResult<PyErr> {
    let obj = create_referencing_error_object(py, message)?;
    Ok(PyErr::from_value(obj.into_bound(py)))
}

/// Type of validation failure with its contextual data.
#[pyclass]
#[derive(Debug)]
enum ValidationErrorKind {
    AdditionalItems { limit: usize },
    AdditionalProperties { unexpected: Py<PyList> },
    AnyOf { context: Py<PyList> },
    BacktrackLimitExceeded { error: String },
    Constant { expected_value: Py<PyAny> },
    Contains {},
    ContentEncoding { content_encoding: String },
    ContentMediaType { content_media_type: String },
    Custom { message: String },
    Enum { options: Py<PyAny> },
    ExclusiveMaximum { limit: Py<PyAny> },
    ExclusiveMinimum { limit: Py<PyAny> },
    FalseSchema {},
    Format { format: String },
    FromUtf8 { error: String },
    MaxItems { limit: u64 },
    Maximum { limit: Py<PyAny> },
    MaxLength { limit: u64 },
    MaxProperties { limit: u64 },
    MinItems { limit: u64 },
    Minimum { limit: Py<PyAny> },
    MinLength { limit: u64 },
    MinProperties { limit: u64 },
    MultipleOf { multiple_of: Py<PyAny> },
    Not { schema: Py<PyAny> },
    OneOfMultipleValid { context: Py<PyList> },
    OneOfNotValid { context: Py<PyList> },
    Pattern { pattern: String },
    PropertyNames { error: Py<PyAny> },
    Required { property: Py<PyAny> },
    Type { types: Py<PyList> },
    UnevaluatedItems { unexpected: Py<PyList> },
    UnevaluatedProperties { unexpected: Py<PyList> },
    UniqueItems {},
    Referencing { error: Py<PyAny> },
}

impl ValidationErrorKind {
    fn try_new(
        py: Python<'_>,
        kind: jsonschema::error::ValidationErrorKind,
        mask: Option<&str>,
    ) -> PyResult<Self> {
        Ok(match kind {
            jsonschema::error::ValidationErrorKind::AdditionalItems { limit } => {
                ValidationErrorKind::AdditionalItems { limit }
            }
            jsonschema::error::ValidationErrorKind::AdditionalProperties { unexpected } => {
                ValidationErrorKind::AdditionalProperties {
                    unexpected: PyList::new(py, unexpected)?.unbind(),
                }
            }
            jsonschema::error::ValidationErrorKind::AnyOf { context } => {
                ValidationErrorKind::AnyOf {
                    context: convert_validation_context(py, context, mask)?,
                }
            }
            jsonschema::error::ValidationErrorKind::BacktrackLimitExceeded { error } => {
                ValidationErrorKind::BacktrackLimitExceeded {
                    error: error.to_string(),
                }
            }
            jsonschema::error::ValidationErrorKind::Constant { expected_value } => {
                ValidationErrorKind::Constant {
                    expected_value: value_to_python(py, &expected_value)?,
                }
            }
            jsonschema::error::ValidationErrorKind::Contains => ValidationErrorKind::Contains {},
            jsonschema::error::ValidationErrorKind::ContentEncoding { content_encoding } => {
                ValidationErrorKind::ContentEncoding { content_encoding }
            }
            jsonschema::error::ValidationErrorKind::ContentMediaType { content_media_type } => {
                ValidationErrorKind::ContentMediaType { content_media_type }
            }
            jsonschema::error::ValidationErrorKind::Custom { message } => {
                ValidationErrorKind::Custom { message }
            }
            jsonschema::error::ValidationErrorKind::Enum { options } => ValidationErrorKind::Enum {
                options: value_to_python(py, &options)?,
            },
            jsonschema::error::ValidationErrorKind::ExclusiveMaximum { limit } => {
                ValidationErrorKind::ExclusiveMaximum {
                    limit: value_to_python(py, &limit)?,
                }
            }
            jsonschema::error::ValidationErrorKind::ExclusiveMinimum { limit } => {
                ValidationErrorKind::ExclusiveMinimum {
                    limit: value_to_python(py, &limit)?,
                }
            }
            jsonschema::error::ValidationErrorKind::FalseSchema => {
                ValidationErrorKind::FalseSchema {}
            }
            jsonschema::error::ValidationErrorKind::Format { format } => {
                ValidationErrorKind::Format { format }
            }
            jsonschema::error::ValidationErrorKind::FromUtf8 { error } => {
                ValidationErrorKind::FromUtf8 {
                    error: error.to_string(),
                }
            }
            jsonschema::error::ValidationErrorKind::MaxItems { limit } => {
                ValidationErrorKind::MaxItems { limit }
            }
            jsonschema::error::ValidationErrorKind::Maximum { limit } => {
                ValidationErrorKind::Maximum {
                    limit: value_to_python(py, &limit)?,
                }
            }
            jsonschema::error::ValidationErrorKind::MaxLength { limit } => {
                ValidationErrorKind::MaxLength { limit }
            }
            jsonschema::error::ValidationErrorKind::MaxProperties { limit } => {
                ValidationErrorKind::MaxProperties { limit }
            }
            jsonschema::error::ValidationErrorKind::MinItems { limit } => {
                ValidationErrorKind::MinItems { limit }
            }
            jsonschema::error::ValidationErrorKind::Minimum { limit } => {
                ValidationErrorKind::Minimum {
                    limit: value_to_python(py, &limit)?,
                }
            }
            jsonschema::error::ValidationErrorKind::MinLength { limit } => {
                ValidationErrorKind::MinLength { limit }
            }
            jsonschema::error::ValidationErrorKind::MinProperties { limit } => {
                ValidationErrorKind::MinProperties { limit }
            }
            jsonschema::error::ValidationErrorKind::MultipleOf { multiple_of } => {
                ValidationErrorKind::MultipleOf {
                    multiple_of: value_to_python(py, &multiple_of)?,
                }
            }
            jsonschema::error::ValidationErrorKind::Not { schema } => ValidationErrorKind::Not {
                schema: value_to_python(py, &schema)?,
            },
            jsonschema::error::ValidationErrorKind::OneOfMultipleValid { context } => {
                ValidationErrorKind::OneOfMultipleValid {
                    context: convert_validation_context(py, context, mask)?,
                }
            }
            jsonschema::error::ValidationErrorKind::OneOfNotValid { context } => {
                ValidationErrorKind::OneOfNotValid {
                    context: convert_validation_context(py, context, mask)?,
                }
            }
            jsonschema::error::ValidationErrorKind::Pattern { pattern } => {
                ValidationErrorKind::Pattern { pattern }
            }
            jsonschema::error::ValidationErrorKind::PropertyNames { error } => {
                ValidationErrorKind::PropertyNames {
                    error: {
                        let (message, verbose_message, schema_path, instance_path, kind, instance) =
                            into_validation_error_args(py, *error, mask)?;
                        create_validation_error_object(
                            py,
                            ValidationErrorArgs {
                                message,
                                verbose_message,
                                schema_path,
                                instance_path,
                                kind,
                                instance,
                            },
                        )?
                    },
                }
            }
            jsonschema::error::ValidationErrorKind::Required { property } => {
                ValidationErrorKind::Required {
                    property: value_to_python(py, &property)?,
                }
            }
            jsonschema::error::ValidationErrorKind::Type { kind } => ValidationErrorKind::Type {
                types: {
                    match kind {
                        jsonschema::error::TypeKind::Single(ty) => {
                            PyList::new(py, [ty.to_string()].iter())?.unbind()
                        }
                        jsonschema::error::TypeKind::Multiple(types) => {
                            PyList::new(py, types.iter().map(|ty| ty.to_string()))?.unbind()
                        }
                    }
                },
            },
            jsonschema::error::ValidationErrorKind::UnevaluatedItems { unexpected } => {
                ValidationErrorKind::UnevaluatedItems {
                    unexpected: PyList::new(py, unexpected)?.unbind(),
                }
            }
            jsonschema::error::ValidationErrorKind::UnevaluatedProperties { unexpected } => {
                ValidationErrorKind::UnevaluatedProperties {
                    unexpected: PyList::new(py, unexpected)?.unbind(),
                }
            }
            jsonschema::error::ValidationErrorKind::UniqueItems => {
                ValidationErrorKind::UniqueItems {}
            }
            jsonschema::error::ValidationErrorKind::Referencing(error) => {
                ValidationErrorKind::Referencing {
                    error: create_referencing_error_object(py, error.to_string())?,
                }
            }
        })
    }
}

fn convert_validation_context(
    py: Python<'_>,
    context: Vec<Vec<jsonschema::error::ValidationError>>,
    mask: Option<&str>,
) -> PyResult<Py<PyList>> {
    let mut py_context: Vec<Py<PyList>> = Vec::with_capacity(context.len());

    for errors in context {
        let mut py_errors: Vec<Py<PyAny>> = Vec::with_capacity(errors.len());

        for error in errors {
            let (message, verbose_message, schema_path, instance_path, kind, instance) =
                into_validation_error_args(py, error, mask)?;

            py_errors.push(create_validation_error_object(
                py,
                ValidationErrorArgs {
                    message,
                    verbose_message,
                    schema_path,
                    instance_path,
                    kind,
                    instance,
                },
            )?);
        }

        py_context.push(PyList::new(py, py_errors)?.unbind());
    }

    Ok(PyList::new(py, py_context)?.unbind())
}

#[pyclass]
struct ValidationErrorIter {
    iter: std::vec::IntoIter<PyErr>,
}

#[pymethods]
impl ValidationErrorIter {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __next__(mut slf: PyRefMut<'_, Self>) -> Option<PyErr> {
        slf.iter.next()
    }
}

#[allow(clippy::type_complexity)]
fn into_validation_error_args(
    py: Python<'_>,
    error: jsonschema::ValidationError<'_>,
    mask: Option<&str>,
) -> PyResult<(
    String,
    String,
    Py<PyList>,
    Py<PyList>,
    ValidationErrorKind,
    Py<PyAny>,
)> {
    let message = if let Some(mask) = mask {
        error.masked_with(mask).to_string()
    } else {
        error.to_string()
    };
    let verbose_message = to_error_message(&error, message.clone(), mask);
    let into_path = |segment: LocationSegment<'_>| match segment {
        LocationSegment::Property(property) => {
            property.into_pyobject(py).and_then(Py::<PyAny>::try_from)
        }
        LocationSegment::Index(idx) => idx.into_pyobject(py).and_then(Py::<PyAny>::try_from),
    };
    let elements = error
        .schema_path
        .into_iter()
        .map(into_path)
        .collect::<Result<Vec<_>, _>>()?;
    let schema_path = PyList::new(py, elements)?.unbind();
    let elements = error
        .instance_path
        .into_iter()
        .map(into_path)
        .collect::<Result<Vec<_>, _>>()?;
    let instance_path = PyList::new(py, elements)?.unbind();
    let kind = ValidationErrorKind::try_new(py, error.kind, mask)?;
    let instance = value_to_python(py, error.instance.as_ref())?;
    Ok((
        message,
        verbose_message,
        schema_path,
        instance_path,
        kind,
        instance,
    ))
}
fn into_py_err(
    py: Python<'_>,
    error: jsonschema::ValidationError<'_>,
    mask: Option<&str>,
) -> PyResult<PyErr> {
    let (message, verbose_message, schema_path, instance_path, kind, instance) =
        into_validation_error_args(py, error, mask)?;
    validation_error_pyerr(
        py,
        ValidationErrorArgs {
            message,
            verbose_message,
            schema_path,
            instance_path,
            kind,
            instance,
        },
    )
}

fn get_draft(draft: u8) -> PyResult<Draft> {
    match draft {
        DRAFT4 => Ok(Draft::Draft4),
        DRAFT6 => Ok(Draft::Draft6),
        DRAFT7 => Ok(Draft::Draft7),
        DRAFT201909 => Ok(Draft::Draft201909),
        DRAFT202012 => Ok(Draft::Draft202012),
        _ => Err(exceptions::PyValueError::new_err(format!(
            "Unknown draft: {draft}"
        ))),
    }
}

thread_local! {
    static LAST_FORMAT_ERROR: RefCell<Option<PyErr>> = const { RefCell::new(None) };
}

fn make_options(
    draft: Option<u8>,
    formats: Option<&Bound<'_, PyDict>>,
    validate_formats: Option<bool>,
    ignore_unknown_formats: Option<bool>,
    retriever: Option<&Bound<'_, PyAny>>,
    registry: Option<&registry::Registry>,
    base_uri: Option<String>,
    pattern_options: Option<&Bound<'_, PyAny>>,
) -> PyResult<jsonschema::ValidationOptions> {
    let mut options = jsonschema::options();
    if let Some(raw_draft_version) = draft {
        options = options.with_draft(get_draft(raw_draft_version)?);
    }
    if let Some(yes) = validate_formats {
        options = options.should_validate_formats(yes);
    }
    if let Some(yes) = ignore_unknown_formats {
        options = options.should_ignore_unknown_formats(yes);
    }
    if let Some(formats) = formats {
        for (name, callback) in formats.iter() {
            if !callback.is_callable() {
                return Err(exceptions::PyValueError::new_err(format!(
                    "Format checker for '{name}' must be a callable",
                )));
            }
            let callback: Py<PyAny> = callback.clone().unbind();
            let call_py_callback = move |value: &str| {
                Python::attach(|py| {
                    let value = PyString::new(py, value);
                    callback.call(py, (value,), None)?.is_truthy(py)
                })
            };
            options =
                options.with_format(name.to_string(), move |value: &str| match call_py_callback(
                    value,
                ) {
                    Ok(r) => r,
                    Err(e) => {
                        LAST_FORMAT_ERROR.with(|last| {
                            *last.borrow_mut() = Some(e);
                        });
                        std::panic::set_hook(Box::new(|_| {}));
                        // Should be caught
                        panic!("Format checker failed")
                    }
                });
        }
    }
    if let Some(retriever) = retriever {
        let func = into_retriever(retriever)?;
        options = options.with_retriever(Retriever { func });
    }
    if let Some(registry) = registry {
        options = options.with_registry(registry.inner.clone());
    }
    if let Some(base_uri) = base_uri {
        options = options.with_base_uri(base_uri);
    }
    if let Some(pattern_options) = pattern_options {
        if let Ok(fancy_options) = pattern_options.extract::<Py<FancyRegexOptions>>() {
            let pattern_options = Python::attach(|py| {
                let fancy_options = fancy_options.borrow(py);
                let mut pattern_options = jsonschema::PatternOptions::fancy_regex();

                if let Some(limit) = fancy_options.backtrack_limit {
                    pattern_options = pattern_options.backtrack_limit(limit);
                }
                if let Some(limit) = fancy_options.size_limit {
                    pattern_options = pattern_options.size_limit(limit);
                }
                if let Some(limit) = fancy_options.dfa_size_limit {
                    pattern_options = pattern_options.dfa_size_limit(limit);
                }
                pattern_options
            });
            options = options.with_pattern_options(pattern_options);
        } else if let Ok(regex_opts) = pattern_options.extract::<Py<RegexOptions>>() {
            let pattern_options = Python::attach(|py| {
                let regex_opts = regex_opts.borrow(py);
                let mut pattern_options = jsonschema::PatternOptions::regex();

                if let Some(limit) = regex_opts.size_limit {
                    pattern_options = pattern_options.size_limit(limit);
                }
                if let Some(limit) = regex_opts.dfa_size_limit {
                    pattern_options = pattern_options.dfa_size_limit(limit);
                }
                pattern_options
            });
            options = options.with_pattern_options(pattern_options);
        } else {
            return Err(exceptions::PyTypeError::new_err(
                "pattern_options must be an instance of FancyRegexOptions or RegexOptions",
            ));
        }
    }
    Ok(options)
}

fn iter_on_error(
    py: Python<'_>,
    validator: &jsonschema::Validator,
    instance: &Bound<'_, PyAny>,
    mask: Option<&str>,
) -> PyResult<ValidationErrorIter> {
    let instance = ser::to_value(instance)?;
    let mut pyerrors = vec![];

    panic::catch_unwind(AssertUnwindSafe(|| {
        for error in validator.iter_errors(&instance) {
            pyerrors.push(into_py_err(py, error, mask)?);
        }
        PyResult::Ok(())
    }))
    .map_err(handle_format_checked_panic)??;
    Ok(ValidationErrorIter {
        iter: pyerrors.into_iter(),
    })
}

fn raise_on_error(
    py: Python<'_>,
    validator: &jsonschema::Validator,
    instance: &Bound<'_, PyAny>,
    mask: Option<&str>,
) -> PyResult<()> {
    let instance = ser::to_value(instance)?;
    let error = panic::catch_unwind(AssertUnwindSafe(|| validator.validate(&instance)))
        .map_err(handle_format_checked_panic)?
        .err();
    error.map_or_else(|| Ok(()), |err| Err(into_py_err(py, err, mask)?))
}

fn is_ascii_number(s: &str) -> bool {
    !s.is_empty() && s.as_bytes().iter().all(|&b| b.is_ascii_digit())
}

fn to_error_message(
    error: &jsonschema::ValidationError<'_>,
    mut message: String,
    mask: Option<&str>,
) -> String {
    // It roughly doubles
    message.reserve(message.len());
    message.push('\n');
    message.push('\n');
    message.push_str("Failed validating");

    let push_segment = |m: &mut String, segment: &str| {
        if is_ascii_number(segment) {
            m.push_str(segment);
        } else {
            m.push('"');
            m.push_str(segment);
            m.push('"');
        }
    };

    let mut schema_path = error.schema_path.as_str();

    if let Some((rest, last)) = schema_path.rsplit_once('/') {
        message.push(' ');
        push_segment(&mut message, last);
        schema_path = rest;
    }
    message.push_str(" in schema");
    for segment in schema_path.split('/').skip(1) {
        message.push('[');
        push_segment(&mut message, segment);
        message.push(']');
    }
    message.push('\n');
    message.push('\n');
    message.push_str("On instance");
    for segment in error.instance_path.as_str().split('/').skip(1) {
        message.push('[');
        push_segment(&mut message, segment);
        message.push(']');
    }
    message.push(':');
    message.push_str("\n    ");
    if let Some(mask) = mask {
        message.push_str(mask);
    } else {
        let mut writer = StringWriter(&mut message);
        serde_json::to_writer(&mut writer, &error.instance).expect("Failed to serialize JSON");
    }
    message
}

struct StringWriter<'a>(&'a mut String);

impl Write for StringWriter<'_> {
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

/// is_valid(schema, instance, draft=None, formats=None, validate_formats=None, ignore_unknown_formats=True, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None)
///
/// A shortcut for validating the input instance against the schema.
///
///     >>> is_valid({"minimum": 5}, 3)
///     False
///
/// If your workflow implies validating against the same schema, consider using `validator_for(...).is_valid`
/// instead.
#[pyfunction]
#[pyo3(signature = (schema, instance, draft=None, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None))]
#[allow(clippy::needless_pass_by_value)]
fn is_valid(
    py: Python<'_>,
    schema: &Bound<'_, PyAny>,
    instance: &Bound<'_, PyAny>,
    draft: Option<u8>,
    formats: Option<&Bound<'_, PyDict>>,
    validate_formats: Option<bool>,
    ignore_unknown_formats: Option<bool>,
    retriever: Option<&Bound<'_, PyAny>>,
    registry: Option<&registry::Registry>,
    mask: Option<String>,
    base_uri: Option<String>,
    pattern_options: Option<&Bound<'_, PyAny>>,
) -> PyResult<bool> {
    let options = make_options(
        draft,
        formats,
        validate_formats,
        ignore_unknown_formats,
        retriever,
        registry,
        base_uri,
        pattern_options,
    )?;
    let schema = ser::to_value(schema)?;
    match options.build(&schema) {
        Ok(validator) => {
            let instance = ser::to_value(instance)?;
            panic::catch_unwind(AssertUnwindSafe(|| Ok(validator.is_valid(&instance))))
                .map_err(handle_format_checked_panic)?
        }
        Err(error) => Err(into_py_err(py, error, mask.as_deref())?),
    }
}

/// validate(schema, instance, draft=None, formats=None, validate_formats=None, ignore_unknown_formats=True, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None)
///
/// Validate the input instance and raise `ValidationError` in the error case
///
///     >>> validate({"minimum": 5}, 3)
///     ...
///     ValidationError: 3 is less than the minimum of 5
///
/// If the input instance is invalid, only the first occurred error is raised.
/// If your workflow implies validating against the same schema, consider using `validator_for(...).validate`
/// instead.
#[pyfunction]
#[pyo3(signature = (schema, instance, draft=None, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None))]
#[allow(clippy::needless_pass_by_value)]
fn validate(
    py: Python<'_>,
    schema: &Bound<'_, PyAny>,
    instance: &Bound<'_, PyAny>,
    draft: Option<u8>,
    formats: Option<&Bound<'_, PyDict>>,
    validate_formats: Option<bool>,
    ignore_unknown_formats: Option<bool>,
    retriever: Option<&Bound<'_, PyAny>>,
    registry: Option<&registry::Registry>,
    mask: Option<String>,
    base_uri: Option<String>,
    pattern_options: Option<&Bound<'_, PyAny>>,
) -> PyResult<()> {
    let options = make_options(
        draft,
        formats,
        validate_formats,
        ignore_unknown_formats,
        retriever,
        registry,
        base_uri,
        pattern_options,
    )?;
    let schema = ser::to_value(schema)?;
    match options.build(&schema) {
        Ok(validator) => raise_on_error(py, &validator, instance, mask.as_deref()),
        Err(error) => Err(into_py_err(py, error, mask.as_deref())?),
    }
}

/// iter_errors(schema, instance, draft=None, formats=None, validate_formats=None, ignore_unknown_formats=True, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None)
///
/// Iterate the validation errors of the input instance
///
///     >>> next(iter_errors({"minimum": 5}, 3))
///     ...
///     ValidationError: 3 is less than the minimum of 5
///
/// If your workflow implies validating against the same schema, consider using `validator_for().iter_errors`
/// instead.
#[pyfunction]
#[pyo3(signature = (schema, instance, draft=None, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None))]
#[allow(clippy::needless_pass_by_value)]
fn iter_errors(
    py: Python<'_>,
    schema: &Bound<'_, PyAny>,
    instance: &Bound<'_, PyAny>,
    draft: Option<u8>,
    formats: Option<&Bound<'_, PyDict>>,
    validate_formats: Option<bool>,
    ignore_unknown_formats: Option<bool>,
    retriever: Option<&Bound<'_, PyAny>>,
    registry: Option<&registry::Registry>,
    mask: Option<String>,
    base_uri: Option<String>,
    pattern_options: Option<&Bound<'_, PyAny>>,
) -> PyResult<ValidationErrorIter> {
    let options = make_options(
        draft,
        formats,
        validate_formats,
        ignore_unknown_formats,
        retriever,
        registry,
        base_uri,
        pattern_options,
    )?;
    let schema = ser::to_value(schema)?;
    match options.build(&schema) {
        Ok(validator) => iter_on_error(py, &validator, instance, mask.as_deref()),
        Err(error) => Err(into_py_err(py, error, mask.as_deref())?),
    }
}

#[allow(clippy::needless_pass_by_value)]
fn handle_format_checked_panic(err: Box<dyn Any + Send>) -> PyErr {
    LAST_FORMAT_ERROR.with(|last| {
        if let Some(err) = last.borrow_mut().take() {
            let _ = panic::take_hook();
            err
        } else {
            exceptions::PyRuntimeError::new_err(format!("Validation panicked: {err:?}"))
        }
    })
}

#[pyclass(module = "jsonschema_rs", subclass)]
struct Validator {
    validator: jsonschema::Validator,
    mask: Option<String>,
}

/// validator_for(schema, formats=None, validate_formats=None, ignore_unknown_formats=True, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None)
///
/// Create a validator for the input schema with automatic draft detection and default options.
///
///     >>> validator = validator_for({"minimum": 5})
///     >>> validator.is_valid(3)
///     False
///
#[pyfunction]
#[pyo3(signature = (schema, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None))]
fn validator_for(
    py: Python<'_>,
    schema: &Bound<'_, PyAny>,
    formats: Option<&Bound<'_, PyDict>>,
    validate_formats: Option<bool>,
    ignore_unknown_formats: Option<bool>,
    retriever: Option<&Bound<'_, PyAny>>,
    registry: Option<&registry::Registry>,
    mask: Option<String>,
    base_uri: Option<String>,
    pattern_options: Option<&Bound<'_, PyAny>>,
) -> PyResult<Validator> {
    validator_for_impl(
        py,
        schema,
        None,
        formats,
        validate_formats,
        ignore_unknown_formats,
        retriever,
        registry,
        mask,
        base_uri,
        pattern_options,
    )
}

fn validator_for_impl(
    py: Python<'_>,
    schema: &Bound<'_, PyAny>,
    draft: Option<u8>,
    formats: Option<&Bound<'_, PyDict>>,
    validate_formats: Option<bool>,
    ignore_unknown_formats: Option<bool>,
    retriever: Option<&Bound<'_, PyAny>>,
    registry: Option<&registry::Registry>,
    mask: Option<String>,
    base_uri: Option<String>,
    pattern_options: Option<&Bound<'_, PyAny>>,
) -> PyResult<Validator> {
    let obj_ptr = schema.as_ptr();
    let object_type = unsafe { pyo3::ffi::Py_TYPE(obj_ptr) };
    let schema = if unsafe { object_type == types::STR_TYPE } {
        let mut str_size: pyo3::ffi::Py_ssize_t = 0;
        let ptr = unsafe { PyUnicode_AsUTF8AndSize(obj_ptr, &raw mut str_size) };
        let slice = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), str_size as usize) };
        serde_json::from_slice(slice)
            .map_err(|error| PyValueError::new_err(format!("Invalid string: {error}")))?
    } else {
        ser::to_value(schema)?
    };
    let options = make_options(
        draft,
        formats,
        validate_formats,
        ignore_unknown_formats,
        retriever,
        registry,
        base_uri,
        pattern_options,
    )?;
    match options.build(&schema) {
        Ok(validator) => Ok(Validator { validator, mask }),
        Err(error) => Err(into_py_err(py, error, mask.as_deref())?),
    }
}

#[pymethods]
impl Validator {
    #[new]
    #[pyo3(signature = (schema, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, registry = None, mask=None, base_uri=None, pattern_options=None))]
    fn new(
        py: Python<'_>,
        schema: &Bound<'_, PyAny>,
        formats: Option<&Bound<'_, PyDict>>,
        validate_formats: Option<bool>,
        ignore_unknown_formats: Option<bool>,
        retriever: Option<&Bound<'_, PyAny>>,
        registry: Option<&registry::Registry>,
        mask: Option<String>,
        base_uri: Option<String>,
        pattern_options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        validator_for(
            py,
            schema,
            formats,
            validate_formats,
            ignore_unknown_formats,
            retriever,
            registry,
            mask,
            base_uri,
            pattern_options,
        )
    }
    /// is_valid(instance)
    ///
    /// Perform fast validation against the schema.
    ///
    ///     >>> validator = validator_for({"minimum": 5})
    ///     >>> validator.is_valid(3)
    ///     False
    ///
    /// The output is a boolean value, that indicates whether the instance is valid or not.
    #[pyo3(text_signature = "(instance)")]
    fn is_valid(&self, instance: &Bound<'_, PyAny>) -> PyResult<bool> {
        let instance = ser::to_value(instance)?;
        panic::catch_unwind(AssertUnwindSafe(|| Ok(self.validator.is_valid(&instance))))
            .map_err(handle_format_checked_panic)?
    }
    /// validate(instance)
    ///
    /// Validate the input instance and raise `ValidationError` in the error case
    ///
    ///     >>> validator = validator_for({"minimum": 5})
    ///     >>> validator.validate(3)
    ///     ...
    ///     ValidationError: 3 is less than the minimum of 5
    ///
    /// If the input instance is invalid, only the first occurred error is raised.
    #[pyo3(text_signature = "(instance)")]
    fn validate(&self, py: Python<'_>, instance: &Bound<'_, PyAny>) -> PyResult<()> {
        raise_on_error(py, &self.validator, instance, self.mask.as_deref())
    }
    /// iter_errors(instance)
    ///
    /// Iterate the validation errors of the input instance
    ///
    ///     >>> validator = validator_for({"minimum": 5})
    ///     >>> next(validator.iter_errors(3))
    ///     ...
    ///     ValidationError: 3 is less than the minimum of 5
    #[pyo3(text_signature = "(instance)")]
    fn iter_errors(
        &self,
        py: Python<'_>,
        instance: &Bound<'_, PyAny>,
    ) -> PyResult<ValidationErrorIter> {
        iter_on_error(py, &self.validator, instance, self.mask.as_deref())
    }
    fn __repr__(&self) -> &'static str {
        match self.validator.draft() {
            Draft::Draft4 => "<Draft4Validator>",
            Draft::Draft6 => "<Draft6Validator>",
            Draft::Draft7 => "<Draft7Validator>",
            Draft::Draft201909 => "<Draft201909Validator>",
            Draft::Draft202012 => "<Draft202012Validator>",
            _ => "Unknown",
        }
    }
}

/// Draft4Validator(schema, formats=None, validate_formats=None, ignore_unknown_formats=True, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None)
///
/// A JSON Schema Draft 4 validator.
///
///     >>> validator = Draft4Validator({"minimum": 5})
///     >>> validator.is_valid(3)
///     False
///
#[pyclass(module = "jsonschema_rs", extends=Validator, subclass)]
struct Draft4Validator;

#[pymethods]
impl Draft4Validator {
    #[new]
    #[pyo3(signature = (schema, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None))]
    fn new(
        py: Python<'_>,
        schema: &Bound<'_, PyAny>,
        formats: Option<&Bound<'_, PyDict>>,
        validate_formats: Option<bool>,
        ignore_unknown_formats: Option<bool>,
        retriever: Option<&Bound<'_, PyAny>>,
        registry: Option<&registry::Registry>,
        mask: Option<String>,
        base_uri: Option<String>,
        pattern_options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<(Self, Validator)> {
        Ok((
            Draft4Validator {},
            validator_for_impl(
                py,
                schema,
                Some(DRAFT4),
                formats,
                validate_formats,
                ignore_unknown_formats,
                retriever,
                registry,
                mask,
                base_uri,
                pattern_options,
            )?,
        ))
    }
}

/// Draft6Validator(schema, formats=None, validate_formats=None, ignore_unknown_formats=True, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None)
///
/// A JSON Schema Draft 6 validator.
///
///     >>> validator = Draft6Validator({"minimum": 5})
///     >>> validator.is_valid(3)
///     False
///
#[pyclass(module = "jsonschema_rs", extends=Validator, subclass)]
struct Draft6Validator;

#[pymethods]
impl Draft6Validator {
    #[new]
    #[pyo3(signature = (schema, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None))]
    fn new(
        py: Python<'_>,
        schema: &Bound<'_, PyAny>,
        formats: Option<&Bound<'_, PyDict>>,
        validate_formats: Option<bool>,
        ignore_unknown_formats: Option<bool>,
        retriever: Option<&Bound<'_, PyAny>>,
        registry: Option<&registry::Registry>,
        mask: Option<String>,
        base_uri: Option<String>,
        pattern_options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<(Self, Validator)> {
        Ok((
            Draft6Validator {},
            validator_for_impl(
                py,
                schema,
                Some(DRAFT6),
                formats,
                validate_formats,
                ignore_unknown_formats,
                retriever,
                registry,
                mask,
                base_uri,
                pattern_options,
            )?,
        ))
    }
}

/// Draft7Validator(schema, formats=None, validate_formats=None, ignore_unknown_formats=True, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None)
///
/// A JSON Schema Draft 7 validator.
///
///     >>> validator = Draft7Validator({"minimum": 5})
///     >>> validator.is_valid(3)
///     False
///
#[pyclass(module = "jsonschema_rs", extends=Validator, subclass)]
struct Draft7Validator;

#[pymethods]
impl Draft7Validator {
    #[new]
    #[pyo3(signature = (schema, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None))]
    fn new(
        py: Python<'_>,
        schema: &Bound<'_, PyAny>,
        formats: Option<&Bound<'_, PyDict>>,
        validate_formats: Option<bool>,
        ignore_unknown_formats: Option<bool>,
        retriever: Option<&Bound<'_, PyAny>>,
        registry: Option<&registry::Registry>,
        mask: Option<String>,
        base_uri: Option<String>,
        pattern_options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<(Self, Validator)> {
        Ok((
            Draft7Validator {},
            validator_for_impl(
                py,
                schema,
                Some(DRAFT7),
                formats,
                validate_formats,
                ignore_unknown_formats,
                retriever,
                registry,
                mask,
                base_uri,
                pattern_options,
            )?,
        ))
    }
}

/// Draft201909Validator(schema, formats=None, validate_formats=None, ignore_unknown_formats=True, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None)
///
/// A JSON Schema Draft 2019-09 validator.
///
///     >>> validator = Draft201909Validator({"minimum": 5})
///     >>> validator.is_valid(3)
///     False
///
#[pyclass(module = "jsonschema_rs", extends=Validator, subclass)]
struct Draft201909Validator;

#[pymethods]
impl Draft201909Validator {
    #[new]
    #[pyo3(signature = (schema, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None))]
    fn new(
        py: Python<'_>,
        schema: &Bound<'_, PyAny>,
        formats: Option<&Bound<'_, PyDict>>,
        validate_formats: Option<bool>,
        ignore_unknown_formats: Option<bool>,
        retriever: Option<&Bound<'_, PyAny>>,
        registry: Option<&registry::Registry>,
        mask: Option<String>,
        base_uri: Option<String>,
        pattern_options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<(Self, Validator)> {
        Ok((
            Draft201909Validator {},
            validator_for_impl(
                py,
                schema,
                Some(DRAFT201909),
                formats,
                validate_formats,
                ignore_unknown_formats,
                retriever,
                registry,
                mask,
                base_uri,
                pattern_options,
            )?,
        ))
    }
}

/// Draft202012Validator(schema, formats=None, validate_formats=None, ignore_unknown_formats=True, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None)
///
/// A JSON Schema Draft 2020-12 validator.
///
///     >>> validator = Draft202012Validator({"minimum": 5})
///     >>> validator.is_valid(3)
///     False
///
#[pyclass(module = "jsonschema_rs", extends=Validator, subclass)]
struct Draft202012Validator;

#[pymethods]
impl Draft202012Validator {
    #[new]
    #[pyo3(signature = (schema, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None))]
    fn new(
        py: Python<'_>,
        schema: &Bound<'_, PyAny>,
        formats: Option<&Bound<'_, PyDict>>,
        validate_formats: Option<bool>,
        ignore_unknown_formats: Option<bool>,
        retriever: Option<&Bound<'_, PyAny>>,
        registry: Option<&registry::Registry>,
        mask: Option<String>,
        base_uri: Option<String>,
        pattern_options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<(Self, Validator)> {
        Ok((
            Draft202012Validator {},
            validator_for_impl(
                py,
                schema,
                Some(DRAFT202012),
                formats,
                validate_formats,
                ignore_unknown_formats,
                retriever,
                registry,
                mask,
                base_uri,
                pattern_options,
            )?,
        ))
    }
}

#[allow(dead_code)]
mod build {
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}

/// Meta-schema validation
mod meta {
    use super::referencing_error_pyerr;
    use pyo3::prelude::*;

    /// is_valid(schema, registry=None)
    ///
    /// Validate a JSON Schema document against its meta-schema. Draft version is detected automatically.
    /// Schemas with unknown `$schema` values raise `jsonschema_rs.ReferencingError`.
    ///
    ///     >>> jsonschema_rs.meta.is_valid({"type": "string"})
    ///     True
    ///     >>> jsonschema_rs.meta.is_valid({"type": "invalid_type"})
    ///     False
    ///
    /// For custom meta-schemas, provide a registry:
    ///
    ///     >>> custom_meta = {"$schema": "https://json-schema.org/draft/2020-12/schema", "type": "object"}
    ///     >>> registry = Registry([("http://example.com/meta", custom_meta)])
    ///     >>> schema = {"$schema": "http://example.com/meta", "customKeyword": "value"}
    ///     >>> jsonschema_rs.meta.is_valid(schema, registry=registry)
    ///     True
    ///
    #[pyfunction]
    #[pyo3(signature = (schema, registry=None))]
    pub(crate) fn is_valid(
        py: Python<'_>,
        schema: &Bound<'_, PyAny>,
        registry: Option<&crate::registry::Registry>,
    ) -> PyResult<bool> {
        let schema = crate::ser::to_value(schema)?;
        let result = if let Some(registry) = registry {
            jsonschema::meta::options()
                .with_registry(registry.inner.clone())
                .validate(&schema)
        } else {
            jsonschema::meta::validate(&schema)
        };

        match result {
            Ok(()) => Ok(true),
            Err(error) => {
                if let jsonschema::error::ValidationErrorKind::Referencing(err) = &error.kind {
                    return Err(referencing_error_pyerr(py, err.to_string())?);
                }
                Ok(false)
            }
        }
    }

    /// validate(schema, registry=None)
    ///
    /// Validate a JSON Schema document against its meta-schema and raise ValidationError if invalid.
    /// Draft version is detected automatically. Schemas with custom/unknown `$schema` values raise `jsonschema_rs.ReferencingError`.
    ///
    ///     >>> jsonschema_rs.meta.validate({"type": "string"})
    ///     >>> jsonschema_rs.meta.validate({"type": "invalid_type"})
    ///     Traceback (most recent call last):
    ///         ...
    ///     jsonschema_rs.ValidationError: ...
    ///     >>> jsonschema_rs.meta.validate({"$schema": "http://custom.example.com/schema", "type": "object"})
    ///     Traceback (most recent call last):
    ///         ...
    ///     jsonschema_rs.ReferencingError: Unknown meta-schema: http://custom.example.com/schema
    ///
    /// For custom meta-schemas, provide a registry:
    ///
    ///     >>> custom_meta = {"$schema": "https://json-schema.org/draft/2020-12/schema", "type": "object"}
    ///     >>> registry = Registry([("http://example.com/meta", custom_meta)])
    ///     >>> schema = {"$schema": "http://example.com/meta", "customKeyword": "value"}
    ///     >>> jsonschema_rs.meta.validate(schema, registry=registry)
    ///
    #[pyfunction]
    #[pyo3(signature = (schema, registry=None))]
    pub(crate) fn validate(
        py: Python<'_>,
        schema: &Bound<'_, PyAny>,
        registry: Option<&crate::registry::Registry>,
    ) -> PyResult<()> {
        let schema = crate::ser::to_value(schema)?;
        let result = if let Some(registry) = registry {
            jsonschema::meta::options()
                .with_registry(registry.inner.clone())
                .validate(&schema)
        } else {
            jsonschema::meta::validate(&schema)
        };

        match result {
            Ok(()) => Ok(()),
            Err(error) => {
                if let jsonschema::error::ValidationErrorKind::Referencing(err) = &error.kind {
                    return Err(referencing_error_pyerr(py, err.to_string())?);
                }
                Err(crate::into_py_err(py, error, None)?)
            }
        }
    }
}

/// JSON Schema validation for Python written in Rust.
#[pymodule]
fn jsonschema_rs(py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    // To provide proper signatures for PyCharm, all the functions have their signatures as the
    // first line in docstrings. The idea is taken from NumPy.
    types::init(py);
    module.add_wrapped(wrap_pyfunction!(is_valid))?;
    module.add_wrapped(wrap_pyfunction!(validate))?;
    module.add_wrapped(wrap_pyfunction!(iter_errors))?;
    module.add_wrapped(wrap_pyfunction!(validator_for))?;
    module.add_class::<Draft4Validator>()?;
    module.add_class::<Draft6Validator>()?;
    module.add_class::<Draft7Validator>()?;
    module.add_class::<Draft201909Validator>()?;
    module.add_class::<Draft202012Validator>()?;
    module.add_class::<registry::Registry>()?;
    module.add_class::<FancyRegexOptions>()?;
    module.add_class::<RegexOptions>()?;
    module.add("ValidationErrorKind", py.get_type::<ValidationErrorKind>())?;
    module.add("Draft4", DRAFT4)?;
    module.add("Draft6", DRAFT6)?;
    module.add("Draft7", DRAFT7)?;
    module.add("Draft201909", DRAFT201909)?;
    module.add("Draft202012", DRAFT202012)?;

    let meta = PyModule::new(py, "meta")?;
    meta.add_function(wrap_pyfunction!(meta::is_valid, &meta)?)?;
    meta.add_function(wrap_pyfunction!(meta::validate, &meta)?)?;
    module.add_submodule(&meta)?;

    // Add build metadata to ease triaging incoming issues
    module.add("__build__", pyo3_built::pyo3_built!(py, build))?;

    Ok(())
}
