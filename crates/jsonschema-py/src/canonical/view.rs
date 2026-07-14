use jsonschema::{
    canonical::{
        ArrayView, BooleanVariant, CanonicalSchema, CanonicalView, ContainsView, NumericView,
        ObjectConstraintView, ObjectRequirementView, ObjectView, StringView,
    },
    JsonType,
};
use pyo3::prelude::*;
use serde_json::{Number, Value};

use crate::{canonical::PyCanonicalSchema, canonical_value_to_python};

fn wrap(inner: CanonicalSchema) -> PyCanonicalSchema {
    PyCanonicalSchema { inner }
}

fn number_to_py(py: Python<'_>, number: Option<Number>) -> PyResult<Option<Py<PyAny>>> {
    match number {
        Some(number) => Ok(Some(canonical_value_to_python(py, &Value::Number(number))?)),
        None => Ok(None),
    }
}

fn required_number_to_py(py: Python<'_>, number: Number) -> PyResult<Py<PyAny>> {
    canonical_value_to_python(py, &Value::Number(number))
}

fn numbers_to_py(py: Python<'_>, numbers: Vec<Number>) -> PyResult<Vec<Py<PyAny>>> {
    numbers
        .into_iter()
        .map(|number| canonical_value_to_python(py, &Value::Number(number)))
        .collect()
}

#[pyclass(frozen, name = "NullView")]
pub(crate) struct NullView;

#[pyclass(frozen, name = "TrueView")]
pub(crate) struct TrueView;

#[pyclass(frozen, name = "FalseView")]
pub(crate) struct FalseView;

#[pyclass(frozen, name = "BooleanView")]
pub(crate) struct BooleanViewPy {
    #[pyo3(get)]
    variant: &'static str,
}

#[pymethods]
impl BooleanViewPy {
    #[classattr]
    fn __match_args__() -> (&'static str,) {
        ("variant",)
    }
}

#[pyclass(frozen, name = "IntegerView")]
pub(crate) struct IntegerViewPy {
    #[pyo3(get)]
    minimum: Option<Py<PyAny>>,
    #[pyo3(get)]
    maximum: Option<Py<PyAny>>,
    #[pyo3(get)]
    exclusive_minimum: Option<Py<PyAny>>,
    #[pyo3(get)]
    exclusive_maximum: Option<Py<PyAny>>,
    #[pyo3(get)]
    multiple_of: Option<Py<PyAny>>,
    #[pyo3(get)]
    not_multiple_of: Vec<Py<PyAny>>,
}

#[pymethods]
impl IntegerViewPy {
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
            "minimum",
            "maximum",
            "exclusive_minimum",
            "exclusive_maximum",
            "multiple_of",
            "not_multiple_of",
        )
    }
}

#[pyclass(frozen, name = "NumberView")]
pub(crate) struct NumberViewPy {
    #[pyo3(get)]
    minimum: Option<Py<PyAny>>,
    #[pyo3(get)]
    maximum: Option<Py<PyAny>>,
    #[pyo3(get)]
    exclusive_minimum: Option<Py<PyAny>>,
    #[pyo3(get)]
    exclusive_maximum: Option<Py<PyAny>>,
    #[pyo3(get)]
    multiple_of: Option<Py<PyAny>>,
    #[pyo3(get)]
    not_multiple_of: Vec<Py<PyAny>>,
}

#[pymethods]
impl NumberViewPy {
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
            "minimum",
            "maximum",
            "exclusive_minimum",
            "exclusive_maximum",
            "multiple_of",
            "not_multiple_of",
        )
    }
}

#[pyclass(frozen, name = "StringView")]
pub(crate) struct StringViewPy {
    #[pyo3(get)]
    min_length: Option<Py<PyAny>>,
    #[pyo3(get)]
    max_length: Option<Py<PyAny>>,
    #[pyo3(get)]
    patterns: Vec<String>,
    #[pyo3(get)]
    not_patterns: Vec<String>,
    #[pyo3(get)]
    format: Option<String>,
    #[pyo3(get)]
    content: Vec<Py<PyAny>>,
    #[pyo3(get)]
    extended_regex: bool,
}

#[pymethods]
impl StringViewPy {
    #[classattr]
    fn __match_args__() -> (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
    ) {
        (
            "min_length",
            "max_length",
            "patterns",
            "not_patterns",
            "format",
            "content",
            "extended_regex",
        )
    }
}

type NumericFields = (
    Option<Py<PyAny>>,
    Option<Py<PyAny>>,
    Option<Py<PyAny>>,
    Option<Py<PyAny>>,
    Option<Py<PyAny>>,
    Vec<Py<PyAny>>,
);

fn numeric_to_py(py: Python<'_>, view: NumericView) -> PyResult<NumericFields> {
    Ok((
        number_to_py(py, view.minimum)?,
        number_to_py(py, view.maximum)?,
        number_to_py(py, view.exclusive_minimum)?,
        number_to_py(py, view.exclusive_maximum)?,
        number_to_py(py, view.multiple_of)?,
        numbers_to_py(py, view.not_multiple_of)?,
    ))
}

fn string_to_py(py: Python<'_>, view: StringView) -> PyResult<StringViewPy> {
    let content = view
        .content
        .into_iter()
        .map(|facet| content_facet_to_py(py, facet))
        .collect::<PyResult<Vec<_>>>()?;
    Ok(StringViewPy {
        min_length: number_to_py(py, view.min_length)?,
        max_length: number_to_py(py, view.max_length)?,
        patterns: view.patterns,
        not_patterns: view.not_patterns,
        format: view.format,
        content,
        extended_regex: view.extended_regex,
    })
}

// Field names mirror the JSON Schema keywords (`contentEncoding`, etc.).
#[allow(clippy::struct_field_names)]
#[pyclass(frozen, name = "ContentFacetView")]
pub(crate) struct ContentFacetViewPy {
    #[pyo3(get)]
    content_encoding: Option<String>,
    #[pyo3(get)]
    content_media_type: Option<String>,
    #[pyo3(get)]
    content_schema: Option<Py<PyAny>>,
}

#[pymethods]
impl ContentFacetViewPy {
    #[classattr]
    fn __match_args__() -> (&'static str, &'static str, &'static str) {
        ("content_encoding", "content_media_type", "content_schema")
    }
}

fn content_facet_to_py(
    py: Python<'_>,
    facet: jsonschema::canonical::ContentFacetView,
) -> PyResult<Py<PyAny>> {
    let content_schema = match facet.content_schema {
        Some(value) => Some(canonical_value_to_python(py, &value)?),
        None => None,
    };
    Ok(Py::new(
        py,
        ContentFacetViewPy {
            content_encoding: facet.content_encoding,
            content_media_type: facet.content_media_type,
            content_schema,
        },
    )?
    .into_any())
}

#[pyclass(frozen, name = "ContainsView")]
pub(crate) struct ContainsViewPy {
    #[pyo3(get)]
    schema: Py<PyAny>,
    #[pyo3(get)]
    min_contains: Py<PyAny>,
    #[pyo3(get)]
    max_contains: Option<Py<PyAny>>,
}

#[pymethods]
impl ContainsViewPy {
    #[classattr]
    fn __match_args__() -> (&'static str, &'static str, &'static str) {
        ("schema", "min_contains", "max_contains")
    }
}

#[pyclass(frozen, name = "ArrayView")]
pub(crate) struct ArrayViewPy {
    #[pyo3(get)]
    prefix: Vec<Py<PyAny>>,
    #[pyo3(get)]
    tail: Option<Py<PyAny>>,
    #[pyo3(get)]
    min_items: Py<PyAny>,
    #[pyo3(get)]
    max_items: Option<Py<PyAny>>,
    #[pyo3(get)]
    unique_items: bool,
    #[pyo3(get)]
    repeated_items: bool,
    #[pyo3(get)]
    contains: Vec<Py<PyAny>>,
}

#[pymethods]
impl ArrayViewPy {
    #[classattr]
    fn __match_args__() -> (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
    ) {
        (
            "prefix",
            "tail",
            "min_items",
            "max_items",
            "unique_items",
            "repeated_items",
            "contains",
        )
    }
}

#[pyclass(frozen, name = "RequiredProperty")]
pub(crate) struct RequiredPropertyPy {
    #[pyo3(get)]
    name: String,
}

#[pymethods]
impl RequiredPropertyPy {
    #[classattr]
    fn __match_args__() -> (&'static str,) {
        ("name",)
    }
}

#[pyclass(frozen, name = "PatternPropertyRequirement")]
pub(crate) struct PatternPropertyRequirementPy {
    #[pyo3(get)]
    pattern: String,
    #[pyo3(get)]
    schema: Py<PyAny>,
}

#[pymethods]
impl PatternPropertyRequirementPy {
    #[classattr]
    fn __match_args__() -> (&'static str, &'static str) {
        ("pattern", "schema")
    }
}

#[pyclass(frozen, name = "AdditionalPropertiesRequirement")]
pub(crate) struct AdditionalPropertiesRequirementPy {
    #[pyo3(get)]
    schema: Py<PyAny>,
}

#[pymethods]
impl AdditionalPropertiesRequirementPy {
    #[classattr]
    fn __match_args__() -> (&'static str,) {
        ("schema",)
    }
}

#[pyclass(frozen, name = "DependentPropertiesRequirement")]
pub(crate) struct DependentPropertiesRequirementPy {
    #[pyo3(get)]
    property: String,
    #[pyo3(get)]
    required_properties: Vec<String>,
}

#[pymethods]
impl DependentPropertiesRequirementPy {
    #[classattr]
    fn __match_args__() -> (&'static str, &'static str) {
        ("property", "required_properties")
    }
}

#[pyclass(frozen, name = "DependentSchemaRequirement")]
pub(crate) struct DependentSchemaRequirementPy {
    #[pyo3(get)]
    property: String,
    #[pyo3(get)]
    schema: Py<PyAny>,
}

#[pymethods]
impl DependentSchemaRequirementPy {
    #[classattr]
    fn __match_args__() -> (&'static str, &'static str) {
        ("property", "schema")
    }
}

#[pyclass(frozen, name = "NamedPropertyConstraint")]
pub(crate) struct NamedPropertyConstraintPy {
    #[pyo3(get)]
    name: String,
    #[pyo3(get)]
    schema: Py<PyAny>,
}

#[pymethods]
impl NamedPropertyConstraintPy {
    #[classattr]
    fn __match_args__() -> (&'static str, &'static str) {
        ("name", "schema")
    }
}

#[pyclass(frozen, name = "PatternPropertyConstraint")]
pub(crate) struct PatternPropertyConstraintPy {
    #[pyo3(get)]
    pattern: String,
    #[pyo3(get)]
    schema: Py<PyAny>,
}

#[pymethods]
impl PatternPropertyConstraintPy {
    #[classattr]
    fn __match_args__() -> (&'static str, &'static str) {
        ("pattern", "schema")
    }
}

#[pyclass(frozen, name = "AdditionalPropertiesConstraint")]
pub(crate) struct AdditionalPropertiesConstraintPy {
    #[pyo3(get)]
    schema: Py<PyAny>,
}

#[pymethods]
impl AdditionalPropertiesConstraintPy {
    #[classattr]
    fn __match_args__() -> (&'static str,) {
        ("schema",)
    }
}

#[pyclass(frozen, name = "ObjectView")]
pub(crate) struct ObjectViewPy {
    #[pyo3(get)]
    requirements: Vec<Py<PyAny>>,
    #[pyo3(get)]
    constraints: Vec<Py<PyAny>>,
    #[pyo3(get)]
    property_names: Option<Py<PyAny>>,
    #[pyo3(get)]
    min_properties: Option<Py<PyAny>>,
    #[pyo3(get)]
    max_properties: Option<Py<PyAny>>,
}

#[pymethods]
impl ObjectViewPy {
    #[classattr]
    fn __match_args__() -> (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
    ) {
        (
            "requirements",
            "constraints",
            "property_names",
            "min_properties",
            "max_properties",
        )
    }
}

fn schema_to_py(py: Python<'_>, schema: CanonicalSchema) -> PyResult<Py<PyAny>> {
    Ok(Py::new(py, wrap(schema))?.into_any())
}

fn array_to_py(py: Python<'_>, view: ArrayView) -> PyResult<ArrayViewPy> {
    let prefix = view
        .prefix
        .into_iter()
        .map(|c| schema_to_py(py, c))
        .collect::<PyResult<Vec<_>>>()?;
    let tail = match view.tail {
        Some(c) => Some(schema_to_py(py, c)?),
        None => None,
    };
    let contains = view
        .contains
        .into_iter()
        .map(|c| contains_to_py(py, c))
        .collect::<PyResult<Vec<_>>>()?;
    Ok(ArrayViewPy {
        prefix,
        tail,
        min_items: required_number_to_py(py, view.min_items)?,
        max_items: number_to_py(py, view.max_items)?,
        unique_items: view.unique_items,
        repeated_items: view.repeated_items,
        contains,
    })
}

fn contains_to_py(py: Python<'_>, view: ContainsView) -> PyResult<Py<PyAny>> {
    Ok(Py::new(
        py,
        ContainsViewPy {
            schema: schema_to_py(py, view.schema)?,
            min_contains: required_number_to_py(py, view.min_contains)?,
            max_contains: number_to_py(py, view.max_contains)?,
        },
    )?
    .into_any())
}

fn requirement_to_py(py: Python<'_>, requirement: ObjectRequirementView) -> PyResult<Py<PyAny>> {
    let object = match requirement {
        ObjectRequirementView::RequiredProperty { name } => {
            Py::new(py, RequiredPropertyPy { name })?.into_any()
        }
        ObjectRequirementView::PatternPropertyRequirement { pattern, schema } => Py::new(
            py,
            PatternPropertyRequirementPy {
                pattern,
                schema: schema_to_py(py, schema)?,
            },
        )?
        .into_any(),
        ObjectRequirementView::AdditionalPropertiesRequirement { schema } => Py::new(
            py,
            AdditionalPropertiesRequirementPy {
                schema: schema_to_py(py, schema)?,
            },
        )?
        .into_any(),
        ObjectRequirementView::DependentPropertiesRequirement {
            property,
            required_properties,
        } => Py::new(
            py,
            DependentPropertiesRequirementPy {
                property,
                required_properties,
            },
        )?
        .into_any(),
        ObjectRequirementView::DependentSchemaRequirement { property, schema } => Py::new(
            py,
            DependentSchemaRequirementPy {
                property,
                schema: schema_to_py(py, schema)?,
            },
        )?
        .into_any(),
    };
    Ok(object)
}

fn constraint_to_py(py: Python<'_>, constraint: ObjectConstraintView) -> PyResult<Py<PyAny>> {
    let object = match constraint {
        ObjectConstraintView::NamedProperty { name, schema } => Py::new(
            py,
            NamedPropertyConstraintPy {
                name,
                schema: schema_to_py(py, schema)?,
            },
        )?
        .into_any(),
        ObjectConstraintView::PatternProperty { pattern, schema } => Py::new(
            py,
            PatternPropertyConstraintPy {
                pattern,
                schema: schema_to_py(py, schema)?,
            },
        )?
        .into_any(),
        ObjectConstraintView::AdditionalProperties { schema } => Py::new(
            py,
            AdditionalPropertiesConstraintPy {
                schema: schema_to_py(py, schema)?,
            },
        )?
        .into_any(),
    };
    Ok(object)
}

#[pyclass(frozen, name = "MultiTypeView")]
pub(crate) struct MultiTypeViewPy {
    #[pyo3(get)]
    types: Vec<String>,
}

#[pymethods]
impl MultiTypeViewPy {
    #[classattr]
    fn __match_args__() -> (&'static str,) {
        ("types",)
    }
}

#[pyclass(frozen, name = "AllOfView")]
pub(crate) struct AllOfViewPy {
    #[pyo3(get)]
    schemas: Vec<Py<PyAny>>,
}

#[pymethods]
impl AllOfViewPy {
    #[classattr]
    fn __match_args__() -> (&'static str,) {
        ("schemas",)
    }
}

#[pyclass(frozen, name = "AnyOfView")]
pub(crate) struct AnyOfViewPy {
    #[pyo3(get)]
    schemas: Vec<Py<PyAny>>,
}

#[pymethods]
impl AnyOfViewPy {
    #[classattr]
    fn __match_args__() -> (&'static str,) {
        ("schemas",)
    }
}

#[pyclass(frozen, name = "OneOfView")]
pub(crate) struct OneOfViewPy {
    #[pyo3(get)]
    schemas: Vec<Py<PyAny>>,
}

#[pymethods]
impl OneOfViewPy {
    #[classattr]
    fn __match_args__() -> (&'static str,) {
        ("schemas",)
    }
}

#[pyclass(frozen, name = "NotView")]
pub(crate) struct NotViewPy {
    #[pyo3(get)]
    schema: Py<PyAny>,
}

#[pymethods]
impl NotViewPy {
    #[classattr]
    fn __match_args__() -> (&'static str,) {
        ("schema",)
    }
}

/// A value matches iff its JSON type is `type_name` *and* it satisfies `body`; other types do not match.
#[pyclass(frozen, name = "TypedGroupView")]
pub(crate) struct TypedGroupViewPy {
    #[pyo3(get)]
    type_name: String,
    #[pyo3(get)]
    body: Py<PyAny>,
}

#[pymethods]
impl TypedGroupViewPy {
    #[classattr]
    fn __match_args__() -> (&'static str, &'static str) {
        ("type_name", "body")
    }
}

/// Constrains only values of JSON type `type_name` (they must satisfy `body`); any other type matches unconditionally.
#[pyclass(frozen, name = "TypeGuardView")]
pub(crate) struct TypeGuardViewPy {
    #[pyo3(get)]
    type_name: String,
    #[pyo3(get)]
    body: Py<PyAny>,
}

#[pymethods]
impl TypeGuardViewPy {
    #[classattr]
    fn __match_args__() -> (&'static str, &'static str) {
        ("type_name", "body")
    }
}

#[pyclass(frozen, name = "ConstView")]
pub(crate) struct ConstViewPy {
    #[pyo3(get)]
    value: Py<PyAny>,
}

#[pymethods]
impl ConstViewPy {
    #[classattr]
    fn __match_args__() -> (&'static str,) {
        ("value",)
    }
}

#[pyclass(frozen, name = "EnumView")]
pub(crate) struct EnumViewPy {
    #[pyo3(get)]
    values: Vec<Py<PyAny>>,
}

#[pymethods]
impl EnumViewPy {
    #[classattr]
    fn __match_args__() -> (&'static str,) {
        ("values",)
    }
}

#[pyclass(frozen, name = "ReferenceView")]
pub(crate) struct ReferenceViewPy {
    #[pyo3(get)]
    uri: String,
}

#[pymethods]
impl ReferenceViewPy {
    #[classattr]
    fn __match_args__() -> (&'static str,) {
        ("uri",)
    }
}

// Re-keyed to the full target uri, so `.uri` keys into `definitions()` exactly like `ReferenceView`; one consumer
// lifter handles both.
#[pyclass(frozen, name = "RecursiveView")]
pub(crate) struct RecursiveViewPy {
    #[pyo3(get)]
    uri: String,
}

#[pymethods]
impl RecursiveViewPy {
    #[classattr]
    fn __match_args__() -> (&'static str,) {
        ("uri",)
    }
}

#[pyclass(frozen, name = "DynamicRefView")]
pub(crate) struct DynamicRefViewPy {
    #[pyo3(get)]
    name: String,
}

#[pymethods]
impl DynamicRefViewPy {
    #[classattr]
    fn __match_args__() -> (&'static str,) {
        ("name",)
    }
}

#[pyclass(frozen, name = "RawView")]
pub(crate) struct RawViewPy {
    #[pyo3(get)]
    schema: Py<PyAny>,
}

#[pymethods]
impl RawViewPy {
    #[classattr]
    fn __match_args__() -> (&'static str,) {
        ("schema",)
    }
}

fn schemas_to_py(py: Python<'_>, schemas: Vec<CanonicalSchema>) -> PyResult<Vec<Py<PyAny>>> {
    schemas.into_iter().map(|s| schema_to_py(py, s)).collect()
}

fn type_name(ty: JsonType) -> String {
    ty.as_str().to_string()
}

fn object_to_py(py: Python<'_>, view: ObjectView) -> PyResult<ObjectViewPy> {
    let requirements = view
        .requirements
        .into_iter()
        .map(|r| requirement_to_py(py, r))
        .collect::<PyResult<Vec<_>>>()?;
    let constraints = view
        .constraints
        .into_iter()
        .map(|c| constraint_to_py(py, c))
        .collect::<PyResult<Vec<_>>>()?;
    let property_names = match view.property_names {
        Some(c) => Some(schema_to_py(py, c)?),
        None => None,
    };
    Ok(ObjectViewPy {
        requirements,
        constraints,
        property_names,
        min_properties: number_to_py(py, view.min_properties)?,
        max_properties: number_to_py(py, view.max_properties)?,
    })
}

/// Convert a core view into the matching Python view object.
pub(crate) fn view_to_py(py: Python<'_>, view: CanonicalView) -> PyResult<Py<PyAny>> {
    let object = match view {
        CanonicalView::Null => Py::new(py, NullView)?.into_any(),
        CanonicalView::True => Py::new(py, TrueView)?.into_any(),
        CanonicalView::False => Py::new(py, FalseView)?.into_any(),
        CanonicalView::Boolean(b) => {
            let variant = match b.variant {
                BooleanVariant::Any => "any",
                BooleanVariant::JustTrue => "just_true",
                BooleanVariant::JustFalse => "just_false",
            };
            Py::new(py, BooleanViewPy { variant })?.into_any()
        }
        CanonicalView::Integer(n) => {
            let (
                minimum,
                maximum,
                exclusive_minimum,
                exclusive_maximum,
                multiple_of,
                not_multiple_of,
            ) = numeric_to_py(py, n)?;
            Py::new(
                py,
                IntegerViewPy {
                    minimum,
                    maximum,
                    exclusive_minimum,
                    exclusive_maximum,
                    multiple_of,
                    not_multiple_of,
                },
            )?
            .into_any()
        }
        CanonicalView::Number(n) => {
            let (
                minimum,
                maximum,
                exclusive_minimum,
                exclusive_maximum,
                multiple_of,
                not_multiple_of,
            ) = numeric_to_py(py, n)?;
            Py::new(
                py,
                NumberViewPy {
                    minimum,
                    maximum,
                    exclusive_minimum,
                    exclusive_maximum,
                    multiple_of,
                    not_multiple_of,
                },
            )?
            .into_any()
        }
        CanonicalView::String(s) => Py::new(py, string_to_py(py, s)?)?.into_any(),
        CanonicalView::Array(a) => Py::new(py, array_to_py(py, a)?)?.into_any(),
        CanonicalView::Object(o) => Py::new(py, object_to_py(py, o)?)?.into_any(),
        CanonicalView::MultiType(types) => Py::new(
            py,
            MultiTypeViewPy {
                types: types.iter().map(type_name).collect(),
            },
        )?
        .into_any(),
        CanonicalView::AllOf(s) => Py::new(
            py,
            AllOfViewPy {
                schemas: schemas_to_py(py, s)?,
            },
        )?
        .into_any(),
        CanonicalView::AnyOf(s) => Py::new(
            py,
            AnyOfViewPy {
                schemas: schemas_to_py(py, s)?,
            },
        )?
        .into_any(),
        CanonicalView::OneOf(s) => Py::new(
            py,
            OneOfViewPy {
                schemas: schemas_to_py(py, s)?,
            },
        )?
        .into_any(),
        CanonicalView::Not(s) => Py::new(
            py,
            NotViewPy {
                schema: schema_to_py(py, s)?,
            },
        )?
        .into_any(),
        CanonicalView::TypedGroup(v) => Py::new(
            py,
            TypedGroupViewPy {
                type_name: type_name(v.ty),
                body: schema_to_py(py, v.body)?,
            },
        )?
        .into_any(),
        CanonicalView::TypeGuard(v) => Py::new(
            py,
            TypeGuardViewPy {
                type_name: type_name(v.ty),
                body: schema_to_py(py, v.body)?,
            },
        )?
        .into_any(),
        CanonicalView::Const(value) => Py::new(
            py,
            ConstViewPy {
                value: canonical_value_to_python(py, &value)?,
            },
        )?
        .into_any(),
        CanonicalView::Enum(values) => {
            let values = values
                .iter()
                .map(|v| canonical_value_to_python(py, v))
                .collect::<PyResult<Vec<_>>>()?;
            Py::new(py, EnumViewPy { values })?.into_any()
        }
        CanonicalView::Reference(uri) => Py::new(py, ReferenceViewPy { uri })?.into_any(),
        CanonicalView::Recursive(uri) => Py::new(py, RecursiveViewPy { uri })?.into_any(),
        CanonicalView::DynamicRef(name) => Py::new(py, DynamicRefViewPy { name })?.into_any(),
        CanonicalView::Raw(schema) => Py::new(
            py,
            RawViewPy {
                schema: canonical_value_to_python(py, &schema)?,
            },
        )?
        .into_any(),
    };
    Ok(object)
}
