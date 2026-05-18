mod json;
pub(crate) mod view;

use std::hash::{Hash, Hasher};

use jsonschema::{
    canonical::{CanonicalSchema, CanonicalizationError},
    Draft,
};
use pyo3::{prelude::*, types::PyDict};

use crate::{
    canonical_value_to_python, get_draft,
    regex::{extract_pattern_options, PyPatternOptions},
    registry,
    retriever::{into_retriever, Retriever},
    ser,
};

#[pyclass(frozen, name = "CanonicalSchema")]
pub(crate) struct PyCanonicalSchema {
    pub(crate) inner: CanonicalSchema,
}

#[pymethods]
impl PyCanonicalSchema {
    /// Convert this canonical schema to a plain Python JSON value.
    fn to_json_schema(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        canonical_value_to_python(py, &self.inner.to_json_schema())
    }

    /// Return a new schema that validates iff both this schema and `other` validate.
    fn intersect(&self, other: &PyCanonicalSchema) -> PyCanonicalSchema {
        PyCanonicalSchema {
            inner: self.inner.intersect(&other.inner),
        }
    }

    /// Return a new schema that validates iff either this schema or `other` validates.
    fn union(&self, other: &PyCanonicalSchema) -> PyCanonicalSchema {
        PyCanonicalSchema {
            inner: self.inner.union(&other.inner),
        }
    }

    /// Return a new schema that validates iff this schema does not.
    fn negate(&self) -> PyCanonicalSchema {
        PyCanonicalSchema {
            inner: self.inner.negate(),
        }
    }

    /// Return a schema validating iff `self` validates but `other` does not (`self \ other`).
    fn subtract(&self, other: &PyCanonicalSchema) -> PyCanonicalSchema {
        PyCanonicalSchema {
            inner: self.inner.subtract(&other.inner),
        }
    }

    /// Return whether every value satisfying `self` also satisfies `other` (`self ⊆ other`).
    ///
    /// `True` = proven; `None` = inconclusive.
    fn is_subschema_of(&self, other: &PyCanonicalSchema) -> Option<bool> {
        self.inner.is_subschema_of(&other.inner)
    }

    /// Return False when this schema canonicalized to `false` or provably admits no instances.
    fn is_satisfiable(&self) -> bool {
        self.inner.is_satisfiable()
    }

    /// The JSON Schema draft as an integer: 4, 6, 7, 19 (2019-09), or 20 (2020-12).
    #[getter]
    fn draft(&self) -> u8 {
        draft_version_number(self.inner.draft())
    }

    fn __hash__(&self) -> u64 {
        let mut hasher = ahash::AHasher::default();
        self.inner.hash(&mut hasher);
        hasher.finish()
    }

    fn __eq__(&self, other: &PyCanonicalSchema) -> bool {
        self.inner == other.inner
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

    /// Structural kind label of this node.
    #[getter]
    fn kind(&self) -> &'static str {
        self.inner.kind().as_str()
    }

    /// Return the single view object for this node.
    fn view(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        view::view_to_py(py, self.inner.view())
    }

    /// Map of reference uri -> canonical target for every symbolic reference reachable from this schema.
    ///
    /// `ReferenceView.uri` and `RecursiveView.uri` are keys in this map; a uri that is absent is dangling/unresolvable.
    /// Values are :class:`CanonicalSchema` objects that may themselves contain further references into the same map.
    fn definitions(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let dict = PyDict::new(py);
        for (uri, target) in self.inner.definitions() {
            dict.set_item(uri, Py::new(py, PyCanonicalSchema { inner: target })?)?;
        }
        Ok(dict.into_any().unbind())
    }
}

fn draft_version_number(draft: Draft) -> u8 {
    match draft {
        Draft::Draft4 => 4,
        Draft::Draft6 => 6,
        Draft::Draft7 => 7,
        Draft::Draft201909 => 19,
        _ => 20,
    }
}

/// canonicalize(schema, /, *, draft=None, validate_formats=None, retriever=None, registry=None, base_uri=None, pattern_options=None, inline_budget=None)
///
/// Parse and normalize a JSON Schema to its canonical form.
///
/// Returns a :class:`CanonicalSchema` that is semantically equivalent to the input but in a stable, reduced representation.
///
/// Parameters
/// ----------
/// schema:
///     The JSON Schema document (a Python dict or bool).
/// draft:
///     Override the draft version (4, 6, 7, 19, or 20).  When omitted the
///     draft is detected from the ``$schema`` keyword, defaulting to Draft
///     2020-12 if it is absent.
/// validate_formats:
///     Override whether ``format`` keywords are treated as assertions during
///     canonicalization. When omitted, the selected draft's validator default is used.
/// retriever:
///     A callable ``(uri: str) -> JsonValue`` for resolving external ``$ref`` URIs.
/// registry:
///     A pre-built :class:`Registry` for external ``$ref`` resolution.
/// base_uri:
///     Optional base URI for resolving relative ``$ref`` targets.
/// pattern_options:
///     A :class:`RegexOptions` or :class:`FancyRegexOptions` selecting the
///     engine used to compile ``pattern`` keywords.
/// inline_budget:
///     Cap on how much a resolvable acyclic ``$ref`` may inline (canonical
///     node-count). Targets larger than the budget are emitted symbolically as a
///     :class:`ReferenceView` resolvable via :meth:`CanonicalSchema.definitions`.
///     Defaults to unbounded (inline everything); pass ``0`` for fully symbolic
///     references. Cyclic refs are always symbolic.
#[pyfunction]
#[pyo3(signature = (schema, *, draft=None, validate_formats=None, retriever=None, registry=None, base_uri=None, pattern_options=None, inline_budget=None))]
pub(crate) fn canonicalize<'py>(
    schema: &Bound<'py, PyAny>,
    draft: Option<u8>,
    validate_formats: Option<bool>,
    retriever: Option<&Bound<'py, PyAny>>,
    registry: Option<&registry::Registry>,
    base_uri: Option<String>,
    pattern_options: Option<&Bound<'py, PyAny>>,
    inline_budget: Option<usize>,
) -> PyResult<PyCanonicalSchema> {
    let schema_value = ser::to_value(schema)?;
    let mut options = jsonschema::canonical::options();
    if let Some(budget) = inline_budget {
        options = options.with_inline_budget(budget);
    }
    if let Some(d) = draft {
        options = options.with_draft(get_draft(d)?);
    }
    if let Some(validate_formats) = validate_formats {
        options = options.should_validate_formats(validate_formats);
    }
    let retriever_was_provided = retriever.is_some();
    if let Some(r) = retriever {
        let func = into_retriever(r)?;
        options = options.with_retriever(Retriever { func });
    }
    if let Some(reg) = registry {
        if !retriever_was_provided {
            if let Some(registry_retriever) = reg.retriever() {
                let func = Python::attach(|py| into_retriever(registry_retriever.bind(py)))?;
                options = options.with_retriever(Retriever { func });
            }
        }
        options = options.with_registry(reg.inner.as_ref());
    }
    if let Some(uri) = base_uri {
        options = options.with_base_uri(uri);
    }
    if let Some(pattern_options) = pattern_options {
        match extract_pattern_options(pattern_options)? {
            PyPatternOptions::Fancy(p) => options = options.with_pattern_options(&p),
            PyPatternOptions::Regex(p) => options = options.with_pattern_options(&p),
        }
    }
    options
        .canonicalize(&schema_value)
        .map(|inner| PyCanonicalSchema { inner })
        .map_err(|error| canonicalization_error(schema.py(), error))
}

/// Map each canonicalization failure to a dedicated Python exception: a structured `ValidationError` for
/// meta-validation, otherwise a `CanonicalizationError` subclass (`InvalidSchemaType` /
/// `InvalidJsonValue` / `InvalidPattern` / `UnguardedRecursion` / `InfiniteRecursion`).
fn canonicalization_error(py: Python<'_>, error: CanonicalizationError) -> PyErr {
    if let CanonicalizationError::ValidationError(validation) = error {
        return crate::into_py_err(py, validation, None).unwrap_or_else(|err| err);
    }
    let message = error.to_string();
    let (name, location) = match &error {
        CanonicalizationError::InvalidSchemaType(_) => ("InvalidSchemaType", None),
        CanonicalizationError::InvalidJsonValue(_) => ("InvalidJsonValue", None),
        CanonicalizationError::UnguardedRecursion(_) => ("UnguardedRecursion", None),
        CanonicalizationError::InfiniteRecursion(_) => ("InfiniteRecursion", None),
        CanonicalizationError::InvalidPattern { pointer, .. } => {
            ("InvalidPattern", Some(pointer.clone()))
        }
        // `ValidationError` returns above; future variants fall back to the base canonical error.
        _ => ("CanonicalizationError", None),
    };
    build_canonical_error(py, name, message, location)
}

fn build_canonical_error(
    py: Python<'_>,
    name: &str,
    message: String,
    location: Option<String>,
) -> PyErr {
    let built = py.import("jsonschema_rs").and_then(|module| {
        let class = module.getattr("canonical")?.getattr(name)?;
        match location {
            Some(location) => class.call1((message, location)),
            None => class.call1((message,)),
        }
    });
    match built {
        Ok(object) => PyErr::from_value(object),
        Err(err) => err,
    }
}

pub(crate) fn init_module(py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    let canonical = PyModule::new(py, "canonical")?;

    canonical.add_class::<view::NullView>()?;
    canonical.add_class::<view::TrueView>()?;
    canonical.add_class::<view::FalseView>()?;
    canonical.add_class::<view::BooleanViewPy>()?;
    canonical.add_class::<view::IntegerViewPy>()?;
    canonical.add_class::<view::NumberViewPy>()?;
    canonical.add_class::<view::StringViewPy>()?;
    canonical.add_class::<view::ContentFacetViewPy>()?;
    canonical.add_class::<view::ContainsViewPy>()?;
    canonical.add_class::<view::ArrayViewPy>()?;
    canonical.add_class::<view::ObjectViewPy>()?;
    canonical.add_class::<view::RequiredPropertyPy>()?;
    canonical.add_class::<view::PatternPropertyRequirementPy>()?;
    canonical.add_class::<view::AdditionalPropertiesRequirementPy>()?;
    canonical.add_class::<view::DependentPropertiesRequirementPy>()?;
    canonical.add_class::<view::DependentSchemaRequirementPy>()?;
    canonical.add_class::<view::NamedPropertyConstraintPy>()?;
    canonical.add_class::<view::PatternPropertyConstraintPy>()?;
    canonical.add_class::<view::AdditionalPropertiesConstraintPy>()?;
    canonical.add_class::<view::MultiTypeViewPy>()?;
    canonical.add_class::<view::AllOfViewPy>()?;
    canonical.add_class::<view::AnyOfViewPy>()?;
    canonical.add_class::<view::OneOfViewPy>()?;
    canonical.add_class::<view::NotViewPy>()?;
    canonical.add_class::<view::TypedGroupViewPy>()?;
    canonical.add_class::<view::TypeGuardViewPy>()?;
    canonical.add_class::<view::ConstViewPy>()?;
    canonical.add_class::<view::EnumViewPy>()?;
    canonical.add_class::<view::ReferenceViewPy>()?;
    canonical.add_class::<view::RecursiveViewPy>()?;
    canonical.add_class::<view::DynamicRefViewPy>()?;
    canonical.add_class::<view::RawViewPy>()?;

    let json_module = PyModule::new(py, "json")?;
    json_module.add_function(pyo3::wrap_pyfunction!(
        json::canonical_json_to_string,
        &json_module
    )?)?;
    canonical.add_submodule(&json_module)?;

    let schema_module = PyModule::new(py, "schema")?;
    schema_module.add_function(pyo3::wrap_pyfunction!(
        crate::clone::canonical_schema_clone,
        &schema_module
    )?)?;
    canonical.add_submodule(&schema_module)?;

    module.add_submodule(&canonical)?;
    Ok(())
}
