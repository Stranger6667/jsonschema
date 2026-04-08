use pyo3::{exceptions::PyValueError, prelude::*};

use crate::{get_draft, retriever::into_retriever, to_value, Retriever};

/// A registry of JSON Schema resources, each identified by their canonical URIs.
#[pyclass]
pub(crate) struct Registry {
    pub(crate) inner: jsonschema::Registry<'static>,
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
        let mut builder = jsonschema::Registry::new();

        if let Some(draft) = draft {
            builder = builder.draft(get_draft(draft)?);
        }

        if let Some(retriever) = retriever {
            let func = into_retriever(retriever)?;
            builder = builder.retriever(Retriever { func });
        }

        for item in resources.try_iter()? {
            let pair = item?.unbind();
            let (key, value) = pair.extract::<(Bound<PyAny>, Bound<PyAny>)>(py)?;
            let uri = key.extract::<String>()?;
            let schema = to_value(&value)?;
            builder = builder
                .add(uri, schema)
                .map_err(|e| PyValueError::new_err(e.to_string()))?;
        }

        let registry = builder
            .prepare()
            .map_err(|e| PyValueError::new_err(e.to_string()))?;

        Ok(Registry { inner: registry })
    }
}
