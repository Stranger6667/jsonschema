use serde_json::Value;
use std::collections::BTreeMap;

/// An individual test case, containing multiple tests of a single schema's behavior.
#[derive(Debug, serde::Deserialize)]
pub struct Case {
    /// The test case description.
    pub description: String,
    /// A valid JSON Schema.
    pub schema: Value,
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
    pub schema: Value,
    pub case: &'static str,
    pub is_optional: bool,
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
}

/// Remote schema contents used by the output test-suite.
#[derive(Debug)]
pub struct OutputRemote {
    pub uri: &'static str,
    pub contents: &'static str,
}
