use jsonschema::{FancyRegex, PatternOptions, Regex};
use pyo3::{exceptions::PyTypeError, prelude::*};

/// A `pattern_options` argument resolved to a concrete engine.
pub(crate) enum PyPatternOptions {
    Fancy(PatternOptions<FancyRegex>),
    Regex(PatternOptions<Regex>),
}

/// Convert a Python `pattern_options` argument into [`PatternOptions`].
pub(crate) fn extract_pattern_options(value: &Bound<'_, PyAny>) -> PyResult<PyPatternOptions> {
    if let Ok(opts) = value.extract::<Py<FancyRegexOptions>>() {
        Ok(PyPatternOptions::Fancy(Python::attach(|py| {
            opts.borrow(py).to_pattern_options()
        })))
    } else if let Ok(opts) = value.extract::<Py<RegexOptions>>() {
        Ok(PyPatternOptions::Regex(Python::attach(|py| {
            opts.borrow(py).to_pattern_options()
        })))
    } else {
        Err(PyTypeError::new_err(
            "pattern_options must be an instance of FancyRegexOptions or RegexOptions",
        ))
    }
}

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

impl FancyRegexOptions {
    pub(crate) fn to_pattern_options(&self) -> PatternOptions<FancyRegex> {
        let mut options = PatternOptions::fancy_regex();
        if let Some(limit) = self.backtrack_limit {
            options = options.backtrack_limit(limit);
        }
        if let Some(limit) = self.size_limit {
            options = options.size_limit(limit);
        }
        if let Some(limit) = self.dfa_size_limit {
            options = options.dfa_size_limit(limit);
        }
        options
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

impl RegexOptions {
    pub(crate) fn to_pattern_options(&self) -> PatternOptions<Regex> {
        let mut options = PatternOptions::regex();
        if let Some(limit) = self.size_limit {
            options = options.size_limit(limit);
        }
        if let Some(limit) = self.dfa_size_limit {
            options = options.dfa_size_limit(limit);
        }
        options
    }
}
