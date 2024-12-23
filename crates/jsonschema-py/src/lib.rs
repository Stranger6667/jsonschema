use std::{
    any::Any,
    cell::RefCell,
    io::Write,
    panic::{self, AssertUnwindSafe},
};

use jsonschema::{Draft, Retrieve, Uri};
use pyo3::{
    exceptions::{self, PyValueError},
    ffi::PyUnicode_AsUTF8AndSize,
    prelude::*,
    types::{PyAny, PyDict, PyList, PyString, PyType},
    wrap_pyfunction,
};
use ser::to_value;
use serde_json::Value;
#[macro_use]
extern crate pyo3_built;

mod ffi;
mod ser;
mod types;

const DRAFT7: u8 = 7;
const DRAFT6: u8 = 6;
const DRAFT4: u8 = 4;
const DRAFT201909: u8 = 19;
const DRAFT202012: u8 = 20;

/// An instance is invalid under a provided schema.
#[pyclass(extends=exceptions::PyValueError, module="jsonschema_rs")]
#[derive(Debug)]
struct ValidationError {
    #[pyo3(get)]
    message: String,
    verbose_message: String,
    #[pyo3(get)]
    schema_path: Py<PyList>,
    #[pyo3(get)]
    instance_path: Py<PyList>,
    #[pyo3(get)]
    kind: ValidationErrorKind,
    #[pyo3(get)]
    instance: PyObject,
}

#[pymethods]
impl ValidationError {
    #[new]
    fn new(
        message: String,
        long_message: String,
        schema_path: Py<PyList>,
        instance_path: Py<PyList>,
        kind: ValidationErrorKind,
        instance: PyObject,
    ) -> Self {
        ValidationError {
            message,
            verbose_message: long_message,
            schema_path,
            instance_path,
            kind,
            instance,
        }
    }
    fn __str__(&self) -> String {
        self.verbose_message.clone()
    }
    fn __repr__(&self) -> String {
        format!("<ValidationError: '{}'>", self.message)
    }
}

#[pyclass(eq, eq_int)]
#[derive(Debug, PartialEq, Clone)]
enum ValidationErrorKind {
    AdditionalItems,
    AdditionalProperties,
    AnyOf,
    BacktrackLimitExceeded,
    Constant,
    Contains,
    ContentEncoding,
    ContentMediaType,
    Custom,
    Enum,
    ExclusiveMaximum,
    ExclusiveMinimum,
    FalseSchema,
    Format,
    FromUtf8,
    MaxItems,
    Maximum,
    MaxLength,
    MaxProperties,
    MinItems,
    Minimum,
    MinLength,
    MinProperties,
    MultipleOf,
    Not,
    OneOfMultipleValid,
    OneOfNotValid,
    Pattern,
    PropertyNames,
    Required,
    Type,
    UnevaluatedItems,
    UnevaluatedProperties,
    UniqueItems,
    Referencing,
}

impl From<jsonschema::error::ValidationErrorKind> for ValidationErrorKind {
    fn from(kind: jsonschema::error::ValidationErrorKind) -> Self {
        match kind {
            jsonschema::error::ValidationErrorKind::AdditionalItems { .. } => {
                ValidationErrorKind::AdditionalItems
            }
            jsonschema::error::ValidationErrorKind::AdditionalProperties { .. } => {
                ValidationErrorKind::AdditionalProperties
            }
            jsonschema::error::ValidationErrorKind::AnyOf => ValidationErrorKind::AnyOf,
            jsonschema::error::ValidationErrorKind::BacktrackLimitExceeded { .. } => {
                ValidationErrorKind::BacktrackLimitExceeded
            }
            jsonschema::error::ValidationErrorKind::Constant { .. } => {
                ValidationErrorKind::Constant
            }
            jsonschema::error::ValidationErrorKind::Contains => ValidationErrorKind::Contains,
            jsonschema::error::ValidationErrorKind::ContentEncoding { .. } => {
                ValidationErrorKind::ContentEncoding
            }
            jsonschema::error::ValidationErrorKind::ContentMediaType { .. } => {
                ValidationErrorKind::ContentMediaType
            }
            jsonschema::error::ValidationErrorKind::Custom { .. } => ValidationErrorKind::Custom,
            jsonschema::error::ValidationErrorKind::Enum { .. } => ValidationErrorKind::Enum,
            jsonschema::error::ValidationErrorKind::ExclusiveMaximum { .. } => {
                ValidationErrorKind::ExclusiveMaximum
            }
            jsonschema::error::ValidationErrorKind::ExclusiveMinimum { .. } => {
                ValidationErrorKind::ExclusiveMinimum
            }
            jsonschema::error::ValidationErrorKind::FalseSchema => ValidationErrorKind::FalseSchema,
            jsonschema::error::ValidationErrorKind::Format { .. } => ValidationErrorKind::Format,
            jsonschema::error::ValidationErrorKind::FromUtf8 { .. } => {
                ValidationErrorKind::FromUtf8
            }
            jsonschema::error::ValidationErrorKind::MaxItems { .. } => {
                ValidationErrorKind::MaxItems
            }
            jsonschema::error::ValidationErrorKind::Maximum { .. } => ValidationErrorKind::Maximum,
            jsonschema::error::ValidationErrorKind::MaxLength { .. } => {
                ValidationErrorKind::MaxLength
            }
            jsonschema::error::ValidationErrorKind::MaxProperties { .. } => {
                ValidationErrorKind::MaxProperties
            }
            jsonschema::error::ValidationErrorKind::MinItems { .. } => {
                ValidationErrorKind::MinItems
            }
            jsonschema::error::ValidationErrorKind::Minimum { .. } => ValidationErrorKind::Minimum,
            jsonschema::error::ValidationErrorKind::MinLength { .. } => {
                ValidationErrorKind::MinLength
            }
            jsonschema::error::ValidationErrorKind::MinProperties { .. } => {
                ValidationErrorKind::MinProperties
            }
            jsonschema::error::ValidationErrorKind::MultipleOf { .. } => {
                ValidationErrorKind::MultipleOf
            }
            jsonschema::error::ValidationErrorKind::Not { .. } => ValidationErrorKind::Not,
            jsonschema::error::ValidationErrorKind::OneOfMultipleValid => {
                ValidationErrorKind::OneOfMultipleValid
            }
            jsonschema::error::ValidationErrorKind::OneOfNotValid => {
                ValidationErrorKind::OneOfNotValid
            }
            jsonschema::error::ValidationErrorKind::Pattern { .. } => ValidationErrorKind::Pattern,
            jsonschema::error::ValidationErrorKind::PropertyNames { .. } => {
                ValidationErrorKind::PropertyNames
            }
            jsonschema::error::ValidationErrorKind::Required { .. } => {
                ValidationErrorKind::Required
            }
            jsonschema::error::ValidationErrorKind::Type { .. } => ValidationErrorKind::Type,
            jsonschema::error::ValidationErrorKind::UnevaluatedItems { .. } => {
                ValidationErrorKind::UnevaluatedItems
            }
            jsonschema::error::ValidationErrorKind::UnevaluatedProperties { .. } => {
                ValidationErrorKind::UnevaluatedProperties
            }
            jsonschema::error::ValidationErrorKind::UniqueItems => ValidationErrorKind::UniqueItems,
            jsonschema::error::ValidationErrorKind::Referencing(_) => {
                ValidationErrorKind::Referencing
            }
        }
    }
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

fn into_py_err(
    py: Python<'_>,
    error: jsonschema::ValidationError<'_>,
    mask: Option<&str>,
) -> PyResult<PyErr> {
    let pyerror_type = PyType::new::<ValidationError>(py);
    let message = if let Some(mask) = mask {
        error.masked_with(mask).to_string()
    } else {
        error.to_string()
    };
    let verbose_message = to_error_message(&error, message.clone(), mask);
    let into_path = |segment: &str| {
        if let Ok(idx) = segment.parse::<usize>() {
            idx.into_pyobject(py).and_then(PyObject::try_from)
        } else {
            segment.into_pyobject(py).and_then(PyObject::try_from)
        }
    };
    let elements = error
        .schema_path
        .as_str()
        .split('/')
        .skip(1)
        .map(into_path)
        .collect::<Result<Vec<_>, _>>()?;
    let schema_path = PyList::new(py, elements)?.unbind();
    let elements = error
        .instance_path
        .as_str()
        .split('/')
        .skip(1)
        .map(into_path)
        .collect::<Result<Vec<_>, _>>()?;
    let instance_path = PyList::new(py, elements)?.unbind();
    let kind: ValidationErrorKind = error.kind.into();
    let instance = pythonize::pythonize(py, error.instance.as_ref())?.unbind();
    Ok(PyErr::from_type(
        pyerror_type,
        (
            message,
            verbose_message,
            schema_path,
            instance_path,
            kind,
            instance,
        ),
    ))
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
) -> PyResult<jsonschema::ValidationOptions> {
    let mut options = jsonschema::options();
    if let Some(raw_draft_version) = draft {
        options.with_draft(get_draft(raw_draft_version)?);
    }
    if let Some(yes) = validate_formats {
        options.should_validate_formats(yes);
    }
    if let Some(yes) = ignore_unknown_formats {
        options.should_ignore_unknown_formats(yes);
    }
    if let Some(formats) = formats {
        for (name, callback) in formats.iter() {
            if !callback.is_callable() {
                return Err(exceptions::PyValueError::new_err(format!(
                    "Format checker for '{}' must be a callable",
                    name
                )));
            }
            let callback: Py<PyAny> = callback.clone().unbind();
            let call_py_callback = move |value: &str| {
                Python::with_gil(|py| {
                    let value = PyString::new(py, value);
                    callback.call(py, (value,), None)?.is_truthy(py)
                })
            };
            options.with_format(
                name.to_string(),
                move |value: &str| match call_py_callback(value) {
                    Ok(r) => r,
                    Err(e) => {
                        LAST_FORMAT_ERROR.with(|last| {
                            *last.borrow_mut() = Some(e);
                        });
                        std::panic::set_hook(Box::new(|_| {}));
                        // Should be caught
                        panic!("Format checker failed")
                    }
                },
            );
        }
    }
    if let Some(retriever) = retriever {
        if !retriever.is_callable() {
            return Err(exceptions::PyValueError::new_err(
                "External resource retriever must be a callable",
            ));
        }
        let retriever: Py<PyAny> = retriever.clone().unbind();

        let call_py_retriever = move |value: &str| {
            Python::with_gil(|py| {
                let value = PyString::new(py, value);
                retriever
                    .call(py, (value,), None)
                    .and_then(|value| to_value(value.bind(py)))
            })
        };

        struct Retriever<T> {
            func: T,
        }

        impl<T: Send + Sync + Fn(&str) -> PyResult<Value>> Retrieve for Retriever<T> {
            fn retrieve(
                &self,
                uri: &Uri<&str>,
            ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
                Ok((self.func)(uri.as_str())?)
            }
        }

        options.with_retriever(Retriever {
            func: call_py_retriever,
        });
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

/// is_valid(schema, instance, draft=None, formats=None, validate_formats=None, ignore_unknown_formats=True, retriever=None, mask=None)
///
/// A shortcut for validating the input instance against the schema.
///
///     >>> is_valid({"minimum": 5}, 3)
///     False
///
/// If your workflow implies validating against the same schema, consider using `validator_for(...).is_valid`
/// instead.
#[pyfunction]
#[allow(unused_variables, clippy::too_many_arguments)]
#[pyo3(signature = (schema, instance, draft=None, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, mask=None))]
fn is_valid(
    py: Python<'_>,
    schema: &Bound<'_, PyAny>,
    instance: &Bound<'_, PyAny>,
    draft: Option<u8>,
    formats: Option<&Bound<'_, PyDict>>,
    validate_formats: Option<bool>,
    ignore_unknown_formats: Option<bool>,
    retriever: Option<&Bound<'_, PyAny>>,
    mask: Option<String>,
) -> PyResult<bool> {
    let options = make_options(
        draft,
        formats,
        validate_formats,
        ignore_unknown_formats,
        retriever,
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

/// validate(schema, instance, draft=None, formats=None, validate_formats=None, ignore_unknown_formats=True, retriever=None, mask=None)
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
#[allow(unused_variables, clippy::too_many_arguments)]
#[pyo3(signature = (schema, instance, draft=None, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, mask=None))]
fn validate(
    py: Python<'_>,
    schema: &Bound<'_, PyAny>,
    instance: &Bound<'_, PyAny>,
    draft: Option<u8>,
    formats: Option<&Bound<'_, PyDict>>,
    validate_formats: Option<bool>,
    ignore_unknown_formats: Option<bool>,
    retriever: Option<&Bound<'_, PyAny>>,
    mask: Option<String>,
) -> PyResult<()> {
    let options = make_options(
        draft,
        formats,
        validate_formats,
        ignore_unknown_formats,
        retriever,
    )?;
    let schema = ser::to_value(schema)?;
    match options.build(&schema) {
        Ok(validator) => raise_on_error(py, &validator, instance, mask.as_deref()),
        Err(error) => Err(into_py_err(py, error, mask.as_deref())?),
    }
}

/// iter_errors(schema, instance, draft=None, formats=None, validate_formats=None, ignore_unknown_formats=True, retriever=None, mask=None)
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
#[allow(unused_variables, clippy::too_many_arguments)]
#[pyo3(signature = (schema, instance, draft=None, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, mask=None))]
fn iter_errors(
    py: Python<'_>,
    schema: &Bound<'_, PyAny>,
    instance: &Bound<'_, PyAny>,
    draft: Option<u8>,
    formats: Option<&Bound<'_, PyDict>>,
    validate_formats: Option<bool>,
    ignore_unknown_formats: Option<bool>,
    retriever: Option<&Bound<'_, PyAny>>,
    mask: Option<String>,
) -> PyResult<ValidationErrorIter> {
    let options = make_options(
        draft,
        formats,
        validate_formats,
        ignore_unknown_formats,
        retriever,
    )?;
    let schema = ser::to_value(schema)?;
    match options.build(&schema) {
        Ok(validator) => iter_on_error(py, &validator, instance, mask.as_deref()),
        Err(error) => Err(into_py_err(py, error, mask.as_deref())?),
    }
}

fn handle_format_checked_panic(err: Box<dyn Any + Send>) -> PyErr {
    LAST_FORMAT_ERROR.with(|last| {
        if let Some(err) = last.borrow_mut().take() {
            let _ = panic::take_hook();
            err
        } else {
            exceptions::PyRuntimeError::new_err(format!("Validation panicked: {:?}", err))
        }
    })
}

#[pyclass(module = "jsonschema_rs", subclass)]
struct Validator {
    validator: jsonschema::Validator,
    mask: Option<String>,
}

/// validator_for(schema, formats=None, validate_formats=None, ignore_unknown_formats=True, retriever=None, mask=None)
///
/// Create a validator for the input schema with automatic draft detection and default options.
///
///     >>> validator = validator_for({"minimum": 5})
///     >>> validator.is_valid(3)
///     False
///
#[pyfunction]
#[pyo3(signature = (schema, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, mask=None))]
fn validator_for(
    py: Python<'_>,
    schema: &Bound<'_, PyAny>,
    formats: Option<&Bound<'_, PyDict>>,
    validate_formats: Option<bool>,
    ignore_unknown_formats: Option<bool>,
    retriever: Option<&Bound<'_, PyAny>>,
    mask: Option<String>,
) -> PyResult<Validator> {
    validator_for_impl(
        py,
        schema,
        None,
        formats,
        validate_formats,
        ignore_unknown_formats,
        retriever,
        mask,
    )
}

#[allow(clippy::too_many_arguments)]
fn validator_for_impl(
    py: Python<'_>,
    schema: &Bound<'_, PyAny>,
    draft: Option<u8>,
    formats: Option<&Bound<'_, PyDict>>,
    validate_formats: Option<bool>,
    ignore_unknown_formats: Option<bool>,
    retriever: Option<&Bound<'_, PyAny>>,
    mask: Option<String>,
) -> PyResult<Validator> {
    let obj_ptr = schema.as_ptr();
    let object_type = unsafe { pyo3::ffi::Py_TYPE(obj_ptr) };
    let schema = if unsafe { object_type == types::STR_TYPE } {
        let mut str_size: pyo3::ffi::Py_ssize_t = 0;
        let ptr = unsafe { PyUnicode_AsUTF8AndSize(obj_ptr, &mut str_size) };
        let slice = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), str_size as usize) };
        serde_json::from_slice(slice)
            .map_err(|error| PyValueError::new_err(format!("Invalid string: {}", error)))?
    } else {
        ser::to_value(schema)?
    };
    let options = make_options(
        draft,
        formats,
        validate_formats,
        ignore_unknown_formats,
        retriever,
    )?;
    match options.build(&schema) {
        Ok(validator) => Ok(Validator { validator, mask }),
        Err(error) => Err(into_py_err(py, error, mask.as_deref())?),
    }
}

#[pymethods]
impl Validator {
    #[new]
    #[pyo3(signature = (schema, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, mask=None))]
    fn new(
        py: Python<'_>,
        schema: &Bound<'_, PyAny>,
        formats: Option<&Bound<'_, PyDict>>,
        validate_formats: Option<bool>,
        ignore_unknown_formats: Option<bool>,
        retriever: Option<&Bound<'_, PyAny>>,
        mask: Option<String>,
    ) -> PyResult<Self> {
        validator_for(
            py,
            schema,
            formats,
            validate_formats,
            ignore_unknown_formats,
            retriever,
            mask,
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

/// Draft4Validator(schema, formats=None, validate_formats=None, ignore_unknown_formats=True, retriever=None, mask=None)
///
/// A JSON Schema Draft 4 validator.
///
///     >>> validator = Draft4Validator({"minimum": 5})
///     >>> validator.is_valid(3)
///     False
///
#[pyclass(module = "jsonschema_rs", extends=Validator, subclass)]
struct Draft4Validator {}

#[pymethods]
impl Draft4Validator {
    #[new]
    #[pyo3(signature = (schema, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, mask=None))]
    fn new(
        py: Python<'_>,
        schema: &Bound<'_, PyAny>,
        formats: Option<&Bound<'_, PyDict>>,
        validate_formats: Option<bool>,
        ignore_unknown_formats: Option<bool>,
        retriever: Option<&Bound<'_, PyAny>>,
        mask: Option<String>,
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
                mask,
            )?,
        ))
    }
}

/// Draft6Validator(schema, formats=None, validate_formats=None, ignore_unknown_formats=True, retriever=None, mask=None)
///
/// A JSON Schema Draft 6 validator.
///
///     >>> validator = Draft6Validator({"minimum": 5})
///     >>> validator.is_valid(3)
///     False
///
#[pyclass(module = "jsonschema_rs", extends=Validator, subclass)]
struct Draft6Validator {}

#[pymethods]
impl Draft6Validator {
    #[new]
    #[pyo3(signature = (schema, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, mask=None))]
    fn new(
        py: Python<'_>,
        schema: &Bound<'_, PyAny>,
        formats: Option<&Bound<'_, PyDict>>,
        validate_formats: Option<bool>,
        ignore_unknown_formats: Option<bool>,
        retriever: Option<&Bound<'_, PyAny>>,
        mask: Option<String>,
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
                mask,
            )?,
        ))
    }
}

/// Draft7Validator(schema, formats=None, validate_formats=None, ignore_unknown_formats=True, retriever=None, mask=None)
///
/// A JSON Schema Draft 7 validator.
///
///     >>> validator = Draft7Validator({"minimum": 5})
///     >>> validator.is_valid(3)
///     False
///
#[pyclass(module = "jsonschema_rs", extends=Validator, subclass)]
struct Draft7Validator {}

#[pymethods]
impl Draft7Validator {
    #[new]
    #[pyo3(signature = (schema, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, mask=None))]
    fn new(
        py: Python<'_>,
        schema: &Bound<'_, PyAny>,
        formats: Option<&Bound<'_, PyDict>>,
        validate_formats: Option<bool>,
        ignore_unknown_formats: Option<bool>,
        retriever: Option<&Bound<'_, PyAny>>,
        mask: Option<String>,
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
                mask,
            )?,
        ))
    }
}

/// Draft201909Validator(schema, formats=None, validate_formats=None, ignore_unknown_formats=True, retriever=None)
///
/// A JSON Schema Draft 2019-09 validator.
///
///     >>> validator = Draft201909Validator({"minimum": 5})
///     >>> validator.is_valid(3)
///     False
///
#[pyclass(module = "jsonschema_rs", extends=Validator, subclass)]
struct Draft201909Validator {}

#[pymethods]
impl Draft201909Validator {
    #[new]
    #[pyo3(signature = (schema, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, mask=None))]
    fn new(
        py: Python<'_>,
        schema: &Bound<'_, PyAny>,
        formats: Option<&Bound<'_, PyDict>>,
        validate_formats: Option<bool>,
        ignore_unknown_formats: Option<bool>,
        retriever: Option<&Bound<'_, PyAny>>,
        mask: Option<String>,
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
                mask,
            )?,
        ))
    }
}

/// Draft202012Validator(schema, formats=None, validate_formats=None, ignore_unknown_formats=True, retriever=None, mask=None)
///
/// A JSON Schema Draft 2020-12 validator.
///
///     >>> validator = Draft202012Validator({"minimum": 5})
///     >>> validator.is_valid(3)
///     False
///
#[pyclass(module = "jsonschema_rs", extends=Validator, subclass)]
struct Draft202012Validator {}

#[pymethods]
impl Draft202012Validator {
    #[new]
    #[pyo3(signature = (schema, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, mask=None))]
    fn new(
        py: Python<'_>,
        schema: &Bound<'_, PyAny>,
        formats: Option<&Bound<'_, PyDict>>,
        validate_formats: Option<bool>,
        ignore_unknown_formats: Option<bool>,
        retriever: Option<&Bound<'_, PyAny>>,
        mask: Option<String>,
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
                mask,
            )?,
        ))
    }
}

#[allow(dead_code)]
mod build {
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}

/// JSON Schema validation for Python written in Rust.
#[pymodule]
fn jsonschema_rs(py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    // To provide proper signatures for PyCharm, all the functions have their signatures as the
    // first line in docstrings. The idea is taken from NumPy.
    types::init();
    module.add_wrapped(wrap_pyfunction!(is_valid))?;
    module.add_wrapped(wrap_pyfunction!(validate))?;
    module.add_wrapped(wrap_pyfunction!(iter_errors))?;
    module.add_wrapped(wrap_pyfunction!(validator_for))?;
    module.add_class::<Draft4Validator>()?;
    module.add_class::<Draft6Validator>()?;
    module.add_class::<Draft7Validator>()?;
    module.add_class::<Draft201909Validator>()?;
    module.add_class::<Draft202012Validator>()?;
    module.add("ValidationError", py.get_type::<ValidationError>())?;
    module.add("ValidationErrorKind", py.get_type::<ValidationErrorKind>())?;
    module.add("Draft4", DRAFT4)?;
    module.add("Draft6", DRAFT6)?;
    module.add("Draft7", DRAFT7)?;
    module.add("Draft201909", DRAFT201909)?;
    module.add("Draft202012", DRAFT202012)?;

    // Add build metadata to ease triaging incoming issues
    #[allow(deprecated)]
    module.add("__build__", pyo3_built::pyo3_built!(py, build))?;

    Ok(())
}
