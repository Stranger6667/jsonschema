pub(crate) mod json;

use std::hash::{Hash, Hasher};

use jsonschema::canonical::{CanonicalSchema, CanonicalView};

use pyo3::prelude::*;

#[pyclass(frozen, name = "CanonicalSchema")]
pub(crate) struct PyCanonicalSchema {
    inner: CanonicalSchema,
}

#[pymethods]
impl PyCanonicalSchema {
    /// Convert this canonical schema back to a plain Python JSON value.
    fn to_json_schema(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        crate::value_to_python(py, &self.inner.to_json_schema())
    }

    /// The JSON Schema draft as an integer: 4, 6, 7, 19 (2019-09), or 20 (2020-12).
    #[getter]
    fn draft(&self) -> u8 {
        match self.inner.draft() {
            jsonschema::Draft::Draft4 => 4,
            jsonschema::Draft::Draft6 => 6,
            jsonschema::Draft::Draft7 => 7,
            jsonschema::Draft::Draft201909 => 19,
            _ => 20,
        }
    }

    /// Structural kind label of this node.
    #[getter]
    fn kind(&self) -> &'static str {
        self.inner.kind().as_str()
    }

    /// Return the single view object for this node.
    fn view(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        Ok(match self.inner.view() {
            CanonicalView::MultiType(set) => Py::new(
                py,
                MultiTypeView {
                    types: set.iter().map(|ty| ty.to_string()).collect(),
                },
            )?
            .into_any(),
            CanonicalView::TypedGroup(group) => Py::new(
                py,
                TypedGroupView {
                    type_name: group.ty.to_string(),
                    body: Py::new(py, PyCanonicalSchema { inner: group.body })?.into_any(),
                },
            )?
            .into_any(),
            CanonicalView::Const(value) => Py::new(
                py,
                ConstView {
                    value: crate::value_to_python(py, &value)?,
                },
            )?
            .into_any(),
            CanonicalView::Enum(values) => Py::new(
                py,
                EnumView {
                    values: values
                        .iter()
                        .map(|value| crate::value_to_python(py, value))
                        .collect::<PyResult<_>>()?,
                },
            )?
            .into_any(),
            CanonicalView::AnyOf(branches) => Py::new(
                py,
                AnyOfView {
                    branches: branches
                        .into_iter()
                        .map(|branch| {
                            Ok(Py::new(py, PyCanonicalSchema { inner: branch })?.into_any())
                        })
                        .collect::<PyResult<_>>()?,
                },
            )?
            .into_any(),
            CanonicalView::String(view) => Py::new(
                py,
                StringView {
                    min_length: view
                        .min_length
                        .map(|number| {
                            crate::value_to_python(py, &serde_json::Value::Number(number))
                        })
                        .transpose()?,
                    max_length: view
                        .max_length
                        .map(|number| {
                            crate::value_to_python(py, &serde_json::Value::Number(number))
                        })
                        .transpose()?,
                    patterns: view.patterns,
                    formats: view.formats,
                },
            )?
            .into_any(),
            CanonicalView::Number(view) => Py::new(
                py,
                NumberView {
                    minimum: view
                        .minimum
                        .map(|number| {
                            crate::value_to_python(py, &serde_json::Value::Number(number))
                        })
                        .transpose()?,
                    exclusive_minimum: view.exclusive_minimum,
                    maximum: view
                        .maximum
                        .map(|number| {
                            crate::value_to_python(py, &serde_json::Value::Number(number))
                        })
                        .transpose()?,
                    exclusive_maximum: view.exclusive_maximum,
                    multiple_of: divisors_to_python(py, view.multiple_of)?,
                },
            )?
            .into_any(),
            CanonicalView::Integer(view) => Py::new(
                py,
                IntegerView {
                    minimum: view
                        .minimum
                        .map(|number| {
                            crate::value_to_python(py, &serde_json::Value::Number(number))
                        })
                        .transpose()?,
                    maximum: view
                        .maximum
                        .map(|number| {
                            crate::value_to_python(py, &serde_json::Value::Number(number))
                        })
                        .transpose()?,
                    multiple_of: divisors_to_python(py, view.multiple_of)?,
                },
            )?
            .into_any(),
            CanonicalView::Array(view) => Py::new(
                py,
                ArrayView {
                    min_items: view
                        .min_items
                        .map(|number| {
                            crate::value_to_python(py, &serde_json::Value::Number(number))
                        })
                        .transpose()?,
                    max_items: view
                        .max_items
                        .map(|number| {
                            crate::value_to_python(py, &serde_json::Value::Number(number))
                        })
                        .transpose()?,
                    unique_items: view.unique_items,
                    prefix_items: view
                        .prefix_items
                        .into_iter()
                        .map(|schema| Py::new(py, PyCanonicalSchema { inner: schema }))
                        .collect::<PyResult<_>>()?,
                    items: view
                        .items
                        .map(|items| Py::new(py, PyCanonicalSchema { inner: items }))
                        .transpose()?,
                },
            )?
            .into_any(),
            CanonicalView::Object(view) => Py::new(
                py,
                ObjectView {
                    min_properties: view
                        .min_properties
                        .map(|number| {
                            crate::value_to_python(py, &serde_json::Value::Number(number))
                        })
                        .transpose()?,
                    max_properties: view
                        .max_properties
                        .map(|number| {
                            crate::value_to_python(py, &serde_json::Value::Number(number))
                        })
                        .transpose()?,
                    required: view.required,
                    property_names: view
                        .property_names
                        .map(|names| Py::new(py, PyCanonicalSchema { inner: names }))
                        .transpose()?,
                    properties: view
                        .properties
                        .into_iter()
                        .map(|(key, schema)| {
                            Ok((key, Py::new(py, PyCanonicalSchema { inner: schema })?))
                        })
                        .collect::<PyResult<_>>()?,
                    pattern_properties: view
                        .pattern_properties
                        .into_iter()
                        .map(|(pattern, schema)| {
                            Ok((pattern, Py::new(py, PyCanonicalSchema { inner: schema })?))
                        })
                        .collect::<PyResult<_>>()?,
                },
            )?
            .into_any(),
            CanonicalView::True => Py::new(py, TrueView)?.into_any(),
            CanonicalView::False => Py::new(py, FalseView)?.into_any(),
            CanonicalView::Raw(schema) => Py::new(
                py,
                RawView {
                    schema: crate::value_to_python(py, &schema)?,
                },
            )?
            .into_any(),
        })
    }

    /// Map of reference uri -> canonical target for every reference reachable from this schema.
    fn definitions<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
        let dict = pyo3::types::PyDict::new(py);
        for (uri, target) in self.inner.definitions() {
            dict.set_item(uri, PyCanonicalSchema { inner: target })?;
        }
        Ok(dict)
    }

    fn __hash__(&self) -> u64 {
        let mut hasher = ahash::AHasher::default();
        self.inner.hash(&mut hasher);
        hasher.finish()
    }

    /// Return `False` when this schema provably admits no instances.
    fn is_satisfiable(&self) -> bool {
        self.inner.is_satisfiable()
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<PyCanonicalSchema>()
            .is_ok_and(|other| self.inner == other.get().inner)
    }

    fn __repr__(&self) -> String {
        // Bounded: `repr` runs implicitly (REPLs, debuggers, f-strings) and a full
        // `to_json_schema` re-emits the whole document.
        format!(
            "<CanonicalSchema kind={} draft={:?}>",
            self.inner.kind().as_str(),
            self.inner.draft()
        )
    }
}

/// A schema the canonical form does not model structurally, kept verbatim.
#[pyclass(frozen, name = "RawView", module = "jsonschema_rs.canonical")]
pub(crate) struct RawView {
    #[pyo3(get)]
    schema: Py<PyAny>,
}

#[pymethods]
impl RawView {
    #[classattr]
    fn __match_args__() -> (&'static str,) {
        ("schema",)
    }
}

/// Matches any value.
#[pyclass(frozen, name = "TrueView", module = "jsonschema_rs.canonical")]
pub(crate) struct TrueView;

/// Matches no value.
#[pyclass(frozen, name = "FalseView", module = "jsonschema_rs.canonical")]
pub(crate) struct FalseView;

/// A value matches iff its JSON type is in `types`.
#[pyclass(frozen, name = "MultiTypeView", module = "jsonschema_rs.canonical")]
pub(crate) struct MultiTypeView {
    #[pyo3(get)]
    types: Vec<String>,
}

#[pymethods]
impl MultiTypeView {
    #[classattr]
    fn __match_args__() -> (&'static str,) {
        ("types",)
    }
}

/// A value matches iff its JSON type is `type_name` *and* it satisfies `body`.
#[pyclass(frozen, name = "TypedGroupView", module = "jsonschema_rs.canonical")]
pub(crate) struct TypedGroupView {
    #[pyo3(get)]
    type_name: String,
    #[pyo3(get)]
    body: Py<PyAny>,
}

#[pymethods]
impl TypedGroupView {
    #[classattr]
    fn __match_args__() -> (&'static str, &'static str) {
        ("type_name", "body")
    }
}

/// A string value within a length window matching every pattern and format.
#[pyclass(frozen, name = "StringView", module = "jsonschema_rs.canonical")]
pub(crate) struct StringView {
    #[pyo3(get)]
    min_length: Option<Py<PyAny>>,
    #[pyo3(get)]
    max_length: Option<Py<PyAny>>,
    #[pyo3(get)]
    patterns: Vec<String>,
    #[pyo3(get)]
    formats: Vec<String>,
}

#[pymethods]
impl StringView {
    #[classattr]
    fn __match_args__() -> (&'static str, &'static str, &'static str, &'static str) {
        ("min_length", "max_length", "patterns", "formats")
    }
}

/// An array value's constraints.
// The fields carry the keywords they came from, whose names share the suffix.
#[allow(clippy::struct_field_names)]
#[pyclass(frozen, name = "ArrayView", module = "jsonschema_rs.canonical")]
pub(crate) struct ArrayView {
    #[pyo3(get)]
    min_items: Option<Py<PyAny>>,
    #[pyo3(get)]
    max_items: Option<Py<PyAny>>,
    #[pyo3(get)]
    unique_items: bool,
    #[pyo3(get)]
    prefix_items: Vec<Py<PyCanonicalSchema>>,
    #[pyo3(get)]
    items: Option<Py<PyCanonicalSchema>>,
}

#[pymethods]
impl ArrayView {
    #[classattr]
    fn __match_args__() -> (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
    ) {
        (
            "min_items",
            "max_items",
            "unique_items",
            "prefix_items",
            "items",
        )
    }
}

/// An object value's constraints.
#[pyclass(frozen, name = "ObjectView", module = "jsonschema_rs.canonical")]
pub(crate) struct ObjectView {
    #[pyo3(get)]
    min_properties: Option<Py<PyAny>>,
    #[pyo3(get)]
    max_properties: Option<Py<PyAny>>,
    #[pyo3(get)]
    required: Vec<String>,
    #[pyo3(get)]
    property_names: Option<Py<PyCanonicalSchema>>,
    #[pyo3(get)]
    properties: std::collections::BTreeMap<String, Py<PyCanonicalSchema>>,
    #[pyo3(get)]
    pattern_properties: std::collections::BTreeMap<String, Py<PyCanonicalSchema>>,
}

#[pymethods]
impl ObjectView {
    #[classattr]
    fn __match_args__() -> (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
    ) {
        (
            "min_properties",
            "max_properties",
            "required",
            "property_names",
            "properties",
            "pattern_properties",
        )
    }
}

/// A number value within a real interval.
#[pyclass(frozen, name = "NumberView", module = "jsonschema_rs.canonical")]
pub(crate) struct NumberView {
    #[pyo3(get)]
    minimum: Option<Py<PyAny>>,
    #[pyo3(get)]
    exclusive_minimum: bool,
    #[pyo3(get)]
    maximum: Option<Py<PyAny>>,
    #[pyo3(get)]
    exclusive_maximum: bool,
    #[pyo3(get)]
    multiple_of: Vec<Py<PyAny>>,
}

#[pymethods]
impl NumberView {
    #[classattr]
    fn __match_args__() -> (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
    ) {
        (
            "minimum",
            "exclusive_minimum",
            "maximum",
            "exclusive_maximum",
            "multiple_of",
        )
    }
}

/// An integer value within a range, optionally a multiple of a divisor.
#[pyclass(frozen, name = "IntegerView", module = "jsonschema_rs.canonical")]
pub(crate) struct IntegerView {
    #[pyo3(get)]
    minimum: Option<Py<PyAny>>,
    #[pyo3(get)]
    maximum: Option<Py<PyAny>>,
    #[pyo3(get)]
    multiple_of: Vec<Py<PyAny>>,
}

#[pymethods]
impl IntegerView {
    #[classattr]
    fn __match_args__() -> (&'static str, &'static str, &'static str) {
        ("minimum", "maximum", "multiple_of")
    }
}

/// A value matches iff at least one branch matches.
#[pyclass(frozen, name = "AnyOfView", module = "jsonschema_rs.canonical")]
pub(crate) struct AnyOfView {
    #[pyo3(get)]
    branches: Vec<Py<PyAny>>,
}

#[pymethods]
impl AnyOfView {
    #[classattr]
    fn __match_args__() -> (&'static str,) {
        ("branches",)
    }
}

/// Exactly one admitted value.
#[pyclass(frozen, name = "ConstView", module = "jsonschema_rs.canonical")]
pub(crate) struct ConstView {
    #[pyo3(get)]
    value: Py<PyAny>,
}

#[pymethods]
impl ConstView {
    #[classattr]
    fn __match_args__() -> (&'static str,) {
        ("value",)
    }
}

/// A sorted, deduplicated finite set of admitted values.
#[pyclass(frozen, name = "EnumView", module = "jsonschema_rs.canonical")]
pub(crate) struct EnumView {
    #[pyo3(get)]
    values: Vec<Py<PyAny>>,
}

#[pymethods]
impl EnumView {
    #[classattr]
    fn __match_args__() -> (&'static str,) {
        ("values",)
    }
}

/// canonicalize(schema, /, *, draft=None, validate_formats=None)
///
/// Parse and normalize a JSON Schema to its canonical form.
///
/// Returns a :class:`CanonicalSchema` that is semantically equivalent to the input.
#[pyfunction]
#[pyo3(signature = (schema, *, draft=None, validate_formats=None))]
pub(crate) fn canonicalize(
    schema: &Bound<'_, PyAny>,
    draft: Option<u8>,
    validate_formats: Option<bool>,
) -> PyResult<PyCanonicalSchema> {
    let schema_value = crate::ser::to_value(schema)?;
    let mut options = jsonschema::canonical::options();
    if let Some(draft) = draft {
        options = options.with_draft(crate::get_draft(draft)?);
    }
    if let Some(validate_formats) = validate_formats {
        options = options.should_validate_formats(validate_formats);
    }
    options
        .canonicalize(&schema_value)
        .map(|inner| PyCanonicalSchema { inner })
        .map_err(|error| canonicalization_error(schema.py(), error))
}

/// Meta-validation failures surface as structured `ValidationError`; everything else maps to the
/// python-side `CanonicalizationError` subclass named after the variant.
fn canonicalization_error(
    py: Python<'_>,
    error: jsonschema::canonical::CanonicalizationError,
) -> PyErr {
    use jsonschema::canonical::CanonicalizationError;
    if let CanonicalizationError::ValidationError(validation) = error {
        return crate::into_py_err(py, validation, None).unwrap_or_else(|err| err);
    }
    let name = match &error {
        CanonicalizationError::InvalidSchemaType(_) => "InvalidSchemaType",
        CanonicalizationError::InvalidPattern { .. } => "InvalidPattern",
        _ => "CanonicalizationError",
    };
    let built = py.import("jsonschema_rs").and_then(|module| {
        module
            .getattr("canonical")?
            .getattr(name)?
            .call1((error.to_string(),))
    });
    match built {
        Ok(object) => PyErr::from_value(object),
        Err(err) => err,
    }
}

pub(crate) fn init_module(py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    let canonical_module = PyModule::new(py, "canonical")?;

    canonical_module.add_class::<TrueView>()?;
    canonical_module.add_class::<FalseView>()?;
    canonical_module.add_class::<MultiTypeView>()?;
    canonical_module.add_class::<TypedGroupView>()?;
    canonical_module.add_class::<StringView>()?;
    canonical_module.add_class::<IntegerView>()?;
    canonical_module.add_class::<ArrayView>()?;
    canonical_module.add_class::<ObjectView>()?;
    canonical_module.add_class::<NumberView>()?;
    canonical_module.add_class::<AnyOfView>()?;
    canonical_module.add_class::<ConstView>()?;
    canonical_module.add_class::<EnumView>()?;
    canonical_module.add_class::<RawView>()?;

    let canonical_json_module = PyModule::new(py, "json")?;
    canonical_json_module.add_function(pyo3::wrap_pyfunction!(
        json::canonical_json_to_string,
        &canonical_json_module
    )?)?;
    canonical_module.add_submodule(&canonical_json_module)?;

    let canonical_schema_module = PyModule::new(py, "schema")?;
    canonical_schema_module.add_function(pyo3::wrap_pyfunction!(
        crate::clone::canonical_schema_clone,
        &canonical_schema_module
    )?)?;
    canonical_module.add_submodule(&canonical_schema_module)?;

    module.add_submodule(&canonical_module)?;
    Ok(())
}

fn divisors_to_python(
    py: Python<'_>,
    divisors: Vec<serde_json::Number>,
) -> PyResult<Vec<Py<PyAny>>> {
    divisors
        .into_iter()
        .map(|number| crate::value_to_python(py, &serde_json::Value::Number(number)))
        .collect()
}
