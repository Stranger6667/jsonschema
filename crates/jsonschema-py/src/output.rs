use std::marker::PhantomData;

use ahash::AHashMap;
use jsonschema::{output::ErrorDescription, OutputUnit};
use pyo3::{
    intern,
    prelude::*,
    types::{PyBool, PyDict, PyList, PyString},
};

use crate::value_to_python;

/// Builds evaluation output units straight into Python objects.
///
/// Dict keys are written in sorted order, the order `serde_json::Map` gives them.
struct OutputBuilder<'a, 'py> {
    py: Python<'py>,
    keys: Keys<'py>,
    /// Units share location strings, so each shared one becomes a single `str`. Keyed by address:
    /// the evaluation is borrowed for `'a`, so no address is freed and reused meanwhile.
    locations: AHashMap<(*const u8, usize), Bound<'py, PyString>>,
    evaluation: PhantomData<&'a str>,
}

struct Keys<'py> {
    annotations: Bound<'py, PyString>,
    details: Bound<'py, PyString>,
    dropped_annotations: Bound<'py, PyString>,
    errors: Bound<'py, PyString>,
    evaluation_path: Bound<'py, PyString>,
    instance_location: Bound<'py, PyString>,
    schema_location: Bound<'py, PyString>,
    valid: Bound<'py, PyString>,
}

impl<'a, 'py> OutputBuilder<'a, 'py> {
    fn new(py: Python<'py>) -> Self {
        OutputBuilder {
            py,
            keys: Keys {
                annotations: intern!(py, "annotations").clone(),
                details: intern!(py, "details").clone(),
                dropped_annotations: intern!(py, "droppedAnnotations").clone(),
                errors: intern!(py, "errors").clone(),
                evaluation_path: intern!(py, "evaluationPath").clone(),
                instance_location: intern!(py, "instanceLocation").clone(),
                schema_location: intern!(py, "schemaLocation").clone(),
                valid: intern!(py, "valid").clone(),
            },
            locations: AHashMap::new(),
            evaluation: PhantomData,
        }
    }

    fn location(&mut self, location: &'a str) -> Bound<'py, PyString> {
        self.locations
            .entry((location.as_ptr(), location.len()))
            .or_insert_with(|| PyString::new(self.py, location))
            .clone()
    }

    fn unit(
        &mut self,
        unit: OutputUnit<'a>,
        details: Option<Bound<'py, PyList>>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let py = self.py;
        let dict = PyDict::new(py);
        if let Some(annotations) = unit.annotations() {
            dict.set_item(
                &self.keys.annotations,
                value_to_python(py, annotations.value())?,
            )?;
        }
        if let Some(details) = details {
            dict.set_item(&self.keys.details, details)?;
        }
        if let Some(annotations) = unit.dropped_annotations() {
            dict.set_item(
                &self.keys.dropped_annotations,
                value_to_python(py, annotations.value())?,
            )?;
        }
        if !unit.errors().is_empty() {
            dict.set_item(&self.keys.errors, errors_to_python(py, unit.errors())?)?;
        }
        let evaluation_path = self.location(unit.evaluation_path().as_str());
        dict.set_item(&self.keys.evaluation_path, evaluation_path)?;
        let instance_location = self.location(unit.instance_location().as_str());
        dict.set_item(&self.keys.instance_location, instance_location)?;
        let schema_location = self.location(unit.schema_location());
        dict.set_item(&self.keys.schema_location, schema_location)?;
        dict.set_item(&self.keys.valid, PyBool::new(py, unit.valid()))?;
        Ok(dict)
    }

    fn list(&mut self, unit: OutputUnit<'a>, out: &Bound<'py, PyList>) -> PyResult<()> {
        out.append(self.unit(unit, None)?)?;
        for child in unit.children() {
            self.list(child, out)?;
        }
        Ok(())
    }

    fn hierarchical(&mut self, unit: OutputUnit<'a>) -> PyResult<Bound<'py, PyDict>> {
        let mut children = unit.children().peekable();
        let details = if children.peek().is_some() {
            let details = PyList::empty(self.py);
            for child in children {
                details.append(self.hierarchical(child)?)?;
            }
            Some(details)
        } else {
            None
        };
        self.unit(unit, details)
    }
}

/// Messages grouped by keyword: one message as a `str`, several as a `list`.
fn errors_to_python<'py>(
    py: Python<'py>,
    errors: &[ErrorDescription],
) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    if let [error] = errors {
        dict.set_item(error.keyword(), error.message())?;
        return Ok(dict);
    }
    let mut sorted: Vec<&ErrorDescription> = errors.iter().collect();
    sorted.sort_by_key(|error| error.keyword());
    for group in sorted.chunk_by(|left, right| left.keyword() == right.keyword()) {
        if let [error] = group {
            dict.set_item(error.keyword(), error.message())?;
        } else {
            let messages = PyList::new(py, group.iter().map(|error| error.message()))?;
            dict.set_item(group[0].keyword(), messages)?;
        }
    }
    Ok(dict)
}

pub(crate) fn flag(py: Python<'_>, valid: bool) -> PyResult<Bound<'_, PyDict>> {
    let dict = PyDict::new(py);
    dict.set_item(intern!(py, "valid"), PyBool::new(py, valid))?;
    Ok(dict)
}

pub(crate) fn list<'py>(py: Python<'py>, root: OutputUnit<'_>) -> PyResult<Bound<'py, PyDict>> {
    let mut builder = OutputBuilder::new(py);
    let details = PyList::empty(py);
    builder.list(root, &details)?;
    let dict = PyDict::new(py);
    dict.set_item(&builder.keys.details, details)?;
    dict.set_item(&builder.keys.valid, PyBool::new(py, root.valid()))?;
    Ok(dict)
}

pub(crate) fn hierarchical<'py>(
    py: Python<'py>,
    root: OutputUnit<'_>,
) -> PyResult<Bound<'py, PyDict>> {
    OutputBuilder::new(py).hierarchical(root)
}
