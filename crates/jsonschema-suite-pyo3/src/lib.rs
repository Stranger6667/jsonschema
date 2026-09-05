use pyo3::{exceptions::PyKeyError, prelude::*, types::PyAny};
use std::{collections::HashMap, sync::LazyLock};

/// A validation error as `(message, schema_path, instance_path)`.
type ErrorTriple = (String, String, String);

pub struct SuiteEntry {
    pub id: &'static str,
    pub is_valid: for<'py> fn(&Bound<'py, PyAny>) -> PyResult<bool>,
    pub validate: for<'py> fn(&Bound<'py, PyAny>) -> PyResult<Option<String>>,
    pub iter_errors: for<'py> fn(&Bound<'py, PyAny>) -> PyResult<Vec<ErrorTriple>>,
}

testsuite::pyo3_suite!(
    path = "crates/jsonschema/tests/suite",
    drafts = ["draft4", "draft6", "draft7", "draft2019-09", "draft2020-12",]
);

static BY_ID: LazyLock<HashMap<&'static str, &'static SuiteEntry>> =
    LazyLock::new(|| SUITE_ENTRIES.iter().map(|e| (e.id, e)).collect());

fn entry(case_id: &str) -> PyResult<&'static SuiteEntry> {
    BY_ID
        .get(case_id)
        .copied()
        .ok_or_else(|| PyKeyError::new_err(format!("No compiled validator for {case_id}")))
}

#[pyfunction]
fn case_ids() -> Vec<&'static str> {
    SUITE_ENTRIES.iter().map(|e| e.id).collect()
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
fn iter_errors(
    case_id: &str,
    instance: &Bound<'_, PyAny>,
) -> PyResult<Vec<(String, String, String)>> {
    (entry(case_id)?.iter_errors)(instance)
}

#[pymodule]
fn jsonschema_suite_pyo3(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(case_ids, module)?)?;
    module.add_function(wrap_pyfunction!(is_valid, module)?)?;
    module.add_function(wrap_pyfunction!(validate, module)?)?;
    module.add_function(wrap_pyfunction!(iter_errors, module)?)?;
    Ok(())
}
