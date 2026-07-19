pub(crate) mod json;

use std::hash::{Hash, Hasher};

use pyo3::prelude::*;

#[pyclass(frozen, name = "CanonicalSchema")]
pub(crate) struct PyCanonicalSchema {
    inner: jsonschema::canonical::CanonicalSchema,
}

fn kind_label(kind: jsonschema::canonical::CanonicalKind) -> &'static str {
    match kind {
        jsonschema::canonical::CanonicalKind::Raw => "raw",
    }
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
        kind_label(self.inner.kind())
    }

    /// Return the single view object for this node.
    fn view(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match self.inner.view() {
            jsonschema::canonical::CanonicalView::Raw(schema) => Ok(Py::new(
                py,
                RawView {
                    schema: crate::value_to_python(py, &schema)?,
                },
            )?
            .into_any()),
            other => unreachable!("unsupported canonical view: {other:?}"),
        }
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
            kind_label(self.inner.kind()),
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
