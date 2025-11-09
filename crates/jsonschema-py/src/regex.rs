use pyo3::prelude::*;

/// FancyRegexOptions(backtrack_limit=None, size_limit=None, dfa_size_limit=None)
///
/// Configuration for the fancy-regex engine, which supports advanced regex features like
/// lookaround assertions and backreferences.
///
///     >>> validator = validator_for(
///     ...     {"type": "string", "pattern": "^(a+)+$"},  # Potentially problematic pattern
///     ...     pattern_options=FancyRegexOptions(backtrack_limit=10000)
///     ... )
///
/// Parameters:
///     backtrack_limit: Maximum number of backtracking steps
///     size_limit: Maximum compiled pattern size in bytes
///     dfa_size_limit: Maximum regex DFA cache size in bytes
#[pyclass(module = "jsonschema_rs")]
#[allow(clippy::struct_field_names)]
pub(crate) struct FancyRegexOptions {
    pub(crate) backtrack_limit: Option<usize>,
    pub(crate) size_limit: Option<usize>,
    pub(crate) dfa_size_limit: Option<usize>,
}

#[pymethods]
impl FancyRegexOptions {
    #[new]
    #[pyo3(signature = (backtrack_limit=None, size_limit=None, dfa_size_limit=None))]
    fn new(
        backtrack_limit: Option<usize>,
        size_limit: Option<usize>,
        dfa_size_limit: Option<usize>,
    ) -> Self {
        Self {
            backtrack_limit,
            size_limit,
            dfa_size_limit,
        }
    }
}

/// RegexOptions(size_limit=None, dfa_size_limit=None)
///
/// Configuration for the standard regex engine, which guarantees linear-time matching
/// to prevent regex DoS attacks but supports fewer features.
///
///     >>> validator = validator_for(
///     ...     schema,
///     ...     pattern_options=RegexOptions()
///     ... )
///
/// Parameters:
///     size_limit: Maximum compiled pattern size in bytes
///     dfa_size_limit: Maximum regex DFA cache size in bytes
///
/// Note: Unlike FancyRegexOptions, this engine doesn't support lookaround or backreferences.
#[pyclass(module = "jsonschema_rs")]
pub(crate) struct RegexOptions {
    pub(crate) size_limit: Option<usize>,
    pub(crate) dfa_size_limit: Option<usize>,
}

#[pymethods]
impl RegexOptions {
    #[new]
    #[pyo3(signature = (size_limit=None, dfa_size_limit=None))]
    fn new(size_limit: Option<usize>, dfa_size_limit: Option<usize>) -> Self {
        Self {
            size_limit,
            dfa_size_limit,
        }
    }
}
