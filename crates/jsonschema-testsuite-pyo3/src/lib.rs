use std::{collections::HashMap, sync::LazyLock};

use jsonschema::{json::Pyo3, paths::Location, Keyword, ValidationError};
use pyo3::{
    exceptions::PyKeyError,
    prelude::*,
    types::{PyAny, PyInt, PyList},
    Borrowed,
};
use serde_json::{Map, Value};

pub struct SuiteEntry {
    pub id: &'static str,
    pub is_valid: for<'py> fn(&Bound<'py, PyAny>) -> PyResult<bool>,
    pub validate: for<'py> fn(&Bound<'py, PyAny>) -> PyResult<Option<String>>,
    pub iter_errors: for<'py> fn(&Bound<'py, PyAny>) -> PyResult<Vec<String>>,
}

type KeywordResult<'a> = Result<Box<dyn for<'i> Keyword<'i, Pyo3>>, ValidationError<'a>>;

// Mirror `EvenValidator` and `AllPositiveValidator` in `tests-py/test_keywords.py`.
struct Even(bool);

impl<'i> Keyword<'i, Pyo3> for Even {
    fn validate(&self, instance: Borrowed<'i, 'i, PyAny>) -> Result<(), ValidationError<'i>> {
        if self.is_valid(instance) {
            Ok(())
        } else {
            Err(ValidationError::custom(format!(
                "{} is not even",
                instance.as_any()
            )))
        }
    }

    fn is_valid(&self, instance: Borrowed<'i, 'i, PyAny>) -> bool {
        !self.0
            || !instance.is_instance_of::<PyInt>()
            || !instance
                .rem(2)
                .and_then(|remainder| remainder.is_truthy())
                .unwrap_or(false)
    }
}

// The keyword factory signature returns a `Result`.
#[allow(clippy::unnecessary_wraps)]
pub(crate) fn even<'a>(
    _: &'a Map<String, Value>,
    value: &'a Value,
    _: Location,
) -> KeywordResult<'a> {
    Ok(Box::new(Even(value.as_bool() == Some(true))))
}

struct AllPositive;

impl AllPositive {
    fn negative_items<'i>(instance: Borrowed<'i, 'i, PyAny>) -> Vec<ValidationError<'i>> {
        let Ok(items) = instance.cast::<PyList>() else {
            return Vec::new();
        };
        items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.lt(0).unwrap_or(false))
            .map(|(index, _)| ValidationError::custom(format!("item {index} is negative")))
            .collect()
    }
}

impl<'i> Keyword<'i, Pyo3> for AllPositive {
    fn validate(&self, instance: Borrowed<'i, 'i, PyAny>) -> Result<(), ValidationError<'i>> {
        Self::negative_items(instance)
            .into_iter()
            .next()
            .map_or(Ok(()), Err)
    }

    fn is_valid(&self, instance: Borrowed<'i, 'i, PyAny>) -> bool {
        Self::negative_items(instance).is_empty()
    }

    fn iter_errors(
        &self,
        instance: Borrowed<'i, 'i, PyAny>,
    ) -> Box<dyn Iterator<Item = ValidationError<'i>> + 'i> {
        Box::new(Self::negative_items(instance).into_iter())
    }
}

// The keyword factory signature returns a `Result`.
#[allow(clippy::unnecessary_wraps)]
pub(crate) fn all_positive<'a>(
    _: &'a Map<String, Value>,
    _: &'a Value,
    _: Location,
) -> KeywordResult<'a> {
    Ok(Box::new(AllPositive))
}

testsuite::pyo3_suite!(
    path = "crates/jsonschema/tests/suite",
    drafts = ["draft4", "draft6", "draft7", "draft2019-09", "draft2020-12"]
);
testsuite::pyo3_schemas!("crates/jsonschema-py/tests-py/codegen_schemas.json");

static BY_ID: LazyLock<HashMap<&'static str, &'static SuiteEntry>> = LazyLock::new(|| {
    SUITE_ENTRIES
        .iter()
        .chain(SCHEMA_ENTRIES)
        .map(|entry| (entry.id, entry))
        .collect()
});

fn entry(case_id: &str) -> PyResult<&'static SuiteEntry> {
    BY_ID
        .get(case_id)
        .copied()
        .ok_or_else(|| PyKeyError::new_err(format!("No compiled validator for {case_id}")))
}

#[pyfunction]
fn is_valid(case_id: &str, instance: &Bound<'_, PyAny>) -> PyResult<bool> {
    (entry(case_id)?.is_valid)(instance)
}

#[pyfunction]
fn validate(case_id: &str, instance: &Bound<'_, PyAny>) -> PyResult<Option<String>> {
    (entry(case_id)?.validate)(instance)
}

#[pyfunction]
fn iter_errors(case_id: &str, instance: &Bound<'_, PyAny>) -> PyResult<Vec<String>> {
    (entry(case_id)?.iter_errors)(instance)
}

#[pymodule]
fn jsonschema_testsuite_pyo3(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(is_valid, module)?)?;
    module.add_function(wrap_pyfunction!(validate, module)?)?;
    module.add_function(wrap_pyfunction!(iter_errors, module)?)?;
    Ok(())
}
