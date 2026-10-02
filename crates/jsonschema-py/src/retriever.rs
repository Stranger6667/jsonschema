use jsonschema::{Retrieve, Uri};
use pyo3::{exceptions::PyValueError, prelude::*, types::PyString};

use serde_json::Value;

use crate::to_value;

pub(crate) struct Retriever<T: Fn(&str) -> PyResult<Value>> {
    pub(crate) func: T,
}

impl<T: Send + Sync + Fn(&str) -> PyResult<Value>> Retrieve for Retriever<T> {
    fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        Ok((self.func)(uri.as_str())?)
    }
}

pub(crate) fn into_retriever(
    retriever: &Bound<'_, PyAny>,
) -> PyResult<impl Fn(&str) -> PyResult<Value>> {
    if !retriever.is_callable() {
        return Err(PyValueError::new_err(
            "External resource retriever must be a callable",
        ));
    }
    let retriever: Py<PyAny> = retriever.clone().unbind();

    Ok(move |value: &str| {
        Python::attach(|py| {
            let value = PyString::new(py, value);
            retriever
                .call(py, (value,), None)
                .and_then(|value| to_value(value.bind(py)))
        })
    })
}

/// The built-in HTTP and file retriever, run with the GIL released so other Python threads keep running while it waits.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct DetachedRetriever(pub(crate) Option<jsonschema::HttpRetriever>);

#[cfg(not(target_arch = "wasm32"))]
impl Retrieve for DetachedRetriever {
    fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        Python::attach(|py| {
            py.detach(|| match &self.0 {
                Some(retriever) => retriever.retrieve(uri),
                // A client per retrieval, as the core default retriever does; building one up front would cost every validator build
                None => {
                    jsonschema::HttpRetriever::new(&jsonschema::HttpOptions::new())?.retrieve(uri)
                }
            })
        })
    }
}
