use std::sync::Arc;

use pyo3::{exceptions::PyValueError, prelude::*, types::PyTuple};

use crate::{
    get_draft, referencing_error_pyerr, retriever::into_retriever, to_value, value_to_python,
    Draft, Retriever, DRAFT201909, DRAFT202012, DRAFT4, DRAFT6, DRAFT7,
};

/// A registry of JSON Schema resources, each identified by their canonical URIs.
#[pyclass(module = "jsonschema_rs")]
pub(crate) struct Registry {
    pub(crate) inner: Arc<jsonschema::Registry<'static>>,
}

#[pyclass(module = "jsonschema_rs", skip_from_py_object)]
#[derive(Clone)]
pub(crate) struct Resolver {
    registry: Arc<jsonschema::Registry<'static>>,
    base_uri: jsonschema::Uri<String>,
    dynamic_scope: Vec<jsonschema::Uri<String>>,
}

#[pyclass(module = "jsonschema_rs", skip_from_py_object)]
pub(crate) struct Resolved {
    contents: Py<PyAny>,
    resolver: Resolver,
    draft: u8,
}

fn draft_to_python(draft: Draft) -> u8 {
    match draft {
        Draft::Draft4 => DRAFT4,
        Draft::Draft6 => DRAFT6,
        Draft::Draft7 => DRAFT7,
        Draft::Draft201909 => DRAFT201909,
        _ => DRAFT202012,
    }
}

fn parse_uri(uri: &str) -> PyResult<jsonschema::Uri<String>> {
    jsonschema::uri::from_str(uri).map_err(|e| PyValueError::new_err(format!("{e}")))
}

#[pymethods]
impl Registry {
    #[new]
    #[pyo3(signature = (resources, draft=None, retriever=None))]
    fn new(
        resources: &Bound<'_, PyAny>,
        draft: Option<u8>,
        retriever: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let mut builder = jsonschema::Registry::new();

        if let Some(draft) = draft {
            builder = builder.draft(get_draft(draft)?);
        }

        if let Some(retriever) = retriever {
            let func = into_retriever(retriever)?;
            builder = builder.retriever(Retriever { func });
        }

        for item in resources.try_iter()? {
            let pair = item?;
            let (key, value) = pair.extract::<(Bound<PyAny>, Bound<PyAny>)>()?;
            let uri = key.extract::<String>()?;
            let schema = to_value(&value)?;
            builder = builder
                .add(uri, schema)
                .map_err(|e| PyValueError::new_err(format!("{e}")))?;
        }

        let registry = builder
            .prepare()
            .map_err(|e| PyValueError::new_err(format!("{e}")))?;

        Ok(Registry {
            inner: Arc::new(registry),
        })
    }

    fn resolver(&self, base_uri: &str) -> PyResult<Resolver> {
        Ok(Resolver {
            registry: Arc::clone(&self.inner),
            base_uri: parse_uri(base_uri)?,
            dynamic_scope: Vec::new(),
        })
    }
}

#[pymethods]
impl Resolver {
    #[getter]
    fn base_uri(&self) -> String {
        self.base_uri.as_str().to_string()
    }

    #[getter]
    fn dynamic_scope(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let scopes: Vec<String> = self
            .dynamic_scope
            .iter()
            .map(|scope| scope.as_str().to_string())
            .collect();
        Ok(PyTuple::new(py, &scopes)?.into_any().unbind())
    }

    fn lookup(&self, py: Python<'_>, reference: &str) -> PyResult<Resolved> {
        let resolver = if self.dynamic_scope.is_empty() {
            self.registry.as_ref().resolver(self.base_uri.clone())
        } else {
            let oldest_uri = self
                .dynamic_scope
                .last()
                .expect("dynamic_scope is not empty")
                .clone();
            let mut resolver = self.registry.as_ref().resolver(oldest_uri);
            for next_uri in self
                .dynamic_scope
                .iter()
                .rev()
                .skip(1)
                .chain(std::iter::once(&self.base_uri))
            {
                let next_resolved = match resolver.lookup(next_uri.as_str()) {
                    Ok(next_resolved) => next_resolved,
                    Err(error) => return Err(referencing_error_pyerr(py, error.to_string())?),
                };
                resolver = next_resolved.into_inner().1;
            }
            resolver
        };

        let resolved = match resolver.lookup(reference) {
            Ok(resolved) => resolved,
            Err(error) => return Err(referencing_error_pyerr(py, error.to_string())?),
        };
        let (contents, resolver, draft) = resolved.into_inner();

        Ok(Resolved {
            contents: value_to_python(py, contents)?,
            resolver: Resolver {
                registry: Arc::clone(&self.registry),
                base_uri: resolver.base_uri().as_ref().clone(),
                dynamic_scope: resolver.dynamic_scope().iter().cloned().collect(),
            },
            draft: draft_to_python(draft),
        })
    }
}

#[pymethods]
impl Resolved {
    #[getter]
    fn contents(&self, py: Python<'_>) -> Py<PyAny> {
        self.contents.clone_ref(py)
    }

    #[getter]
    fn resolver(&self) -> Resolver {
        self.resolver.clone()
    }

    #[getter]
    fn draft(&self) -> u8 {
        self.draft
    }
}
