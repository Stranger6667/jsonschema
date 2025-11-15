use jsonschema::Resource;
use pyo3::{exceptions::PyValueError, prelude::*};

use crate::{get_draft, retriever::into_retriever, to_value, Retriever};

/// A registry of JSON Schema resources, each identified by their canonical URIs.
#[pyclass]
pub(crate) struct Registry {
    pub(crate) inner: jsonschema::Registry,
}

#[pymethods]
impl Registry {
    #[new]
    #[pyo3(signature = (resources, draft=None, retriever=None))]
    fn new(
        py: Python<'_>,
        resources: &Bound<'_, PyAny>,
        draft: Option<u8>,
        retriever: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let mut options = jsonschema::Registry::options();

        if let Some(draft) = draft {
            options = options.draft(get_draft(draft)?);
        }

        if let Some(retriever) = retriever {
            let func = into_retriever(retriever)?;
            options = options.retriever(Retriever { func });
        }

        let pairs = resources.try_iter()?.map(|item| {
            let pair = item?.unbind();
            let (key, value) = pair.extract::<(Bound<PyAny>, Bound<PyAny>)>(py)?;
            let uri = key.extract::<String>()?;
            let schema = to_value(&value)?;
            let resource = Resource::from_contents(schema);
            Ok((uri, resource))
        });

        let pairs: Result<Vec<_>, PyErr> = pairs.collect();

        let registry = options
            .build(pairs?)
            .map_err(|e| PyValueError::new_err(e.to_string()))?;

        Ok(Registry { inner: registry })
    }
}
