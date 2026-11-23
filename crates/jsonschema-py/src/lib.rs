#![allow(clippy::too_many_arguments)]
#![allow(unsafe_code)]
#![allow(unreachable_pub)]
#![allow(rustdoc::bare_urls)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_wrap)]
use std::{
    any::Any,
    borrow::Cow,
    cell::RefCell,
    collections::VecDeque,
    fmt::Write as _,
    io::Write,
    panic::{self, AssertUnwindSafe},
    sync::Arc,
};

use email::EmailOptions;
#[cfg(not(target_arch = "wasm32"))]
use http::HttpOptions;
use jsonschema::{
    canonical::json::canonical_number,
    json::{
        inspect, Array, Json, JsonNumber, JsonText, JsonTextDocument, JsonTextNode, Node, Object,
        Pyo3,
    },
    paths::{Location, LocationSegment},
    Draft, Retrieve, ValidationOptions,
};
use pyo3::{
    exceptions::{self, PyKeyError, PyValueError},
    ffi::{PyImport_GetModule, PyList_New, PyList_SetItem, PyUnicode_AsUTF8AndSize, Py_DECREF},
    intern,
    prelude::*,
    types::{PyAny, PyBool, PyBytes, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple, PyType},
    wrap_pyfunction,
};
use referencing::{parse_index, unescape_segment};
use regex::{FancyRegexOptions, RegexOptions};
#[cfg(not(target_arch = "wasm32"))]
use retriever::DetachedRetriever;
use retriever::{into_retriever, Retriever};
use ser::to_value;
#[macro_use]
extern crate pyo3_built;

mod canonical;
mod clone;
mod email;
#[cfg(not(target_arch = "wasm32"))]
mod http;
mod output;
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

// Reading `sys.modules` skips the import machinery `py.import` runs on every call. The module is
// only absent if something removed it from `sys.modules`, and importing it again restores it.
fn module(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    let name = intern!(py, "jsonschema_rs");
    let cached = unsafe { PyImport_GetModule(name.as_ptr()) };
    match unsafe { Bound::from_owned_ptr_or_opt(py, cached) } {
        Some(module) => Ok(module),
        None => Ok(py.import(name)?.into_any()),
    }
}

fn validation_error_type(py: Python<'_>) -> PyResult<Bound<'_, PyType>> {
    let exception_class = module(py)?.getattr(intern!(py, "ValidationError"))?;
    Ok(exception_class.cast_into::<PyType>()?)
}

fn referencing_error_type(py: Python<'_>) -> PyResult<Bound<'_, PyType>> {
    let exception_class = module(py)?.getattr(intern!(py, "ReferencingError"))?;
    Ok(exception_class.cast_into::<PyType>()?)
}

/// Whether the float `value` is the same number as the JSON literal `text`.
fn float_spells(value: f64, text: &str) -> bool {
    // Two written forms of one value share a canonical form, so this compares values, not text: `1e20`
    // and `100000000000000000000` agree, while `100000000000000000000.1` differs from both.
    let same_value = |written: &str| {
        canonical_number(written).as_deref().unwrap_or(written)
            == canonical_number(text).as_deref().unwrap_or(text)
    };
    let mut buffer = zmij::Buffer::new();
    // A whole float past 2^53 may be written with its exact digits, which its shortest text can miss.
    same_value(buffer.format_finite(value))
        || (value.fract() == 0.0 && same_value(&format!("{value:.1}")))
}

/// Convert a JSON number to a Python object. Integers stay exact; a literal no float holds becomes
/// `Decimal`.
pub(crate) fn number_to_python(py: Python<'_>, n: &serde_json::Number) -> PyResult<Py<PyAny>> {
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
            if let Some(f) = n.as_f64().filter(|f| float_spells(*f, s)) {
                Ok(pyo3::types::PyFloat::new(py, f).into_any().unbind())
            } else {
                // Fall back to decimal.Decimal for values no float carries, so Python callers
                // observe the exact literal from JSON instead of ValueError/inf when numbers
                // exceed binary64 limits, or a rounded neighbour when they exceed its
                // precision. A canonical bound can sit a fraction away from a round number,
                // and rounding it there would accept values the schema rejects.
                let decimal_type =
                    unsafe { PyType::from_borrowed_type_ptr(py, types::DECIMAL_TYPE) };
                decimal_type.call1((s,)).map(|obj| obj.into_any().unbind())
            }
        } else {
            // Large integer - parse from string representation
            let int_type = py.get_type::<pyo3::types::PyInt>();
            int_type.call1((s,)).map(|obj| obj.into_any().unbind())
        }
    }
}

/// Convert a serde_json::Value to a Python object. Integers stay exact; a literal no float holds
/// becomes `Decimal`.
pub(crate) fn value_to_python(py: Python<'_>, value: &serde_json::Value) -> PyResult<Py<PyAny>> {
    match value {
        serde_json::Value::Null => Ok(py.None()),
        serde_json::Value::Bool(b) => Ok(pyo3::types::PyBool::new(py, *b)
            .to_owned()
            .into_any()
            .unbind()),
        serde_json::Value::Number(n) => number_to_python(py, n),
        serde_json::Value::String(s) => Ok(PyString::new(py, s).into_any().unbind()),
        serde_json::Value::Array(arr) => {
            let len = arr.len() as pyo3::ffi::Py_ssize_t;
            unsafe {
                let list = PyList_New(len);
                if list.is_null() {
                    return Err(PyErr::fetch(py));
                }
                for (i, item) in arr.iter().enumerate() {
                    match value_to_python(py, item) {
                        Ok(py_item) => {
                            // PyList_SetItem steals the reference
                            PyList_SetItem(list, i as pyo3::ffi::Py_ssize_t, py_item.into_ptr());
                        }
                        Err(e) => {
                            // Clean up the partially filled list
                            Py_DECREF(list);
                            return Err(e);
                        }
                    }
                }
                Ok(Bound::from_owned_ptr(py, list).unbind())
            }
        }
        serde_json::Value::Object(obj) => map_to_python(py, obj),
    }
}

/// Convert a serde_json Map directly to a Python dict (avoids wrapping in Value::Object)
fn map_to_python(
    py: Python<'_>,
    obj: &serde_json::Map<String, serde_json::Value>,
) -> PyResult<Py<PyAny>> {
    let py_dict = PyDict::new(py);
    for (k, v) in obj {
        py_dict.set_item(k, value_to_python(py, v)?)?;
    }
    Ok(py_dict.into_any().unbind())
}

fn annotation_entry_to_py(
    py: Python<'_>,
    entry: jsonschema::AnnotationEntry<'_>,
) -> PyResult<Py<PyAny>> {
    let dict = PyDict::new(py);
    dict.set_item("schemaLocation", entry.schema_location)?;
    if let Some(uri) = entry.absolute_keyword_location {
        dict.set_item("absoluteKeywordLocation", uri.as_str())?;
    } else {
        dict.set_item("absoluteKeywordLocation", py.None())?;
    }
    dict.set_item("instanceLocation", entry.instance_location.as_str())?;
    dict.set_item(
        "annotations",
        value_to_python(py, entry.annotations.value())?,
    )?;
    Ok(dict.into())
}

fn error_entry_to_py(py: Python<'_>, entry: jsonschema::ErrorEntry<'_>) -> PyResult<Py<PyAny>> {
    let dict = PyDict::new(py);
    dict.set_item("schemaLocation", entry.schema_location)?;
    if let Some(uri) = entry.absolute_keyword_location {
        dict.set_item("absoluteKeywordLocation", uri.as_str())?;
    } else {
        dict.set_item("absoluteKeywordLocation", py.None())?;
    }
    dict.set_item("instanceLocation", entry.instance_location.as_str())?;
    dict.set_item("error", entry.error.to_string())?;
    Ok(dict.into())
}

#[pyclass(module = "jsonschema_rs", name = "Evaluation")]
struct PyEvaluation {
    inner: jsonschema::Evaluation,
}

impl PyEvaluation {
    fn new(evaluation: jsonschema::Evaluation) -> Self {
        PyEvaluation { inner: evaluation }
    }
}

#[pymethods]
impl PyEvaluation {
    /// Whether the evaluated instance is valid.
    #[getter]
    fn valid(&self) -> bool {
        self.inner.is_valid()
    }

    /// Return the flag output as a dict.
    fn flag(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        Ok(output::flag(py, self.inner.is_valid())?.into_any().unbind())
    }

    /// Return the list output as a dict.
    fn list(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        Ok(output::list(py, self.inner.root())?.into_any().unbind())
    }

    /// Return the hierarchical output as a dict.
    fn hierarchical(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        Ok(output::hierarchical(py, self.inner.root())?
            .into_any()
            .unbind())
    }

    /// Return collected annotations for all evaluated nodes.
    fn annotations(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let entries = PyList::empty(py);
        for entry in self.inner.iter_annotations() {
            entries.append(annotation_entry_to_py(py, entry)?)?;
        }
        Ok(entries.into())
    }

    /// Return collected errors for all evaluated nodes.
    fn errors(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let entries = PyList::empty(py);
        for entry in self.inner.iter_errors() {
            entries.append(error_entry_to_py(py, entry)?)?;
        }
        Ok(entries.into())
    }

    fn __repr__(&self) -> String {
        format!("<Evaluation valid={}>", self.inner.is_valid())
    }
}

struct ValidationErrorArgs {
    /// The message is the prefix of `verbose_message` up to this byte.
    message_len: usize,
    verbose_message: String,
    schema_path: Py<PyList>,
    instance_path: Py<PyList>,
    evaluation_path: Py<PyList>,
    kind: ValidationErrorKind,
    instance: Py<PyAny>,
    absolute_keyword_location: Option<String>,
}

fn create_validation_error_object(
    py: Python<'_>,
    args: ValidationErrorArgs,
) -> PyResult<Py<PyAny>> {
    let ty = validation_error_type(py)?;
    let kind_obj = args.kind.into_pyobject(py)?.unbind();
    let obj = ty.call1((
        &args.verbose_message[..args.message_len],
        &args.verbose_message,
        args.schema_path,
        args.instance_path,
        args.evaluation_path,
        kind_obj,
        args.instance,
        args.absolute_keyword_location,
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

pub(crate) fn referencing_error_pyerr(py: Python<'_>, message: String) -> PyResult<PyErr> {
    let obj = create_referencing_error_object(py, message)?;
    Ok(PyErr::from_value(obj.into_bound(py)))
}

/// Type of validation failure with its contextual data.
#[pyclass(module = "jsonschema_rs")]
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
    Custom { keyword: String, message: String },
    Enum { options: Py<PyAny> },
    ExclusiveMaximum { limit: Py<PyAny> },
    ExclusiveMinimum { limit: Py<PyAny> },
    FalseSchema {},
    Format { format: String },
    FromUtf8 { error: String },
    MaxContains { limit: u64 },
    MaxItems { limit: u64 },
    Maximum { limit: Py<PyAny> },
    MaxLength { limit: u64 },
    MaxProperties { limit: u64 },
    MinContains { limit: u64 },
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
        root: Option<InstanceRoot<'_, '_>>,
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
                    context: convert_validation_context(py, context, mask, root)?,
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
            jsonschema::error::ValidationErrorKind::Custom { keyword, message } => {
                ValidationErrorKind::Custom { keyword, message }
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
            jsonschema::error::ValidationErrorKind::MinContains { limit } => {
                ValidationErrorKind::MinContains { limit }
            }
            jsonschema::error::ValidationErrorKind::MaxContains { limit } => {
                ValidationErrorKind::MaxContains { limit }
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
                    context: convert_validation_context(py, context, mask, root)?,
                }
            }
            jsonschema::error::ValidationErrorKind::OneOfNotValid { context } => {
                ValidationErrorKind::OneOfNotValid {
                    context: convert_validation_context(py, context, mask, root)?,
                }
            }
            jsonschema::error::ValidationErrorKind::Pattern { pattern } => {
                ValidationErrorKind::Pattern { pattern }
            }
            jsonschema::error::ValidationErrorKind::PropertyNames { error } => {
                ValidationErrorKind::PropertyNames {
                    error: {
                        create_validation_error_object(
                            py,
                            // The nested error reports the property name at the object's location.
                            into_validation_error_args(py, *error, mask, None)?,
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
                            PyList::new(py, [ty.as_str()])?.unbind()
                        }
                        jsonschema::error::TypeKind::Multiple(types) => {
                            PyList::new(py, types.iter().map(jsonschema::JsonType::as_str))?
                                .unbind()
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

#[pymethods]
impl ValidationErrorKind {
    /// The keyword that failed, or ``"falseSchema"`` for a ``false`` schema.
    #[getter]
    fn name(&self) -> &str {
        match self {
            Self::AdditionalItems { .. } => "additionalItems",
            Self::AdditionalProperties { .. } => "additionalProperties",
            Self::AnyOf { .. } => "anyOf",
            Self::BacktrackLimitExceeded { .. } | Self::Pattern { .. } => "pattern",
            Self::Constant { .. } => "const",
            Self::Contains { .. } => "contains",
            Self::ContentEncoding { .. } | Self::FromUtf8 { .. } => "contentEncoding",
            Self::ContentMediaType { .. } => "contentMediaType",
            Self::Custom { keyword, .. } => keyword,
            Self::Enum { .. } => "enum",
            Self::ExclusiveMaximum { .. } => "exclusiveMaximum",
            Self::ExclusiveMinimum { .. } => "exclusiveMinimum",
            Self::FalseSchema { .. } => "falseSchema",
            Self::Format { .. } => "format",
            Self::MaxContains { .. } => "maxContains",
            Self::MaxItems { .. } => "maxItems",
            Self::Maximum { .. } => "maximum",
            Self::MaxLength { .. } => "maxLength",
            Self::MaxProperties { .. } => "maxProperties",
            Self::MinContains { .. } => "minContains",
            Self::MinItems { .. } => "minItems",
            Self::Minimum { .. } => "minimum",
            Self::MinLength { .. } => "minLength",
            Self::MinProperties { .. } => "minProperties",
            Self::MultipleOf { .. } => "multipleOf",
            Self::Not { .. } => "not",
            Self::OneOfMultipleValid { .. } | Self::OneOfNotValid { .. } => "oneOf",
            Self::PropertyNames { .. } => "propertyNames",
            Self::Required { .. } => "required",
            Self::Type { .. } => "type",
            Self::UnevaluatedItems { .. } => "unevaluatedItems",
            Self::UnevaluatedProperties { .. } => "unevaluatedProperties",
            Self::UniqueItems { .. } => "uniqueItems",
            Self::Referencing { .. } => "$ref",
        }
    }

    #[getter]
    fn value(&self, py: Python<'_>) -> Py<PyAny> {
        match self {
            Self::AdditionalItems { limit } => limit.into_pyobject(py).unwrap().into_any().unbind(),
            Self::AdditionalProperties { unexpected }
            | Self::UnevaluatedItems { unexpected }
            | Self::UnevaluatedProperties { unexpected } => unexpected.clone_ref(py).into_any(),
            Self::AnyOf { context }
            | Self::OneOfMultipleValid { context }
            | Self::OneOfNotValid { context } => context.clone_ref(py).into_any(),
            Self::BacktrackLimitExceeded { error } | Self::FromUtf8 { error } => {
                error.into_pyobject(py).unwrap().into_any().unbind()
            }
            Self::Constant { expected_value } => expected_value.clone_ref(py),
            Self::Contains {} | Self::FalseSchema {} | Self::UniqueItems {} => py.None(),
            Self::ContentEncoding { content_encoding } => content_encoding
                .into_pyobject(py)
                .unwrap()
                .into_any()
                .unbind(),
            Self::ContentMediaType { content_media_type } => content_media_type
                .into_pyobject(py)
                .unwrap()
                .into_any()
                .unbind(),
            Self::Custom { message, .. } => message.into_pyobject(py).unwrap().into_any().unbind(),
            Self::Enum { options } => options.clone_ref(py),
            Self::ExclusiveMaximum { limit }
            | Self::ExclusiveMinimum { limit }
            | Self::Maximum { limit }
            | Self::Minimum { limit } => limit.clone_ref(py),
            Self::Format { format } => format.into_pyobject(py).unwrap().into_any().unbind(),
            Self::MaxContains { limit }
            | Self::MaxItems { limit }
            | Self::MaxLength { limit }
            | Self::MaxProperties { limit }
            | Self::MinContains { limit }
            | Self::MinItems { limit }
            | Self::MinLength { limit }
            | Self::MinProperties { limit } => limit.into_pyobject(py).unwrap().into_any().unbind(),
            Self::MultipleOf { multiple_of } => multiple_of.clone_ref(py),
            Self::Not { schema } => schema.clone_ref(py),
            Self::Pattern { pattern } => pattern.into_pyobject(py).unwrap().into_any().unbind(),
            Self::PropertyNames { error } | Self::Referencing { error } => error.clone_ref(py),
            Self::Required { property } => property.clone_ref(py),
            Self::Type { types } => types.clone_ref(py).into_any(),
        }
    }

    fn as_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let dict = PyDict::new(py);
        match self {
            Self::AdditionalItems { limit } => dict.set_item("limit", limit)?,
            Self::AdditionalProperties { unexpected }
            | Self::UnevaluatedItems { unexpected }
            | Self::UnevaluatedProperties { unexpected } => {
                dict.set_item("unexpected", unexpected)?;
            }
            Self::AnyOf { context }
            | Self::OneOfMultipleValid { context }
            | Self::OneOfNotValid { context } => dict.set_item("context", context)?,
            Self::BacktrackLimitExceeded { error } | Self::FromUtf8 { error } => {
                dict.set_item("error", error)?;
            }
            Self::Constant { expected_value } => dict.set_item("expected_value", expected_value)?,
            Self::Contains {} | Self::FalseSchema {} | Self::UniqueItems {} => {}
            Self::ContentEncoding { content_encoding } => {
                dict.set_item("content_encoding", content_encoding)?;
            }
            Self::ContentMediaType { content_media_type } => {
                dict.set_item("content_media_type", content_media_type)?;
            }
            Self::Custom { keyword, message } => {
                dict.set_item("keyword", keyword)?;
                dict.set_item("message", message)?;
            }
            Self::Enum { options } => dict.set_item("options", options)?,
            Self::ExclusiveMaximum { limit }
            | Self::ExclusiveMinimum { limit }
            | Self::Maximum { limit }
            | Self::Minimum { limit } => dict.set_item("limit", limit)?,
            Self::Format { format } => dict.set_item("format", format)?,
            Self::MaxContains { limit }
            | Self::MaxItems { limit }
            | Self::MaxLength { limit }
            | Self::MaxProperties { limit }
            | Self::MinContains { limit }
            | Self::MinItems { limit }
            | Self::MinLength { limit }
            | Self::MinProperties { limit } => dict.set_item("limit", limit)?,
            Self::MultipleOf { multiple_of } => dict.set_item("multiple_of", multiple_of)?,
            Self::Not { schema } => dict.set_item("schema", schema)?,
            Self::Pattern { pattern } => dict.set_item("pattern", pattern)?,
            Self::PropertyNames { error } | Self::Referencing { error } => {
                dict.set_item("error", error)?;
            }
            Self::Required { property } => dict.set_item("property", property)?,
            Self::Type { types } => dict.set_item("types", types)?,
        }
        Ok(dict.into())
    }
}

fn convert_validation_context(
    py: Python<'_>,
    context: Vec<Vec<jsonschema::error::ValidationError<'static>>>,
    mask: Option<&str>,
    root: Option<InstanceRoot<'_, '_>>,
) -> PyResult<Py<PyList>> {
    unsafe {
        let outer_len = context.len() as pyo3::ffi::Py_ssize_t;
        let outer_list = PyList_New(outer_len);
        if outer_list.is_null() {
            return Err(PyErr::fetch(py));
        }

        for (outer_idx, errors) in context.into_iter().enumerate() {
            let inner_len = errors.len() as pyo3::ffi::Py_ssize_t;
            let inner_list = PyList_New(inner_len);
            if inner_list.is_null() {
                Py_DECREF(outer_list);
                return Err(PyErr::fetch(py));
            }

            for (inner_idx, error) in errors.into_iter().enumerate() {
                let args = match into_validation_error_args(py, error, mask, root) {
                    Ok(args) => args,
                    Err(e) => {
                        Py_DECREF(inner_list);
                        Py_DECREF(outer_list);
                        return Err(e);
                    }
                };

                let error_obj = match create_validation_error_object(py, args) {
                    Ok(obj) => obj,
                    Err(e) => {
                        Py_DECREF(inner_list);
                        Py_DECREF(outer_list);
                        return Err(e);
                    }
                };

                PyList_SetItem(
                    inner_list,
                    inner_idx as pyo3::ffi::Py_ssize_t,
                    error_obj.into_ptr(),
                );
            }

            PyList_SetItem(outer_list, outer_idx as pyo3::ffi::Py_ssize_t, inner_list);
        }

        // SAFETY: outer_list is a PyList created by PyList_New
        Ok(Bound::from_owned_ptr(py, outer_list)
            .cast_into_unchecked::<PyList>()
            .unbind())
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

fn location_to_list(py: Python<'_>, location: &Location) -> PyResult<Py<PyList>> {
    let list = unsafe { PyList_New(location.segments().count() as pyo3::ffi::Py_ssize_t) };
    for (index, segment) in location.segments().enumerate() {
        let object = match segment {
            LocationSegment::Property(property) => PyString::new(py, &property).into_any(),
            LocationSegment::Index(value) => value.into_pyobject(py)?.into_any(),
        };
        // SAFETY: `list` has room for every segment and each slot is written once
        unsafe {
            PyList_SetItem(list, index as pyo3::ffi::Py_ssize_t, object.into_ptr());
        }
    }
    // SAFETY: `list` is a `PyList` created by `PyList_New`
    Ok(unsafe {
        Bound::from_owned_ptr(py, list)
            .cast_into_unchecked::<PyList>()
            .unbind()
    })
}

/// Where an error reads its instance from: the Python object validated, or the JSON text.
#[derive(Clone, Copy)]
pub(crate) enum InstanceRoot<'a, 'py> {
    Object(&'a Bound<'py, PyAny>),
    Text(JsonTextNode<'a>),
}

impl<'py> InstanceRoot<'_, 'py> {
    // The instance at `location`, or `None` to convert the error's JSON copy instead. JSON text
    // builds only that value, as `json.loads` would, never the rest of the document.
    fn instance_at(
        &self,
        py: Python<'py>,
        location: &Location,
    ) -> PyResult<Option<Bound<'py, PyAny>>> {
        match self {
            InstanceRoot::Object(root) => Ok(plain_instance_at(root, location)),
            InstanceRoot::Text(root) => text_node_at(*root, location)
                .map(|node| text_node_to_python(py, &node))
                .transpose(),
        }
    }
}

fn text_node_at<'a>(root: JsonTextNode<'a>, location: &Location) -> Option<JsonTextNode<'a>> {
    let mut current = root;
    for segment in location.as_str().split('/').skip(1) {
        current = if let Some(object) = current.as_object() {
            object.get(&JsonText::prepare_key(&unescape_segment(segment)))?
        } else {
            current.as_array()?.elements().nth(parse_index(segment)?)?
        };
    }
    Some(current)
}

// The object at `location` under `root`, when it and everything in it are plain JSON types, which
// convert to and from JSON unchanged; `None` otherwise, and the caller converts the JSON value.
fn plain_instance_at<'py>(
    root: &Bound<'py, PyAny>,
    location: &Location,
) -> Option<Bound<'py, PyAny>> {
    let mut current = root.clone();
    // The container decides how to read a segment: `segments()` would read the key "200" as an index.
    for segment in location.as_str().split('/').skip(1) {
        current = if let Ok(dict) = current.cast_exact::<PyDict>() {
            dict.get_item(unescape_segment(segment).as_ref()).ok()??
        } else if let Ok(list) = current.cast_exact::<PyList>() {
            list.get_item(parse_index(segment)?).ok()?
        } else {
            current
                .cast_exact::<PyTuple>()
                .ok()?
                .get_item(parse_index(segment)?)
                .ok()?
        };
    }
    is_plain_json(&current, 0).then_some(current)
}

fn is_plain_json(value: &Bound<'_, PyAny>, depth: u8) -> bool {
    // Deeper values, cycles included, keep the conversion that reports the recursion limit.
    if depth == ser::RECURSION_LIMIT {
        return false;
    }
    if value.is_none()
        || value.is_exact_instance_of::<PyString>()
        || value.is_exact_instance_of::<PyBool>()
        || value.is_exact_instance_of::<PyInt>()
        || value.is_exact_instance_of::<PyFloat>()
    {
        return true;
    }
    if let Ok(list) = value.cast_exact::<PyList>() {
        return list.iter().all(|item| is_plain_json(&item, depth + 1));
    }
    if let Ok(dict) = value.cast_exact::<PyDict>() {
        return dict.iter().all(|(key, item)| {
            key.is_exact_instance_of::<PyString>() && is_plain_json(&item, depth + 1)
        });
    }
    false
}

fn into_validation_error_args(
    py: Python<'_>,
    error: jsonschema::ValidationError<'_>,
    mask: Option<&str>,
    root: Option<InstanceRoot<'_, '_>>,
) -> PyResult<ValidationErrorArgs> {
    // Sized for the message plus what `to_error_message` appends, so neither reallocates.
    let mut message = String::with_capacity(256);
    if let Some(mask) = mask {
        write!(message, "{}", error.masked_with(mask))
    } else {
        write!(message, "{error}")
    }
    .expect("Writing to a String cannot fail");
    let message_len = message.len();
    let verbose_message = to_error_message(&error, message, mask);
    let parts = error.into_parts();
    let schema_path = location_to_list(py, &parts.schema_path)?;
    let instance_path = location_to_list(py, &parts.instance_path)?;
    // Without a `$ref` on the way the two locations are the same, and copying the list beats
    // parsing the location and building its segments again.
    let evaluation_path = if parts.evaluation_path.as_str() == parts.schema_path.as_str() {
        let bound = schema_path.bind(py);
        bound.get_slice(0, bound.len()).unbind()
    } else {
        location_to_list(py, &parts.evaluation_path)?
    };
    let kind = ValidationErrorKind::try_new(py, parts.kind, mask, root)?;
    // Scalars convert faster than the path to them is followed.
    let original = match parts.instance.as_ref() {
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => match root {
            Some(root) => root.instance_at(py, &parts.instance_path)?,
            None => None,
        },
        _ => None,
    };
    let instance = match original {
        Some(instance) => instance.unbind(),
        None => value_to_python(py, parts.instance.as_ref())?,
    };
    let absolute_keyword_location = parts
        .absolute_keyword_location
        .map(|u| u.as_str().to_owned());
    Ok(ValidationErrorArgs {
        message_len,
        verbose_message,
        schema_path,
        instance_path,
        evaluation_path,
        kind,
        instance,
        absolute_keyword_location,
    })
}
pub(crate) fn into_py_err(
    py: Python<'_>,
    error: jsonschema::ValidationError<'_>,
    mask: Option<&str>,
    root: Option<InstanceRoot<'_, '_>>,
) -> PyResult<PyErr> {
    let args = into_validation_error_args(py, error, mask, root)?;
    let is_custom = matches!(args.kind, ValidationErrorKind::Custom { .. });
    let py_err = validation_error_pyerr(py, args)?;
    if is_custom {
        if let Some(cause) = CUSTOM_KEYWORD_CAUSE.with(|cell| cell.borrow_mut().pop_front()) {
            let err_obj = py_err.value(py);
            err_obj.setattr("__cause__", cause.value(py))?;
            err_obj.setattr("__suppress_context__", true)?;
        }
    }
    Ok(py_err)
}

pub(crate) fn get_draft(draft: u8) -> PyResult<Draft> {
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
    // Stores the original Python exceptions from a custom keyword's validate()/iter_errors() calls
    // so each can be attached as __cause__ on the resulting ValidationError, in FIFO order.
    static CUSTOM_KEYWORD_CAUSE: RefCell<VecDeque<PyErr>> = const { RefCell::new(VecDeque::new()) };
}

// Custom keyword validator - instantiated with (parent_schema, value, schema_path), must have validate(instance) method
struct CustomKeyword {
    instance: Py<PyAny>,
}

impl CustomKeyword {
    fn validate_object<'i>(
        &self,
        instance: &Bound<'_, PyAny>,
    ) -> Result<(), jsonschema::ValidationError<'i>> {
        let py = instance.py();
        match self.instance.call_method1(py, "validate", (instance,)) {
            Ok(_) => Ok(()),
            Err(error) => Err(custom_error_with_cause(py, error)),
        }
    }

    fn is_valid_object(&self, instance: &Bound<'_, PyAny>) -> bool {
        self.instance
            .call_method1(instance.py(), "validate", (instance,))
            .is_ok()
    }

    fn iter_errors_object<'i>(
        &self,
        instance: &Bound<'_, PyAny>,
    ) -> Box<dyn Iterator<Item = jsonschema::ValidationError<'i>> + 'i> {
        let py = instance.py();
        // Without an `iter_errors` method, fall back to the single-error `validate` path.
        if !self
            .instance
            .bind(py)
            .hasattr("iter_errors")
            .unwrap_or(false)
        {
            return boxed_error_iter(self.validate_object(instance).err());
        }
        let iterable = match self.instance.call_method1(py, "iter_errors", (instance,)) {
            Ok(iterable) => iterable,
            Err(error) => return boxed_error_iter(Some(custom_error_with_cause(py, error))),
        };
        let iterator = match iterable.bind(py).try_iter() {
            Ok(iterator) => iterator,
            Err(error) => return boxed_error_iter(Some(custom_error_with_cause(py, error))),
        };
        let mut errors = Vec::new();
        for item in iterator {
            let error = match item {
                Ok(item) => custom_error_with_cause(py, PyErr::from_value(item)),
                Err(error) => custom_error_with_cause(py, error),
            };
            errors.push(error);
        }
        Box::new(errors.into_iter())
    }
}

impl<'i> jsonschema::Keyword<'i, Pyo3> for CustomKeyword {
    fn validate(
        &self,
        instance: Borrowed<'i, 'i, PyAny>,
    ) -> Result<(), jsonschema::ValidationError<'i>> {
        self.validate_object(&instance.to_owned())
    }

    fn is_valid(&self, instance: Borrowed<'i, 'i, PyAny>) -> bool {
        self.is_valid_object(&instance.to_owned())
    }

    fn iter_errors(
        &self,
        instance: Borrowed<'i, 'i, PyAny>,
    ) -> Box<dyn Iterator<Item = jsonschema::ValidationError<'i>> + 'i> {
        self.iter_errors_object(&instance.to_owned())
    }
}

// The keyword class reads Python objects, so a node of JSON text becomes one first.
impl<'i> jsonschema::Keyword<'i, JsonText> for CustomKeyword {
    fn validate(&self, instance: JsonTextNode<'i>) -> Result<(), jsonschema::ValidationError<'i>> {
        Python::attach(|py| self.validate_object(&keyword_instance(py, &instance)))
    }

    fn is_valid(&self, instance: JsonTextNode<'i>) -> bool {
        Python::attach(|py| self.is_valid_object(&keyword_instance(py, &instance)))
    }

    fn iter_errors(
        &self,
        instance: JsonTextNode<'i>,
    ) -> Box<dyn Iterator<Item = jsonschema::ValidationError<'i>> + 'i> {
        Python::attach(|py| self.iter_errors_object(&keyword_instance(py, &instance)))
    }
}

// What a keyword reads. An instance Python cannot hold, such as an integer past
// `sys.get_int_max_str_digits()`, raises as `json.loads` does: the panic carries the error to
// `handle_format_checked_panic`, which re-raises it.
fn keyword_instance<'py>(py: Python<'py>, node: &JsonTextNode<'_>) -> Bound<'py, PyAny> {
    text_node_to_python(py, node).unwrap_or_else(|error| {
        LAST_FORMAT_ERROR.with(|last| *last.borrow_mut() = Some(error));
        std::panic::set_hook(Box::new(|_| {}));
        panic!("Building the instance failed")
    })
}

// What `json.loads` builds: members in document order, `int` for integers, `float` otherwise.
fn text_node_to_python<'py>(
    py: Python<'py>,
    node: &JsonTextNode<'_>,
) -> PyResult<Bound<'py, PyAny>> {
    if let Some(object) = node.as_object() {
        let dict = PyDict::new(py);
        for (name, value) in object.members() {
            dict.set_item(name.as_ref(), text_node_to_python(py, &value)?)?;
        }
        return Ok(dict.into_any());
    }
    if let Some(array) = node.as_array() {
        let list = PyList::empty(py);
        for element in array.elements() {
            list.append(text_node_to_python(py, &element)?)?;
        }
        return Ok(list.into_any());
    }
    if let Some(string) = node.as_string() {
        return Ok(PyString::new(py, &string).into_any());
    }
    if let Some(number) = node.as_number() {
        if let Some(value) = number.as_i64() {
            return Ok(value.into_pyobject(py)?.into_any());
        }
        let digits = JsonNumber::as_str(&number);
        if digits.contains(['.', 'e', 'E']) {
            // Past `f64` this is infinite, as with `json.loads`.
            let value: f64 = digits.parse().expect("a checked number");
            return Ok(PyFloat::new(py, value).into_any());
        }
        return py.get_type::<PyInt>().call1((digits.as_ref(),));
    }
    Ok(match node.as_boolean() {
        Some(value) => PyBool::new(py, value).to_owned().into_any(),
        None => py.None().into_bound(py),
    })
}

// Build a custom `ValidationError` from a Python exception, queuing the exception as the error's
// cause so it surfaces as `__cause__`, exactly like the single `validate` path.
fn custom_error_with_cause<'i>(py: Python<'_>, error: PyErr) -> jsonschema::ValidationError<'i> {
    let message = error.value(py).to_string();
    CUSTOM_KEYWORD_CAUSE.with(|cell| cell.borrow_mut().push_back(error));
    jsonschema::ValidationError::custom(message)
}

fn boxed_error_iter<'i>(
    error: Option<jsonschema::ValidationError<'i>>,
) -> Box<dyn Iterator<Item = jsonschema::ValidationError<'i>> + 'i> {
    Box::new(error.into_iter())
}

// RAII scope giving each surfacing call its own keyword-cause queue and restoring the caller's on
// exit. This discards causes a call leaves unconsumed (`evaluate`, which renders messages rather
// than exceptions, or a panic — the drop still runs) and lets a keyword re-enter validation without
// clobbering the outer call's pending causes.
struct KeywordCauseScope {
    saved: VecDeque<PyErr>,
}

impl KeywordCauseScope {
    fn enter() -> Self {
        Self {
            saved: CUSTOM_KEYWORD_CAUSE.with(|cell| std::mem::take(&mut *cell.borrow_mut())),
        }
    }
}

impl Drop for KeywordCauseScope {
    fn drop(&mut self) {
        let saved = std::mem::take(&mut self.saved);
        CUSTOM_KEYWORD_CAUSE.with(|cell| *cell.borrow_mut() = saved);
    }
}

/// A `format` check that reads numbers, made by `number_format`.
#[pyclass(module = "jsonschema_rs", frozen)]
struct NumberFormat {
    check: Py<PyAny>,
}

#[pymethods]
impl NumberFormat {
    fn __call__(&self, py: Python<'_>, number: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        self.check.call1(py, (number,))
    }
}

/// number_format(check)
///
/// A `format` check that reads numbers, for the `formats` argument. `check` receives an `int`,
/// `float` or `Decimal`; instances of other types pass the format.
#[pyfunction]
fn number_format(check: &Bound<'_, PyAny>) -> PyResult<NumberFormat> {
    if !check.is_callable() {
        return Err(exceptions::PyTypeError::new_err(
            "number_format expects a callable",
        ));
    }
    Ok(NumberFormat {
        check: check.clone().unbind(),
    })
}

/// known_formats(draft, formats=None)
///
/// The names of the formats with a check under `draft`, sorted and without duplicates: the
/// built-in formats and those in `formats`.
#[pyfunction]
#[pyo3(signature = (draft, formats=None))]
fn known_formats(draft: u8, formats: Option<&Bound<'_, PyDict>>) -> PyResult<Vec<String>> {
    let draft = get_draft(draft)?;
    let options = make_options::<Pyo3>(
        None, formats, None, None, None, None, None, None, None, None, None, None, None,
    )?;
    Ok(options
        .known_formats(draft)
        .into_iter()
        .map(str::to_owned)
        .collect())
}

// A format check's result; a Python error is stored and re-raised once validation unwinds.
fn format_outcome(result: PyResult<bool>) -> bool {
    match result {
        Ok(valid) => valid,
        Err(error) => {
            LAST_FORMAT_ERROR.with(|last| {
                *last.borrow_mut() = Some(error);
            });
            std::panic::set_hook(Box::new(|_| {}));
            // Caught by `handle_format_checked_panic`, which re-raises the stored error
            panic!("Format checker failed")
        }
    }
}

fn make_options<'a, F: Json>(
    draft: Option<u8>,
    formats: Option<&Bound<'a, PyDict>>,
    validate_formats: Option<bool>,
    ignore_unknown_formats: Option<bool>,
    retriever: Option<&Bound<'a, PyAny>>,
    registry: Option<&'a registry::Registry>,
    base_uri: Option<String>,
    pattern_options: Option<&Bound<'a, PyAny>>,
    email_options: Option<&Bound<'a, PyAny>>,
    http_options: Option<&Bound<'a, PyAny>>,
    keywords: Option<&Bound<'a, PyDict>>,
    vocabularies: Option<Vec<String>>,
    offline: Option<bool>,
) -> PyResult<ValidationOptions<'a, Arc<dyn Retrieve>, F>>
where
    CustomKeyword: for<'i> jsonschema::Keyword<'i, F>,
{
    let mut options = jsonschema::options_for::<F>();
    #[cfg(not(target_arch = "wasm32"))]
    {
        options = options.with_retriever(DetachedRetriever(None));
    }
    if let Some(raw_draft_version) = draft {
        options = options.with_draft(get_draft(raw_draft_version)?);
    }
    if let Some(yes) = validate_formats {
        options = options.should_validate_formats(yes);
    }
    if let Some(yes) = ignore_unknown_formats {
        options = options.should_ignore_unknown_formats(yes);
    }
    for vocabulary in vocabularies.into_iter().flatten() {
        options = options.with_vocabulary(vocabulary);
    }
    let offline = offline.unwrap_or(false);
    if offline && retriever.is_some() {
        return Err(exceptions::PyValueError::new_err(
            "`offline` cannot be used together with `retriever`",
        ));
    }
    let retriever_was_provided = retriever.is_some();
    if let Some(formats) = formats {
        for (name, callback) in formats.iter() {
            if let Ok(number_format) = callback.cast::<NumberFormat>() {
                let check = number_format.get().check.clone_ref(callback.py());
                options = options.with_number_format(
                    name.to_string(),
                    move |number: &serde_json::Number| {
                        format_outcome(Python::attach(|py| {
                            let number = number_to_python(py, number)?;
                            check.call1(py, (number,))?.is_truthy(py)
                        }))
                    },
                );
                continue;
            }
            if !callback.is_callable() {
                return Err(exceptions::PyValueError::new_err(format!(
                    "Format checker for '{name}' must be a callable",
                )));
            }
            let callback: Py<PyAny> = callback.clone().unbind();
            options = options.with_format(name.to_string(), move |value: &str| {
                format_outcome(Python::attach(|py| {
                    let value = PyString::new(py, value);
                    callback.call(py, (value,), None)?.is_truthy(py)
                }))
            });
        }
    }
    if let Some(retriever) = retriever {
        let func = into_retriever(retriever)?;
        options = options.with_retriever(Retriever { func });
    }
    if let Some(registry) = registry {
        if !retriever_was_provided && !offline {
            if let Some(registry_retriever) = registry.retriever() {
                let func = Python::attach(|py| into_retriever(registry_retriever.bind(py)))?;
                options = options.with_retriever(Retriever { func });
            }
        }
        options = options.with_registry(registry.inner.as_ref());
    }
    if offline {
        options = options.offline();
    }
    if let Some(base_uri) = base_uri {
        options = options.with_base_uri(base_uri);
    }
    if let Some(pattern_options) = pattern_options {
        match crate::regex::extract_pattern_options(pattern_options)? {
            crate::regex::PyPatternOptions::Fancy(p) => {
                options = options.with_pattern_options(p);
            }
            crate::regex::PyPatternOptions::Regex(p) => {
                options = options.with_pattern_options(p);
            }
        }
    }
    if let Some(email_options) = email_options {
        let opts = email_options.extract::<Py<EmailOptions>>().map_err(|_| {
            exceptions::PyTypeError::new_err("email_options must be an instance of EmailOptions")
        })?;
        let email_opts = Python::attach(|py| {
            let opts = opts.borrow(py);
            let mut email_opts = jsonschema::EmailOptions::default();
            if opts.require_tld {
                email_opts = email_opts.with_required_tld();
            }
            if !opts.allow_domain_literal {
                email_opts = email_opts.without_domain_literal();
            }
            if !opts.allow_display_text {
                email_opts = email_opts.without_display_text();
            }
            if let Some(min) = opts.minimum_sub_domains {
                email_opts = email_opts.with_minimum_sub_domains(min);
            }
            email_opts
        });
        options = options.with_email_options(email_opts);
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(http_options) = http_options {
        let opts = http_options.extract::<HttpOptions>().map_err(|_| {
            exceptions::PyTypeError::new_err("http_options must be an instance of HttpOptions")
        })?;
        let mut http_opts = jsonschema::HttpOptions::new();
        if let Some(timeout) = opts.timeout {
            http_opts = http_opts.timeout(std::time::Duration::from_secs_f64(timeout));
        }
        if let Some(connect_timeout) = opts.connect_timeout {
            http_opts =
                http_opts.connect_timeout(std::time::Duration::from_secs_f64(connect_timeout));
        }
        if !opts.tls_verify {
            http_opts = http_opts.danger_accept_invalid_certs(true);
        }
        if let Some(ref ca_cert) = opts.ca_cert {
            http_opts = http_opts.add_root_certificate(ca_cert);
        }
        let retriever = jsonschema::HttpRetriever::new(&http_opts).map_err(|e| {
            exceptions::PyRuntimeError::new_err(format!("Failed to configure HTTP options: {e}"))
        })?;
        options = options.with_retriever(DetachedRetriever(Some(retriever)));
    }
    #[cfg(target_arch = "wasm32")]
    if http_options.is_some() {
        return Err(exceptions::PyValueError::new_err(
            "http_options is not supported on WebAssembly targets (the built-in HTTP retriever is unavailable); use a custom retriever= instead",
        ));
    }
    if let Some(keywords) = keywords {
        for (name, callback) in keywords.iter() {
            let name_str = name.to_string();
            if !callback.is_callable() {
                return Err(exceptions::PyValueError::new_err(format!(
                    "Keyword validator for '{name_str}' must be a callable",
                )));
            }

            let callback: Py<PyAny> = callback.clone().unbind();
            let name_for_closure = name_str.clone();

            // Instantiate with (parent_schema, value, schema_path), must have validate(instance) method
            options = options.with_keyword(
                name_str,
                move |parent: &serde_json::Map<String, serde_json::Value>,
                      value: &serde_json::Value,
                      path: jsonschema::paths::Location| {
                    let name_for_error = name_for_closure.clone();
                    Python::attach(|py| {
                        let py_schema = map_to_python(py, parent).map_err(|e| {
                            jsonschema::ValidationError::custom(format!(
                                "Failed to convert schema to Python: {e}"
                            ))
                        })?;
                        let py_value = value_to_python(py, value).map_err(|e| {
                            jsonschema::ValidationError::custom(format!(
                                "Failed to convert keyword value to Python: {e}"
                            ))
                        })?;
                        let py_path: Vec<Py<PyAny>> = path
                            .iter()
                            .map(|segment| match segment {
                                jsonschema::paths::LocationSegment::Property(p) => {
                                    PyString::new(py, p.as_ref()).into_any().unbind()
                                }
                                jsonschema::paths::LocationSegment::Index(i) => {
                                    pyo3::types::PyInt::new(py, i).into_any().unbind()
                                }
                            })
                            .collect();
                        let py_path_list = PyList::new(py, py_path).map_err(|e| {
                            jsonschema::ValidationError::custom(format!(
                                "Failed to create path list: {e}"
                            ))
                        })?;

                        match callback.call1(py, (py_schema, py_value, py_path_list)) {
                            Ok(instance) => Ok(Box::new(CustomKeyword { instance })
                                as Box<dyn for<'i> jsonschema::Keyword<'i, F>>),
                            Err(e) => Err(jsonschema::ValidationError::custom(format!(
                                "Failed to instantiate keyword class '{name_for_error}': {e}"
                            ))),
                        }
                    })
                },
            );
        }
    }
    Ok(options)
}

// Errors the `Pyo3` representation recorded while reading the instance, since validation cannot
// return them. A recorded error outranks whatever `run` produced: building a validation error can
// itself hit an unreadable part of the instance, and that error is the accurate one.
fn surface_pending_errors<T>(
    instance: &Bound<'_, PyAny>,
    holding: bool,
    run: impl FnMut() -> PyResult<T>,
) -> PyResult<T> {
    inspect(instance.as_borrowed(), holding, run)?
}

// Custom formats and keywords run Python code mid-validation, which may drop parts of the instance
// the validator still reads.
fn runs_python(formats: Option<&Bound<'_, PyDict>>, keywords: Option<&Bound<'_, PyDict>>) -> bool {
    formats.is_some_and(|formats| !formats.is_empty())
        || keywords.is_some_and(|keywords| !keywords.is_empty())
}

fn iter_on_error(
    py: Python<'_>,
    validator: &jsonschema::Validator<Pyo3>,
    instance: &Bound<'_, PyAny>,
    mask: Option<&str>,
    holding: bool,
) -> PyResult<ValidationErrorIter> {
    surface_pending_errors(instance, holding, || {
        let _scope = KeywordCauseScope::enter();
        let mut pyerrors = vec![];
        let node = instance.as_borrowed();

        panic::catch_unwind(AssertUnwindSafe(|| {
            for error in validator.iter_errors(node) {
                pyerrors.push(into_py_err(
                    py,
                    error,
                    mask,
                    Some(InstanceRoot::Object(instance)),
                )?);
            }
            PyResult::Ok(())
        }))
        .map_err(handle_format_checked_panic)??;
        Ok(ValidationErrorIter {
            iter: pyerrors.into_iter(),
        })
    })
}

fn raise_on_error(
    py: Python<'_>,
    validator: &jsonschema::Validator<Pyo3>,
    instance: &Bound<'_, PyAny>,
    mask: Option<&str>,
    holding: bool,
) -> PyResult<()> {
    surface_pending_errors(instance, holding, || {
        let _scope = KeywordCauseScope::enter();
        let node = instance.as_borrowed();
        let error = panic::catch_unwind(AssertUnwindSafe(|| validator.validate(node)))
            .map_err(handle_format_checked_panic)?
            .err();
        error.map_or_else(
            || Ok(()),
            |err| {
                Err(into_py_err(
                    py,
                    err,
                    mask,
                    Some(InstanceRoot::Object(instance)),
                )?)
            },
        )
    })
}

fn is_ascii_number(s: &str) -> bool {
    !s.is_empty() && s.as_bytes().iter().all(|&b| b.is_ascii_digit())
}

fn to_error_message(
    error: &jsonschema::ValidationError<'_>,
    mut message: String,
    mask: Option<&str>,
) -> String {
    // The verbose message roughly doubles the short one
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

    let mut schema_path = error.schema_path().as_str();

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
    for segment in error.instance_path().as_str().split('/').skip(1) {
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
        serde_json::to_writer(&mut writer, error.instance()).expect("Failed to serialize JSON");
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

/// is_valid(schema, instance, draft=None, formats=None, validate_formats=None, ignore_unknown_formats=True, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None, email_options=None, http_options=None, keywords=None, vocabularies=None, offline=None)
///
/// Return whether `instance` is valid under `schema`. Stops at the first error.
///
///     >>> is_valid({"minimum": 5}, 3)
///     False
///
/// To validate many instances against one schema, use `validator_for(schema).is_valid`.
#[pyfunction]
#[pyo3(signature = (schema, instance, draft=None, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None, email_options=None, http_options=None, keywords=None, vocabularies=None, offline=None))]
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
    email_options: Option<&Bound<'_, PyAny>>,
    http_options: Option<&Bound<'_, PyAny>>,
    keywords: Option<&Bound<'_, PyDict>>,
    vocabularies: Option<Vec<String>>,
    offline: Option<bool>,
) -> PyResult<bool> {
    let options = make_options::<Pyo3>(
        draft,
        formats,
        validate_formats,
        ignore_unknown_formats,
        retriever,
        registry,
        base_uri,
        pattern_options,
        email_options,
        http_options,
        keywords,
        vocabularies,
        offline,
    )?;
    let schema = ser::to_value(schema)?;
    match options.build(&schema) {
        Ok(validator) => surface_pending_errors(instance, runs_python(formats, keywords), || {
            panic::catch_unwind(AssertUnwindSafe(|| {
                Ok(validator.is_valid(instance.as_borrowed()))
            }))
            .map_err(handle_format_checked_panic)?
        }),
        Err(error) => Err(into_py_err(py, error, mask.as_deref(), None)?),
    }
}

/// validate(schema, instance, draft=None, formats=None, validate_formats=None, ignore_unknown_formats=True, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None, email_options=None, http_options=None, keywords=None, vocabularies=None, offline=None)
///
/// Raise `ValidationError` for the first error of `instance` under `schema`.
///
///     >>> validate({"minimum": 5}, 3)
///     ...
///     ValidationError: 3 is less than the minimum of 5
///
/// To validate many instances against one schema, use `validator_for(schema).validate`.
#[pyfunction]
#[pyo3(signature = (schema, instance, draft=None, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None, email_options=None, http_options=None, keywords=None, vocabularies=None, offline=None))]
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
    email_options: Option<&Bound<'_, PyAny>>,
    http_options: Option<&Bound<'_, PyAny>>,
    keywords: Option<&Bound<'_, PyDict>>,
    vocabularies: Option<Vec<String>>,
    offline: Option<bool>,
) -> PyResult<()> {
    let options = make_options::<Pyo3>(
        draft,
        formats,
        validate_formats,
        ignore_unknown_formats,
        retriever,
        registry,
        base_uri,
        pattern_options,
        email_options,
        http_options,
        keywords,
        vocabularies,
        offline,
    )?;
    let schema = ser::to_value(schema)?;
    match options.build(&schema) {
        Ok(validator) => raise_on_error(
            py,
            &validator,
            instance,
            mask.as_deref(),
            runs_python(formats, keywords),
        ),
        Err(error) => Err(into_py_err(py, error, mask.as_deref(), None)?),
    }
}

/// iter_errors(schema, instance, draft=None, formats=None, validate_formats=None, ignore_unknown_formats=True, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None, email_options=None, http_options=None, keywords=None, vocabularies=None, offline=None)
///
/// Iterate over every `ValidationError` of `instance` under `schema`.
///
///     >>> print(next(iter_errors({"minimum": 5}, 3)).message)
///     3 is less than the minimum of 5
///
/// To validate many instances against one schema, use `validator_for(schema).iter_errors`.
#[pyfunction]
#[pyo3(signature = (schema, instance, draft=None, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None, email_options=None, http_options=None, keywords=None, vocabularies=None, offline=None))]
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
    email_options: Option<&Bound<'_, PyAny>>,
    http_options: Option<&Bound<'_, PyAny>>,
    keywords: Option<&Bound<'_, PyDict>>,
    vocabularies: Option<Vec<String>>,
    offline: Option<bool>,
) -> PyResult<ValidationErrorIter> {
    let options = make_options::<Pyo3>(
        draft,
        formats,
        validate_formats,
        ignore_unknown_formats,
        retriever,
        registry,
        base_uri,
        pattern_options,
        email_options,
        http_options,
        keywords,
        vocabularies,
        offline,
    )?;
    let schema = ser::to_value(schema)?;
    match options.build(&schema) {
        Ok(validator) => iter_on_error(
            py,
            &validator,
            instance,
            mask.as_deref(),
            runs_python(formats, keywords),
        ),
        Err(error) => Err(into_py_err(py, error, mask.as_deref(), None)?),
    }
}

/// evaluate(schema, instance, draft=None, formats=None, validate_formats=None, ignore_unknown_formats=True, retriever=None, registry=None, base_uri=None, pattern_options=None, email_options=None, http_options=None, keywords=None, vocabularies=None, offline=None)
///
/// Evaluate `instance` against `schema` and return an `Evaluation` holding the output formats.
///
///     >>> evaluation = evaluate({"type": "integer"}, "oops")
///     >>> evaluation.flag()["valid"]
///     False
///     >>> print(evaluation.errors()[0]["error"])
///     "oops" is not of type "integer"
#[pyfunction]
#[pyo3(signature = (schema, instance, draft=None, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, registry=None, base_uri=None, pattern_options=None, email_options=None, http_options=None, keywords=None, vocabularies=None, offline=None))]
#[allow(clippy::needless_pass_by_value)]
fn evaluate(
    py: Python<'_>,
    schema: &Bound<'_, PyAny>,
    instance: &Bound<'_, PyAny>,
    draft: Option<u8>,
    formats: Option<&Bound<'_, PyDict>>,
    validate_formats: Option<bool>,
    ignore_unknown_formats: Option<bool>,
    retriever: Option<&Bound<'_, PyAny>>,
    registry: Option<&registry::Registry>,
    base_uri: Option<String>,
    pattern_options: Option<&Bound<'_, PyAny>>,
    email_options: Option<&Bound<'_, PyAny>>,
    http_options: Option<&Bound<'_, PyAny>>,
    keywords: Option<&Bound<'_, PyDict>>,
    vocabularies: Option<Vec<String>>,
    offline: Option<bool>,
) -> PyResult<PyEvaluation> {
    let _scope = KeywordCauseScope::enter();
    let options = make_options::<Pyo3>(
        draft,
        formats,
        validate_formats,
        ignore_unknown_formats,
        retriever,
        registry,
        base_uri,
        pattern_options,
        email_options,
        http_options,
        keywords,
        vocabularies,
        offline,
    )?;
    let schema = ser::to_value(schema)?;
    let validator = match options.build(&schema) {
        Ok(validator) => validator,
        Err(error) => return Err(into_py_err(py, error, None, None)?),
    };
    let evaluation = surface_pending_errors(instance, runs_python(formats, keywords), || {
        panic::catch_unwind(AssertUnwindSafe(|| {
            validator.evaluate(instance.as_borrowed())
        }))
        .map_err(handle_format_checked_panic)
    })?;
    Ok(PyEvaluation::new(evaluation))
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

/// JsonTextValidator(schema, draft=None, formats=None, validate_formats=None, ignore_unknown_formats=True, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None, email_options=None, http_options=None, keywords=None, vocabularies=None, offline=None)
///
/// A validator for instances given as JSON text, without building Python objects for them. The
/// draft comes from `draft`, or from the schema's `$schema` (Draft 2020-12 without one).
///
///     >>> validator = JsonTextValidator({"minimum": 5})
///     >>> validator.is_valid("3")
///     False
#[pyclass(module = "jsonschema_rs", frozen)]
struct JsonTextValidator {
    validator: jsonschema::Validator<JsonText>,
    mask: Option<String>,
}

fn json_text<'a>(instance: &'a Bound<'_, PyAny>) -> PyResult<Cow<'a, [u8]>> {
    if let Ok(string) = instance.cast::<PyString>() {
        return Ok(Cow::Borrowed(string.to_str()?.as_bytes()));
    }
    if let Ok(bytes) = instance.cast::<PyBytes>() {
        let data = bytes.as_bytes();
        // `json.loads` also reads UTF-16 and UTF-32, and UTF-8 behind a byte order mark, none of
        // which UTF-8 JSON text starts with.
        let other_encoding = data.starts_with(b"\xef\xbb\xbf")
            || data.starts_with(b"\xfe\xff")
            || data.starts_with(b"\xff\xfe")
            || (data.len() >= 2 && (data[0] == 0 || data[1] == 0));
        if !other_encoding {
            return Ok(Cow::Borrowed(data));
        }
        let encoding = instance
            .py()
            .import("json")?
            .call_method1("detect_encoding", (bytes,))?;
        let text = bytes.call_method1("decode", (encoding,))?;
        return Ok(Cow::Owned(
            text.cast_into::<PyString>()?.to_str()?.as_bytes().to_vec(),
        ));
    }
    Err(exceptions::PyTypeError::new_err(format!(
        "JSON text must be str or bytes, not {}",
        instance.get_type().name()?
    )))
}

fn parse_json_text(text: &[u8]) -> PyResult<JsonTextDocument<'_>> {
    JsonText::parse(text).map_err(|error| PyValueError::new_err(format!("Invalid JSON: {error}")))
}

#[pymethods]
impl JsonTextValidator {
    #[new]
    #[pyo3(signature = (schema, draft=None, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None, email_options=None, http_options=None, keywords=None, vocabularies=None, offline=None))]
    fn new(
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
        email_options: Option<&Bound<'_, PyAny>>,
        http_options: Option<&Bound<'_, PyAny>>,
        keywords: Option<&Bound<'_, PyDict>>,
        vocabularies: Option<Vec<String>>,
        offline: Option<bool>,
    ) -> PyResult<Self> {
        let schema = parse_schema_str(schema)?;
        let options = make_options::<JsonText>(
            draft,
            formats,
            validate_formats,
            ignore_unknown_formats,
            retriever,
            registry,
            base_uri,
            pattern_options,
            email_options,
            http_options,
            keywords,
            vocabularies,
            offline,
        )?;
        match options.build(&schema) {
            Ok(validator) => Ok(Self { validator, mask }),
            Err(error) => Err(into_py_err(py, error, mask.as_deref(), None)?),
        }
    }
    /// is_valid(instance)
    ///
    /// Return whether the JSON text `instance` holds a valid document. Stops at the first error.
    ///
    ///     >>> validator = JsonTextValidator({"minimum": 5})
    ///     >>> validator.is_valid(b"3")
    ///     False
    fn is_valid(&self, instance: &Bound<'_, PyAny>) -> PyResult<bool> {
        let text = json_text(instance)?;
        let document = parse_json_text(&text)?;
        panic::catch_unwind(AssertUnwindSafe(|| {
            self.validator.is_valid(document.root())
        }))
        .map_err(handle_format_checked_panic)
    }
    /// validate(instance)
    ///
    /// Raise `ValidationError` for the first error of the document in the JSON text `instance`.
    ///
    ///     >>> validator = JsonTextValidator({"minimum": 5})
    ///     >>> validator.validate("3")
    ///     ...
    ///     ValidationError: 3 is less than the minimum of 5
    fn validate(&self, py: Python<'_>, instance: &Bound<'_, PyAny>) -> PyResult<()> {
        let text = json_text(instance)?;
        let document = parse_json_text(&text)?;
        let _scope = KeywordCauseScope::enter();
        let error = panic::catch_unwind(AssertUnwindSafe(|| {
            self.validator.validate(document.root()).err()
        }))
        .map_err(handle_format_checked_panic)?;
        match error {
            Some(error) => {
                let root = InstanceRoot::Text(document.root());
                Err(into_py_err(py, error, self.mask.as_deref(), Some(root))?)
            }
            None => Ok(()),
        }
    }
    /// iter_errors(instance)
    ///
    /// Iterate over every `ValidationError` of the document in the JSON text `instance`.
    ///
    ///     >>> validator = JsonTextValidator({"minimum": 5})
    ///     >>> print(next(validator.iter_errors("3")).message)
    ///     3 is less than the minimum of 5
    fn iter_errors(
        &self,
        py: Python<'_>,
        instance: &Bound<'_, PyAny>,
    ) -> PyResult<ValidationErrorIter> {
        let text = json_text(instance)?;
        let document = parse_json_text(&text)?;
        let _scope = KeywordCauseScope::enter();
        let mut errors = vec![];
        panic::catch_unwind(AssertUnwindSafe(|| {
            let root = InstanceRoot::Text(document.root());
            for error in self.validator.iter_errors(document.root()) {
                errors.push(into_py_err(py, error, self.mask.as_deref(), Some(root))?);
            }
            PyResult::Ok(())
        }))
        .map_err(handle_format_checked_panic)??;
        Ok(ValidationErrorIter {
            iter: errors.into_iter(),
        })
    }
    /// evaluate(instance)
    ///
    /// Evaluate the document in the JSON text `instance` and return an `Evaluation` holding the
    /// output formats.
    ///
    ///     >>> validator = JsonTextValidator({"type": "integer"})
    ///     >>> validator.evaluate("true").flag()["valid"]
    ///     False
    fn evaluate(&self, instance: &Bound<'_, PyAny>) -> PyResult<PyEvaluation> {
        let text = json_text(instance)?;
        let document = parse_json_text(&text)?;
        let _scope = KeywordCauseScope::enter();
        let evaluation = panic::catch_unwind(AssertUnwindSafe(|| {
            self.validator.evaluate(document.root())
        }))
        .map_err(handle_format_checked_panic)?;
        Ok(PyEvaluation::new(evaluation))
    }
    fn __repr__(&self) -> &'static str {
        match self.validator.draft() {
            Draft::Draft4 => "<JsonTextValidator: Draft4>",
            Draft::Draft6 => "<JsonTextValidator: Draft6>",
            Draft::Draft7 => "<JsonTextValidator: Draft7>",
            Draft::Draft201909 => "<JsonTextValidator: Draft201909>",
            _ => "<JsonTextValidator: Draft202012>",
        }
    }
}

#[pyclass(module = "jsonschema_rs", subclass)]
#[allow(clippy::struct_field_names)]
struct Validator {
    validator: jsonschema::Validator<Pyo3>,
    mask: Option<String>,
    holding: bool,
}

/// Compiled validators keyed by URI-fragment JSON pointer (e.g. ``"#/$defs/User"``), from
/// :func:`validator_map_for`.
#[pyclass(module = "jsonschema_rs")]
struct ValidatorMap {
    inner: jsonschema::ValidatorMap<Pyo3>,
    mask: Option<String>,
    holding: bool,
}

impl ValidatorMap {
    fn entry<'py>(
        &self,
        py: Python<'py>,
        validator: &jsonschema::Validator<Pyo3>,
    ) -> PyResult<Bound<'py, Validator>> {
        Validator {
            validator: validator.clone(),
            mask: self.mask.clone(),
            holding: self.holding,
        }
        .into_draft_instance(py)
    }
}

#[pymethods]
impl ValidatorMap {
    /// get(pointer)
    ///
    /// Return the validator for ``pointer``, or ``None`` if not found.
    ///
    ///     >>> m = validator_map_for({"$defs": {"T": {"type": "string"}}})
    ///     >>> v = m.get("#/$defs/T")
    ///
    fn get<'py>(&self, py: Python<'py>, pointer: &str) -> PyResult<Option<Bound<'py, Validator>>> {
        self.inner
            .get(pointer)
            .map(|v| self.entry(py, v))
            .transpose()
    }

    fn __getitem__<'py>(&self, py: Python<'py>, pointer: &str) -> PyResult<Bound<'py, Validator>> {
        match self.inner.get(pointer) {
            Some(v) => self.entry(py, v),
            None => Err(PyKeyError::new_err(pointer.to_owned())),
        }
    }

    fn __contains__(&self, pointer: &str) -> bool {
        self.inner.contains_key(pointer)
    }

    /// keys()
    ///
    /// Return all URI-fragment JSON pointer keys in this map.
    fn keys(&self) -> Vec<String> {
        self.inner.keys().map(str::to_owned).collect()
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }
}

/// validator_for(schema, formats=None, validate_formats=None, ignore_unknown_formats=True, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None, email_options=None, http_options=None, keywords=None, vocabularies=None, offline=None)
///
/// Create a validator for `schema`, an instance of the draft class its `$schema` names (Draft
/// 2020-12 without one).
///
///     >>> validator = validator_for({"minimum": 5})
///     >>> validator.is_valid(3)
///     False
///
#[pyfunction]
#[pyo3(signature = (schema, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None, email_options=None, http_options=None, keywords=None, vocabularies=None, offline=None))]
fn validator_for<'py>(
    py: Python<'py>,
    schema: &Bound<'_, PyAny>,
    formats: Option<&Bound<'_, PyDict>>,
    validate_formats: Option<bool>,
    ignore_unknown_formats: Option<bool>,
    retriever: Option<&Bound<'_, PyAny>>,
    registry: Option<&registry::Registry>,
    mask: Option<String>,
    base_uri: Option<String>,
    pattern_options: Option<&Bound<'_, PyAny>>,
    email_options: Option<&Bound<'_, PyAny>>,
    http_options: Option<&Bound<'_, PyAny>>,
    keywords: Option<&Bound<'_, PyDict>>,
    vocabularies: Option<Vec<String>>,
    offline: Option<bool>,
) -> PyResult<Bound<'py, Validator>> {
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
        email_options,
        http_options,
        keywords,
        vocabularies,
        offline,
    )?
    .into_draft_instance(py)
}

/// validator_map_for(schema, formats=None, validate_formats=None, ignore_unknown_formats=True, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None, email_options=None, http_options=None, keywords=None, vocabularies=None, offline=None)
///
/// Compile all subschemas in ``schema`` into a map keyed by URI-fragment JSON pointer. The root
/// schema is under ``"#"``.
///
///     >>> schema = {"$defs": {"User": {"type": "object"}}}
///     >>> m = validator_map_for(schema)
///     >>> v = m.get("#/$defs/User")
///     >>> v.is_valid({})
///     True
///
#[pyfunction]
#[pyo3(signature = (schema, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None, email_options=None, http_options=None, keywords=None, vocabularies=None, offline=None))]
fn validator_map_for(
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
    email_options: Option<&Bound<'_, PyAny>>,
    http_options: Option<&Bound<'_, PyAny>>,
    keywords: Option<&Bound<'_, PyDict>>,
    vocabularies: Option<Vec<String>>,
    offline: Option<bool>,
) -> PyResult<ValidatorMap> {
    let schema = parse_schema_str(schema)?;
    let options = make_options::<Pyo3>(
        None,
        formats,
        validate_formats,
        ignore_unknown_formats,
        retriever,
        registry,
        base_uri,
        pattern_options,
        email_options,
        http_options,
        keywords,
        vocabularies,
        offline,
    )?;
    match options.build_map(&schema) {
        Ok(inner) => Ok(ValidatorMap {
            inner,
            mask,
            holding: runs_python(formats, keywords),
        }),
        Err(error) => Err(into_py_err(py, error, mask.as_deref(), None)?),
    }
}

/// bundle(schema, /, *, retriever=None, registry=None, draft=None, base_uri=None, offline=None)
///
/// Bundle a JSON Schema into a Compound Schema Document.
///
/// All externally-referenced schemas reachable via `$ref` are embedded in a
/// draft-appropriate container (`definitions` for Draft 4/6/7, `$defs` for
/// Draft 2019-09/2020-12). Original `$ref` values are preserved unchanged.
///
/// :raises ReferencingError: if a referenced document cannot be retrieved.
/// :raises ValueError: for any other `$ref` that cannot be resolved.
///
///     >>> bundled = bundle({"$ref": "https://example.com/thing.json"}, registry=my_registry)
///
#[pyfunction]
#[pyo3(signature = (schema, /, *, retriever=None, registry=None, draft=None, base_uri=None, offline=None))]
fn bundle(
    py: Python<'_>,
    schema: &Bound<'_, PyAny>,
    retriever: Option<&Bound<'_, PyAny>>,
    registry: Option<&registry::Registry>,
    draft: Option<u8>,
    base_uri: Option<String>,
    offline: Option<bool>,
) -> PyResult<Py<PyAny>> {
    let schema_value = ser::to_value(schema)?;
    let options = make_options::<Pyo3>(
        draft, None, None, None, retriever, registry, base_uri, None, None, None, None, None,
        offline,
    )?;
    match options.bundle(&schema_value) {
        Ok(bundled) => value_to_python(py, &bundled),
        Err(e @ jsonschema::ReferencingError::Unretrievable { .. }) => {
            Err(referencing_error_pyerr(py, e.to_string())?)
        }
        Err(e) => Err(exceptions::PyValueError::new_err(e.to_string())),
    }
}

/// dereference(schema, /, *, retriever=None, registry=None, draft=None, base_uri=None, offline=None)
///
/// Recursively inline all `$ref` values in a JSON Schema.
///
/// Circular references are left in place as `$ref` strings.
///
/// :param schema: The JSON Schema to dereference.
/// :param retriever: Optional callable ``(uri: str) -> dict`` for fetching external schemas.
/// :param registry: Optional pre-built :class:`Registry` of known schemas.
/// :param offline: Refuse to fetch anything outside ``registry``.
/// :param draft: Optional draft version (e.g. ``jsonschema_rs.Draft202012``).
/// :param base_uri: Optional base URI for the root schema.
/// :raises ReferencingError: if a referenced document cannot be retrieved.
/// :raises ValueError: for any other `$ref` that cannot be resolved.
///
///     >>> result = dereference({"$defs": {"t": {"type": "string"}}, "properties": {"x": {"$ref": "#/$defs/t"}}})
///
#[pyfunction]
#[pyo3(signature = (schema, /, *, retriever=None, registry=None, draft=None, base_uri=None, offline=None))]
fn dereference(
    py: Python<'_>,
    schema: &Bound<'_, PyAny>,
    retriever: Option<&Bound<'_, PyAny>>,
    registry: Option<&registry::Registry>,
    draft: Option<u8>,
    base_uri: Option<String>,
    offline: Option<bool>,
) -> PyResult<Py<PyAny>> {
    let schema_value = ser::to_value(schema)?;
    let options = make_options::<Pyo3>(
        draft, None, None, None, retriever, registry, base_uri, None, None, None, None, None,
        offline,
    )?;
    match options.dereference(&schema_value) {
        Ok(result) => value_to_python(py, &result),
        Err(e @ jsonschema::ReferencingError::Unretrievable { .. }) => {
            Err(referencing_error_pyerr(py, e.to_string())?)
        }
        Err(e) => Err(exceptions::PyValueError::new_err(e.to_string())),
    }
}

fn parse_schema_str(schema: &Bound<'_, PyAny>) -> PyResult<serde_json::Value> {
    let obj_ptr = schema.as_ptr();
    let object_type = unsafe { pyo3::ffi::Py_TYPE(obj_ptr) };
    if unsafe { object_type == types::STR_TYPE } {
        let mut str_size: pyo3::ffi::Py_ssize_t = 0;
        let ptr = unsafe { PyUnicode_AsUTF8AndSize(obj_ptr, &raw mut str_size) };
        let slice = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), str_size as usize) };
        serde_json::from_slice(slice)
            .map_err(|error| PyValueError::new_err(format!("Invalid string: {error}")))
    } else {
        ser::to_value(schema)
    }
}

fn detect_draft(schema: &Bound<'_, PyAny>) -> PyResult<Draft> {
    if let Ok(dict) = schema.cast::<PyDict>() {
        if let Ok(Some(val)) = dict.get_item("$schema") {
            if let Ok(s) = val.extract::<&str>() {
                return Ok(Draft::from_schema_uri(s));
            }
        }
        return Ok(Draft::default());
    }
    let value = parse_schema_str(schema)?;
    if let serde_json::Value::Object(map) = &value {
        if let Some(serde_json::Value::String(s)) = map.get("$schema") {
            return Ok(Draft::from_schema_uri(s));
        }
    }
    Ok(Draft::default())
}

/// validator_cls_for(schema)
///
/// Detect the JSON Schema draft for a schema and return the corresponding validator class.
/// Draft is detected automatically from the `$schema` field. Defaults to Draft202012Validator.
///
///     >>> cls = validator_cls_for({"type": "string"})
///     >>> validator = cls({"type": "string"})
///     >>> cls = validator_cls_for({"$schema": "http://json-schema.org/draft-07/schema#", "type": "string"})
///     >>> validator = cls({"$schema": "http://json-schema.org/draft-07/schema#", "type": "string"})
///
#[pyfunction]
fn validator_cls_for(py: Python<'_>, schema: &Bound<'_, PyAny>) -> PyResult<Py<PyType>> {
    let cls: Bound<'_, PyType> = match detect_draft(schema)? {
        Draft::Draft4 => py.get_type::<Draft4Validator>(),
        Draft::Draft6 => py.get_type::<Draft6Validator>(),
        Draft::Draft7 => py.get_type::<Draft7Validator>(),
        Draft::Draft201909 => py.get_type::<Draft201909Validator>(),
        Draft::Draft202012 | Draft::Unknown | _ => py.get_type::<Draft202012Validator>(),
    };
    Ok(cls.unbind())
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
    email_options: Option<&Bound<'_, PyAny>>,
    http_options: Option<&Bound<'_, PyAny>>,
    keywords: Option<&Bound<'_, PyDict>>,
    vocabularies: Option<Vec<String>>,
    offline: Option<bool>,
) -> PyResult<Validator> {
    let schema = parse_schema_str(schema)?;
    let options = make_options::<Pyo3>(
        draft,
        formats,
        validate_formats,
        ignore_unknown_formats,
        retriever,
        registry,
        base_uri,
        pattern_options,
        email_options,
        http_options,
        keywords,
        vocabularies,
        offline,
    )?;
    match options.build(&schema) {
        Ok(validator) => Ok(Validator {
            validator,
            mask,
            holding: runs_python(formats, keywords),
        }),
        Err(error) => Err(into_py_err(py, error, mask.as_deref(), None)?),
    }
}

impl Validator {
    /// Wrap the validator in the Python class of its draft, so `isinstance` checks against draft classes hold.
    fn into_draft_instance(self, py: Python<'_>) -> PyResult<Bound<'_, Validator>> {
        let draft = self.validator.draft();
        let initializer = PyClassInitializer::from(self);
        Ok(match draft {
            Draft::Draft4 => {
                Bound::new(py, initializer.add_subclass(Draft4Validator))?.into_super()
            }
            Draft::Draft6 => {
                Bound::new(py, initializer.add_subclass(Draft6Validator))?.into_super()
            }
            Draft::Draft7 => {
                Bound::new(py, initializer.add_subclass(Draft7Validator))?.into_super()
            }
            Draft::Draft201909 => {
                Bound::new(py, initializer.add_subclass(Draft201909Validator))?.into_super()
            }
            Draft::Draft202012 | _ => {
                Bound::new(py, initializer.add_subclass(Draft202012Validator))?.into_super()
            }
        })
    }
}

#[pymethods]
impl Validator {
    #[new]
    #[pyo3(signature = (schema, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, registry = None, mask=None, base_uri=None, pattern_options=None, email_options=None, http_options=None, keywords=None, vocabularies=None, offline=None))]
    fn new<'py>(
        py: Python<'py>,
        schema: &Bound<'_, PyAny>,
        formats: Option<&Bound<'_, PyDict>>,
        validate_formats: Option<bool>,
        ignore_unknown_formats: Option<bool>,
        retriever: Option<&Bound<'_, PyAny>>,
        registry: Option<&registry::Registry>,
        mask: Option<String>,
        base_uri: Option<String>,
        pattern_options: Option<&Bound<'_, PyAny>>,
        email_options: Option<&Bound<'_, PyAny>>,
        http_options: Option<&Bound<'_, PyAny>>,
        keywords: Option<&Bound<'_, PyDict>>,
        vocabularies: Option<Vec<String>>,
        offline: Option<bool>,
    ) -> PyResult<Bound<'py, Self>> {
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
            email_options,
            http_options,
            keywords,
            vocabularies,
            offline,
        )
    }
    /// is_valid(instance)
    ///
    /// Return whether `instance` is valid. Stops at the first error.
    ///
    ///     >>> validator = validator_for({"minimum": 5})
    ///     >>> validator.is_valid(3)
    ///     False
    fn is_valid(&self, instance: &Bound<'_, PyAny>) -> PyResult<bool> {
        // The panic is mapped after the call, so the call itself carries no `PyResult`.
        inspect(instance.as_borrowed(), self.holding, || {
            panic::catch_unwind(AssertUnwindSafe(|| {
                self.validator.is_valid(instance.as_borrowed())
            }))
        })?
        .map_err(handle_format_checked_panic)
    }
    /// validate(instance)
    ///
    /// Raise `ValidationError` for the first error of `instance`.
    ///
    ///     >>> validator = validator_for({"minimum": 5})
    ///     >>> validator.validate(3)
    ///     ...
    ///     ValidationError: 3 is less than the minimum of 5
    fn validate(&self, py: Python<'_>, instance: &Bound<'_, PyAny>) -> PyResult<()> {
        raise_on_error(
            py,
            &self.validator,
            instance,
            self.mask.as_deref(),
            self.holding,
        )
    }
    /// iter_errors(instance)
    ///
    /// Iterate over every `ValidationError` of `instance`.
    ///
    ///     >>> validator = validator_for({"minimum": 5})
    ///     >>> print(next(validator.iter_errors(3)).message)
    ///     3 is less than the minimum of 5
    fn iter_errors(
        &self,
        py: Python<'_>,
        instance: &Bound<'_, PyAny>,
    ) -> PyResult<ValidationErrorIter> {
        iter_on_error(
            py,
            &self.validator,
            instance,
            self.mask.as_deref(),
            self.holding,
        )
    }
    /// evaluate(instance)
    ///
    /// Evaluate `instance` and return an `Evaluation` holding the output formats.
    ///
    ///     >>> validator = validator_for({"type": "integer"})
    ///     >>> validator.evaluate("oops").flag()["valid"]
    ///     False
    fn evaluate(&self, instance: &Bound<'_, PyAny>) -> PyResult<PyEvaluation> {
        surface_pending_errors(instance, self.holding, || {
            let _scope = KeywordCauseScope::enter();
            let evaluation = panic::catch_unwind(AssertUnwindSafe(|| {
                self.validator.evaluate(instance.as_borrowed())
            }))
            .map_err(handle_format_checked_panic)?;
            Ok(PyEvaluation::new(evaluation))
        })
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

/// Draft4Validator(schema, formats=None, validate_formats=None, ignore_unknown_formats=True, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None, email_options=None, http_options=None, keywords=None, vocabularies=None, offline=None)
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
    #[pyo3(signature = (schema, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None, email_options=None, http_options=None, keywords=None, vocabularies=None, offline=None))]
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
        email_options: Option<&Bound<'_, PyAny>>,
        http_options: Option<&Bound<'_, PyAny>>,
        keywords: Option<&Bound<'_, PyDict>>,
        vocabularies: Option<Vec<String>>,
        offline: Option<bool>,
    ) -> PyResult<PyClassInitializer<Self>> {
        Ok(PyClassInitializer::from(validator_for_impl(
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
            email_options,
            http_options,
            keywords,
            vocabularies,
            offline,
        )?)
        .add_subclass(Draft4Validator {}))
    }
}

/// Draft6Validator(schema, formats=None, validate_formats=None, ignore_unknown_formats=True, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None, email_options=None, http_options=None, keywords=None, vocabularies=None, offline=None)
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
    #[pyo3(signature = (schema, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None, email_options=None, http_options=None, keywords=None, vocabularies=None, offline=None))]
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
        email_options: Option<&Bound<'_, PyAny>>,
        http_options: Option<&Bound<'_, PyAny>>,
        keywords: Option<&Bound<'_, PyDict>>,
        vocabularies: Option<Vec<String>>,
        offline: Option<bool>,
    ) -> PyResult<PyClassInitializer<Self>> {
        Ok(PyClassInitializer::from(validator_for_impl(
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
            email_options,
            http_options,
            keywords,
            vocabularies,
            offline,
        )?)
        .add_subclass(Draft6Validator {}))
    }
}

/// Draft7Validator(schema, formats=None, validate_formats=None, ignore_unknown_formats=True, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None, email_options=None, http_options=None, keywords=None, vocabularies=None, offline=None)
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
    #[pyo3(signature = (schema, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None, email_options=None, http_options=None, keywords=None, vocabularies=None, offline=None))]
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
        email_options: Option<&Bound<'_, PyAny>>,
        http_options: Option<&Bound<'_, PyAny>>,
        keywords: Option<&Bound<'_, PyDict>>,
        vocabularies: Option<Vec<String>>,
        offline: Option<bool>,
    ) -> PyResult<PyClassInitializer<Self>> {
        Ok(PyClassInitializer::from(validator_for_impl(
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
            email_options,
            http_options,
            keywords,
            vocabularies,
            offline,
        )?)
        .add_subclass(Draft7Validator {}))
    }
}

/// Draft201909Validator(schema, formats=None, validate_formats=None, ignore_unknown_formats=True, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None, email_options=None, http_options=None, keywords=None, vocabularies=None, offline=None)
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
    #[pyo3(signature = (schema, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None, email_options=None, http_options=None, keywords=None, vocabularies=None, offline=None))]
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
        email_options: Option<&Bound<'_, PyAny>>,
        http_options: Option<&Bound<'_, PyAny>>,
        keywords: Option<&Bound<'_, PyDict>>,
        vocabularies: Option<Vec<String>>,
        offline: Option<bool>,
    ) -> PyResult<PyClassInitializer<Self>> {
        Ok(PyClassInitializer::from(validator_for_impl(
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
            email_options,
            http_options,
            keywords,
            vocabularies,
            offline,
        )?)
        .add_subclass(Draft201909Validator {}))
    }
}

/// Draft202012Validator(schema, formats=None, validate_formats=None, ignore_unknown_formats=True, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None, email_options=None, http_options=None, keywords=None, vocabularies=None, offline=None)
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
    #[pyo3(signature = (schema, formats=None, validate_formats=None, ignore_unknown_formats=true, retriever=None, registry=None, mask=None, base_uri=None, pattern_options=None, email_options=None, http_options=None, keywords=None, vocabularies=None, offline=None))]
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
        email_options: Option<&Bound<'_, PyAny>>,
        http_options: Option<&Bound<'_, PyAny>>,
        keywords: Option<&Bound<'_, PyDict>>,
        vocabularies: Option<Vec<String>>,
        offline: Option<bool>,
    ) -> PyResult<PyClassInitializer<Self>> {
        Ok(PyClassInitializer::from(validator_for_impl(
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
            email_options,
            http_options,
            keywords,
            vocabularies,
            offline,
        )?)
        .add_subclass(Draft202012Validator {}))
    }
}

#[allow(dead_code)]
mod build {
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}

/// Meta-schema validation
mod meta {
    use super::referencing_error_pyerr;
    use jsonschema::json::Pyo3;
    use pyo3::prelude::*;

    // The bundled meta-schemas are compiled in and read the Python object in place. `None` for a
    // `$schema` outside them, and for every schema on wasm, where they are not built; the caller
    // then uses `meta::is_valid_for` or `meta::validate_for`.
    #[cfg(not(target_family = "wasm"))]
    fn compiled_is_valid(schema: &Bound<'_, PyAny>) -> Option<PyResult<bool>> {
        match jsonschema::meta::pyo3::draft_of(schema.as_borrowed()) {
            jsonschema::Draft::Unknown => None,
            draft => Some(jsonschema::meta::pyo3::is_valid_fn(draft)(schema)),
        }
    }

    #[cfg(target_family = "wasm")]
    fn compiled_is_valid(_: &Bound<'_, PyAny>) -> Option<PyResult<bool>> {
        None
    }

    #[cfg(not(target_family = "wasm"))]
    fn compiled_validate<'i>(
        schema: &'i Bound<'i, PyAny>,
    ) -> Option<PyResult<Result<(), jsonschema::ValidationError<'i>>>> {
        match jsonschema::meta::pyo3::draft_of(schema.as_borrowed()) {
            jsonschema::Draft::Unknown => None,
            draft => Some(jsonschema::meta::pyo3::validate_fn(draft)(schema)),
        }
    }

    #[cfg(target_family = "wasm")]
    fn compiled_validate<'i>(
        _: &'i Bound<'i, PyAny>,
    ) -> Option<PyResult<Result<(), jsonschema::ValidationError<'i>>>> {
        None
    }

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
        let Some(registry) = registry else {
            return crate::surface_pending_errors(schema, false, || {
                if let Some(result) = compiled_is_valid(schema) {
                    return result;
                }
                match jsonschema::meta::is_valid_for::<Pyo3>(schema.as_borrowed()) {
                    Ok(valid) => Ok(valid),
                    Err(error) => {
                        raise_if_unresolvable(py, &error)?;
                        Ok(false)
                    }
                }
            });
        };
        let schema = crate::ser::to_value(schema)?;
        match jsonschema::meta::options()
            .with_registry(registry.inner.as_ref())
            .validate(&schema)
        {
            Ok(()) => Ok(true),
            Err(error) => {
                raise_if_unresolvable(py, &error)?;
                Ok(false)
            }
        }
    }

    fn raise_if_unresolvable(
        py: Python<'_>,
        error: &jsonschema::ValidationError<'_>,
    ) -> PyResult<()> {
        if let jsonschema::error::ValidationErrorKind::Referencing(err) = error.kind() {
            return Err(referencing_error_pyerr(py, err.to_string())?);
        }
        Ok(())
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
    ///     jsonschema_rs.ReferencingError: Unknown meta-schema: 'http://custom.example.com/schema'. Custom meta-schemas must be registered in the registry before use
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
        let Some(registry) = registry else {
            return crate::surface_pending_errors(schema, false, || {
                if let Some(result) = compiled_validate(schema) {
                    return match result? {
                        Ok(()) => Ok(()),
                        Err(error) => Err(crate::into_py_err(py, error, None, None)?),
                    };
                }
                match jsonschema::meta::validate_for::<Pyo3>(schema.as_borrowed()) {
                    Ok(()) => Ok(()),
                    Err(error) => {
                        raise_if_unresolvable(py, &error)?;
                        Err(crate::into_py_err(py, error, None, None)?)
                    }
                }
            });
        };
        let schema = crate::ser::to_value(schema)?;
        match jsonschema::meta::options()
            .with_registry(registry.inner.as_ref())
            .validate(&schema)
        {
            Ok(()) => Ok(()),
            Err(error) => {
                raise_if_unresolvable(py, &error)?;
                Err(crate::into_py_err(py, error, None, None)?)
            }
        }
    }
}

/// JSON Schema validation for Python written in Rust.
#[pymodule]
fn jsonschema_rs(py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    // PyCharm reads a signature from a docstring's first line, so module-level functions start
    // with one.
    types::init(py);
    module.add_wrapped(wrap_pyfunction!(is_valid))?;
    module.add_wrapped(wrap_pyfunction!(validate))?;
    module.add_wrapped(wrap_pyfunction!(iter_errors))?;
    module.add_wrapped(wrap_pyfunction!(evaluate))?;
    module.add_wrapped(wrap_pyfunction!(validator_for))?;
    module.add_wrapped(wrap_pyfunction!(validator_map_for))?;
    module.add_wrapped(wrap_pyfunction!(number_format))?;
    module.add_wrapped(wrap_pyfunction!(known_formats))?;
    module.add_class::<NumberFormat>()?;
    module.add_class::<ValidatorMap>()?;
    module.add_class::<JsonTextValidator>()?;
    module.add_wrapped(wrap_pyfunction!(bundle))?;
    module.add_wrapped(wrap_pyfunction!(dereference))?;
    module.add_wrapped(wrap_pyfunction!(validator_cls_for))?;
    module.add_class::<Validator>()?;
    module.add_class::<Draft4Validator>()?;
    module.add_class::<Draft6Validator>()?;
    module.add_class::<Draft7Validator>()?;
    module.add_class::<Draft201909Validator>()?;
    module.add_class::<Draft202012Validator>()?;
    module.add_class::<PyEvaluation>()?;
    module.add_class::<registry::Registry>()?;
    module.add_class::<registry::Resolver>()?;
    module.add_class::<registry::Resolved>()?;
    module.add_class::<FancyRegexOptions>()?;
    module.add_class::<RegexOptions>()?;
    module.add_class::<EmailOptions>()?;
    #[cfg(not(target_arch = "wasm32"))]
    module.add_class::<HttpOptions>()?;
    module.add("ValidationErrorKind", py.get_type::<ValidationErrorKind>())?;
    module.add("Draft4", DRAFT4)?;
    module.add("Draft6", DRAFT6)?;
    module.add("Draft7", DRAFT7)?;
    module.add("Draft201909", DRAFT201909)?;
    module.add("Draft202012", DRAFT202012)?;

    module.add_class::<canonical::PyCanonicalSchema>()?;
    module.add_wrapped(wrap_pyfunction!(canonical::canonicalize))?;
    canonical::init_module(py, module)?;

    let meta = PyModule::new(py, "meta")?;
    meta.add_function(wrap_pyfunction!(meta::is_valid, &meta)?)?;
    meta.add_function(wrap_pyfunction!(meta::validate, &meta)?)?;
    module.add_submodule(&meta)?;

    // Add build metadata to ease triaging incoming issues
    module.add("__build__", pyo3_built::pyo3_built!(py, build))?;

    Ok(())
}
