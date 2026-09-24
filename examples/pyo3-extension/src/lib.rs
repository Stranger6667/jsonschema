use jsonschema::{json::Pyo3, paths::Location, Keyword, ValidationError};
use pyo3::{exceptions::PyValueError, prelude::*, types::PyString, Borrowed};
use serde_json::{Map, Value};

// `"uppercase": true` accepts strings without lowercase letters; other values are left alone.
struct Uppercase;

impl<'i> Keyword<'i, Pyo3> for Uppercase {
    fn validate(&self, instance: Borrowed<'i, 'i, PyAny>) -> Result<(), ValidationError<'i>> {
        if self.is_valid(instance) {
            Ok(())
        } else {
            Err(ValidationError::custom(format!(
                "{} is not uppercase",
                instance.as_any()
            )))
        }
    }

    fn is_valid(&self, instance: Borrowed<'i, 'i, PyAny>) -> bool {
        instance.cast::<PyString>().map_or(true, |text| {
            text.to_str()
                .is_ok_and(|text| !text.chars().any(char::is_lowercase))
        })
    }
}

// The keyword factory signature returns a `Result`.
#[allow(clippy::unnecessary_wraps)]
fn uppercase<'a>(
    _: &'a Map<String, Value>,
    _: &'a Value,
    _: Location,
) -> Result<Box<dyn for<'i> Keyword<'i, Pyo3>>, ValidationError<'a>> {
    Ok(Box::new(Uppercase))
}

#[jsonschema::validator(
    path = "schema.json",
    backend = Pyo3,
    keywords = { "uppercase" => crate::uppercase }
)]
struct Event;

#[pyfunction]
fn is_valid(instance: &Bound<'_, PyAny>) -> PyResult<bool> {
    Event::is_valid(instance)
}

/// Raises `ValueError` with the first error.
#[pyfunction]
fn validate(instance: &Bound<'_, PyAny>) -> PyResult<()> {
    match Event::validate(instance)? {
        Ok(()) => Ok(()),
        Err(error) => Err(PyValueError::new_err(error.to_string())),
    }
}

#[pyfunction]
fn errors(instance: &Bound<'_, PyAny>) -> PyResult<Vec<String>> {
    Ok(Event::iter_errors(instance)?
        .map(|error| error.to_string())
        .collect())
}

#[pymodule]
fn jsonschema_example_pyo3(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(is_valid, module)?)?;
    module.add_function(wrap_pyfunction!(validate, module)?)?;
    module.add_function(wrap_pyfunction!(errors, module)?)?;
    Ok(())
}
