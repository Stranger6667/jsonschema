#![cfg(not(target_arch = "wasm32"))]

use jsonschema::{validator_for, Draft, Evaluation, Retrieve, Uri, Validator};
use serde_json::Value;
use std::sync::OnceLock;
use testsuite::{output_suite, OutputRemote, OutputTest};

#[output_suite(
    path = "crates/jsonschema/tests/suite/output-tests",
    drafts = [
        "v1"
    ]
)]
fn output_suite(test: OutputTest) {
    run_output_case(test);
}

#[output_suite(
    path = "crates/jsonschema/tests/output-extra",
    drafts = [
        "v1-extra"
    ]
)]
fn output_suite_extra(test: OutputTest) {
    run_output_case(test);
}

#[allow(clippy::print_stderr)]
fn run_output_case(test: OutputTest) {
    let OutputTest {
        version,
        file,
        schema,
        case,
        description,
        data,
        outputs,
        remotes,
    } = test;

    let prepared_schema = prepare_schema_for_version(&schema, version);
    let validator = build_validator(&prepared_schema, version, file);
    let evaluation = validator.evaluate(&data);
    let retriever = output_schema_retriever(remotes);

    for expected in outputs {
        let format = expected.format;
        let schema = expected.schema;
        let mut expected_schema = prepare_schema_for_version(&schema, version);
        let mut actual_output = produce_output(&evaluation, format).unwrap_or_else(|| {
            panic!(
                "Output format `{format}` is not supported (file: {file}, case: `{case}`, test: `{description}`)"
            )
        });
        normalize_output(&mut actual_output);
        normalize_const_values(&mut expected_schema);
        validate_against_output_spec(&actual_output);
        let mut options = jsonschema::options().with_retriever(retriever);
        if let Some(draft) = version_draft_override(version) {
            options = options.with_draft(draft);
        }
        let output_validator = options.build(&expected_schema).unwrap_or_else(|err| {
            panic!("Invalid output schema for {file} format {format}: {err}")
        });
        if let Err(error) = output_validator.validate(&actual_output) {
            eprintln!("Output validation error: {error:?}");
            panic!(
                "Output format `{format}` failed for {file} (case: `{case}`, test: `{description}`): {error}"
            );
        }
    }
}

fn output_entry_sort_key(value: &Value) -> (&str, &str, &str) {
    let Some(entry) = value.as_object() else {
        return ("", "", "");
    };
    let evaluation_path = entry
        .get("evaluationPath")
        .and_then(Value::as_str)
        .unwrap_or("");
    let schema_location = entry
        .get("schemaLocation")
        .and_then(Value::as_str)
        .unwrap_or("");
    let instance_location = entry
        .get("instanceLocation")
        .and_then(Value::as_str)
        .unwrap_or("");
    (evaluation_path, schema_location, instance_location)
}

fn normalize_output(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for nested in map.values_mut() {
                normalize_output(nested);
            }
            if let Some(details) = map.get_mut("details").and_then(Value::as_array_mut) {
                details.sort_by(|left, right| {
                    output_entry_sort_key(left).cmp(&output_entry_sort_key(right))
                });
            }
            for key in ["annotations", "droppedAnnotations"] {
                if let Some(items) = map.get_mut(key).and_then(Value::as_array_mut) {
                    if items.iter().all(Value::is_string) {
                        items.sort_by(|left, right| {
                            left.as_str()
                                .unwrap_or("")
                                .cmp(right.as_str().unwrap_or(""))
                        });
                    }
                }
            }

            let mut keys: Vec<_> = map.keys().cloned().collect();
            keys.sort_unstable();
            let mut sorted = serde_json::Map::new();
            for key in keys {
                if let Some(value) = map.remove(&key) {
                    sorted.insert(key, value);
                }
            }
            *map = sorted;
        }
        Value::Array(items) => {
            for item in items {
                normalize_output(item);
            }
        }
        _ => {}
    }
}

fn normalize_const_values(schema: &mut Value) {
    match schema {
        Value::Object(map) => {
            for nested in map.values_mut() {
                normalize_const_values(nested);
            }
            if let Some(const_value) = map.get_mut("const") {
                normalize_output(const_value);
            }
        }
        Value::Array(items) => {
            for item in items {
                normalize_const_values(item);
            }
        }
        _ => {}
    }
}

fn output_spec_validator() -> &'static Validator {
    static VALIDATOR: OnceLock<Validator> = OnceLock::new();
    VALIDATOR.get_or_init(|| {
        let mut schema: Value = serde_json::from_str(include_str!("output_spec_schema.json"))
            .expect("output spec schema JSON is valid");
        if let Value::Object(ref mut map) = schema {
            map.remove("$schema");
        }
        validator_for(&schema).expect("output spec schema must be valid")
    })
}

fn validate_against_output_spec(value: &Value) {
    if let Err(error) = output_spec_validator().validate(value) {
        panic!("Output does not match JSON Schema validation-output schema: {error}");
    }
}

fn build_validator(schema: &Value, version: &str, file: &str) -> Validator {
    match version_draft_override(version) {
        Some(draft) => jsonschema::options()
            .with_draft(draft)
            .build(schema)
            .unwrap_or_else(|err| panic!("Invalid schema in {file}: {err}")),
        None => {
            validator_for(schema).unwrap_or_else(|err| panic!("Invalid schema in {file}: {err}"))
        }
    }
}

fn produce_output(evaluation: &Evaluation, format: &str) -> Option<Value> {
    match format {
        "flag" => {
            let value = serde_json::to_value(evaluation.flag()).expect("flag output serializable");
            debug_output("flag", &value);
            Some(value)
        }
        "list" => {
            let value = serde_json::to_value(evaluation.list()).expect("list output serializable");
            debug_output("list", &value);
            Some(value)
        }
        "hierarchical" => {
            let value = serde_json::to_value(evaluation.hierarchical())
                .expect("hierarchical output serializable");
            debug_output("hierarchical", &value);
            Some(value)
        }
        _ => None,
    }
}

// Prints serialized output when `JSONSCHEMA_DEBUG_OUTPUT` is set.
#[allow(clippy::print_stderr)]
fn debug_output(format: &str, value: &Value) {
    if std::env::var("JSONSCHEMA_DEBUG_OUTPUT").is_ok() {
        eprintln!(
            "=== {format} ===\n{}",
            serde_json::to_string_pretty(value).expect("output to stringify")
        );
    }
}

fn prepare_schema_for_version(schema: &Value, version: &str) -> Value {
    if is_v1(version) {
        if let Value::Object(mut map) = schema.clone() {
            map.remove("$schema");
            map.into()
        } else {
            schema.clone()
        }
    } else {
        schema.clone()
    }
}

fn version_draft_override(version: &str) -> Option<Draft> {
    match version {
        v if is_v1(v) => Some(Draft::Draft202012),
        _ => None,
    }
}

fn is_v1(version: &str) -> bool {
    version == "v1" || version.starts_with("v1-")
}

fn output_schema_retriever(remotes: &'static [OutputRemote]) -> OutputSchemaRetriever {
    OutputSchemaRetriever { documents: remotes }
}

#[derive(Clone, Copy)]
struct OutputSchemaRetriever {
    documents: &'static [OutputRemote],
}

impl Retrieve for OutputSchemaRetriever {
    fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        self.documents
            .iter()
            .find(|doc| doc.uri == uri.as_str())
            .map(|doc| {
                serde_json::from_str(doc.contents).expect("Output schema must be valid JSON")
            })
            .ok_or_else(|| format!("Unknown output schema reference: {uri}").into())
    }
}
