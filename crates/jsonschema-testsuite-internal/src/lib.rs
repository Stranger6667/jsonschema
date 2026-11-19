use serde_json::Value;
use std::collections::BTreeMap;

/// A suite file: related test cases, usually for one keyword.
#[derive(Debug, serde::Deserialize)]
pub struct SuiteFile {
    pub tests: Vec<Case>,
}

/// An individual test case, containing multiple tests of a single schema's behavior.
#[derive(Debug, serde::Deserialize)]
pub struct Case {
    /// Position within the suite file.
    #[serde(skip)]
    pub index: usize,
    /// The test case description.
    pub description: String,
    /// Dialects the case applies to; `None` means every dialect.
    pub compatibility: Option<String>,
    /// Behaviors an implementation must provide for the case to apply.
    #[serde(default)]
    pub tags: Vec<String>,
    /// A valid JSON Schema.
    pub schema: Value,
    /// Schemas the case's schema references, keyed by retrieval URI.
    #[serde(default, rename = "externalSchemas")]
    pub external_schemas: BTreeMap<String, Value>,
    /// A set of related tests all using the same schema.
    pub tests: Vec<InnerTest>,
}

/// A single test.
#[derive(Debug, serde::Deserialize)]
pub struct InnerTest {
    /// The test description, briefly explaining which behavior it exercises.
    pub description: String,
    /// Any additional comments about the test.
    pub comment: Option<String>,
    /// Instance validated against the surrounding `schema`.
    pub data: Value,
    /// Whether the instance is expected to be valid.
    pub valid: bool,
}

#[derive(Debug)]
pub struct Test {
    pub draft: &'static str,
    /// Suite file the case comes from, relative to `validation/tests`.
    pub file: &'static str,
    pub schema: Value,
    /// Schemas the case's schema references, keyed by retrieval URI.
    pub external_schemas: Vec<(&'static str, Value)>,
    pub case: &'static str,
    /// The `format` behavior the case expects; `None` leaves the draft's default.
    pub validate_formats: Option<bool>,
    /// The test description, briefly explaining which behavior it exercises.
    pub description: &'static str,
    /// Instance validated against the surrounding `schema`.
    pub data: Value,
    /// Whether the instance is expected to be valid.
    pub valid: bool,
}

/// A test case used by the output test-suite.
#[derive(Debug, serde::Deserialize)]
pub struct OutputCase {
    /// The test case description.
    pub description: String,
    /// A valid JSON Schema.
    pub schema: Value,
    /// A set of related tests all using the same schema.
    pub tests: Vec<OutputInnerTest>,
}

/// A single output test.
#[derive(Debug, serde::Deserialize)]
pub struct OutputInnerTest {
    /// The test description.
    pub description: String,
    /// The instance which should be validated against the schema in schema.
    pub data: Value,
    /// Expected output schemas keyed by format name.
    #[serde(default)]
    pub output: BTreeMap<String, Value>,
}

/// A single output format expectation.
#[derive(Debug)]
pub struct OutputFormat {
    /// Output format name (e.g. `flag`, `list`, `hierarchical`).
    pub format: &'static str,
    /// Schema describing the shape of the expected output.
    pub schema: Value,
}

/// A fully materialized test entry used by generated output tests.
#[derive(Debug)]
pub struct OutputTest {
    pub version: &'static str,
    pub file: &'static str,
    pub schema: Value,
    pub case: &'static str,
    pub description: &'static str,
    pub data: Value,
    pub outputs: Vec<OutputFormat>,
    pub remotes: &'static [OutputRemote],
    /// The generated validator's `evaluate`, when the generator supports the schema.
    pub evaluate: Option<fn(&Value) -> EvaluateOutputs>,
}

/// The `list` and `hierarchical` outputs of one `evaluate` run.
#[derive(Debug, PartialEq)]
pub struct EvaluateOutputs {
    pub list: Value,
    pub hierarchical: Value,
}

/// Remote schema contents used by the output test-suite.
#[derive(Debug)]
pub struct OutputRemote {
    pub uri: &'static str,
    pub contents: &'static str,
}

/// What a validation error reports, compared between generated and runtime validators.
#[derive(Debug, PartialEq, Eq)]
pub struct ErrorReport {
    pub message: String,
    pub schema_path: String,
    pub evaluation_path: String,
    pub absolute_keyword_location: Option<String>,
    pub instance_path: String,
}

/// Trait for compile-time generated validators.
///
/// This trait allows generated validators to be passed to test functions
/// using dynamic dispatch (`&dyn CodegenValidator`).
pub trait CodegenValidator {
    fn is_valid(&self, instance: &Value) -> bool;
    /// # Errors
    ///
    /// Returns an error if the instance is invalid according to the schema.
    fn validate(&self, instance: &Value) -> Result<(), Box<dyn std::any::Any + Send + Sync>>;
    /// Every error, in iteration order.
    fn iter_errors(&self, instance: &Value) -> Vec<ErrorReport>;
    /// The `list` and `hierarchical` outputs of `evaluate`.
    fn evaluate(&self, instance: &Value) -> (Value, Value);
}
