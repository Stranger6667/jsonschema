//! Retriever callback wrapper for Ruby.
use jsonschema::{Retrieve, Uri};
use magnus::{block::Proc, prelude::*, value::Opaque, Error, Ruby, Value};
use serde_json::Value as JsonValue;
use std::{ffi::c_void, panic::AssertUnwindSafe};

use crate::ser::to_value;

#[derive(Debug)]
pub enum RubyRetrieverError {
    ReturnedNil { uri: String },
    ConversionFailed { message: String },
    CallbackFailed { uri: String, message: String },
}

impl std::fmt::Display for RubyRetrieverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReturnedNil { uri } => write!(f, "Retriever returned nil for URI: {uri}"),
            Self::ConversionFailed { message } => {
                write!(f, "Failed to convert retriever result: {message}")
            }
            Self::CallbackFailed { uri, message } => {
                write!(f, "Retriever failed for {uri}: {message}")
            }
        }
    }
}

impl std::error::Error for RubyRetrieverError {}

pub fn retriever_error_message(error: &(dyn std::error::Error + 'static)) -> Option<String> {
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(err) = current {
        if let Some(retriever_error) = err.downcast_ref::<RubyRetrieverError>() {
            return Some(retriever_error.to_string());
        }
        current = err.source();
    }
    None
}

pub struct RubyRetriever {
    proc: Opaque<Proc>,
}

impl RubyRetriever {
    pub fn new(proc: Proc) -> Self {
        RubyRetriever {
            proc: Opaque::from(proc),
        }
    }
}

impl Retrieve for RubyRetriever {
    fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> Result<JsonValue, Box<dyn std::error::Error + Send + Sync>> {
        let ruby = Ruby::get().expect("Ruby VM should be initialized");
        let uri_str = uri.as_str();
        let proc = ruby.get_inner(self.proc);

        let result: Result<Value, Error> = proc.call((uri_str,));

        match result {
            Ok(value) => {
                if value.is_nil() {
                    return Err(Box::new(RubyRetrieverError::ReturnedNil {
                        uri: uri_str.to_owned(),
                    }));
                }
                to_value(&ruby, value).map_err(|e| {
                    Box::new(RubyRetrieverError::ConversionFailed {
                        message: e.to_string(),
                    }) as Box<dyn std::error::Error + Send + Sync>
                })
            }
            Err(e) => Err(Box::new(RubyRetrieverError::CallbackFailed {
                uri: uri_str.to_owned(),
                message: e.to_string(),
            })),
        }
    }
}

/// Convert a Ruby value (should be a Proc) to a retriever, if present
pub fn make_retriever(ruby: &Ruby, value: Value) -> Result<Option<RubyRetriever>, Error> {
    if value.is_nil() {
        return Ok(None);
    }

    let proc = Proc::from_value(value).ok_or_else(|| {
        Error::new(
            ruby.exception_type_error(),
            "Retriever must be a callable (Proc or Lambda)",
        )
    })?;

    Ok(Some(RubyRetriever::new(proc)))
}

/// The built-in HTTP and file retriever, run with the GVL released so other Ruby threads keep running while it waits.
pub struct DetachedRetriever(pub Option<jsonschema::HttpRetriever>);

type RetrieveResult = Result<JsonValue, Box<dyn std::error::Error + Send + Sync>>;

#[allow(unsafe_code)]
impl Retrieve for DetachedRetriever {
    fn retrieve(&self, uri: &Uri<String>) -> RetrieveResult {
        struct Call<'a> {
            retriever: &'a DetachedRetriever,
            uri: &'a Uri<String>,
            result: Option<std::thread::Result<RetrieveResult>>,
        }

        unsafe extern "C" fn call(data: *mut c_void) -> *mut c_void {
            // SAFETY: `data` is the `Call` below, alive until `rb_thread_call_without_gvl` returns
            let call = unsafe { &mut *data.cast::<Call<'_>>() };
            call.result = Some(std::panic::catch_unwind(AssertUnwindSafe(|| {
                match &call.retriever.0 {
                    Some(retriever) => retriever.retrieve(call.uri),
                    // A client per retrieval, as the core default retriever does; building one up front would cost every validator build
                    None => jsonschema::HttpRetriever::new(&jsonschema::HttpOptions::new())?
                        .retrieve(call.uri),
                }
            })));
            std::ptr::null_mut()
        }

        let mut data = Call {
            retriever: self,
            uri,
            result: None,
        };
        // SAFETY: `call` touches no Ruby objects, so it may run without the GVL
        unsafe {
            rb_sys::rb_thread_call_without_gvl(
                Some(call),
                (&raw mut data).cast(),
                None,
                std::ptr::null_mut(),
            );
        }
        match data.result.expect("Retrieval runs to completion") {
            Ok(result) => result,
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }
}
