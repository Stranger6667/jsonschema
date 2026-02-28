#![cfg(not(target_arch = "wasm32"))]

use ahash::HashMap;
use serde::Deserialize;
use serde_json::Value;
use std::{fs, path::Path};

#[derive(Debug, Deserialize)]
struct AnnotationTestFile {
    suite: Vec<SuiteCase>,
}

#[derive(Debug, Deserialize)]
struct SuiteCase {
    description: String,
    schema: Value,
    tests: Vec<TestCase>,
}

#[derive(Debug, Deserialize)]
struct TestCase {
    instance: Value,
    assertions: Vec<Assertion>,
}

#[derive(Debug, Deserialize)]
struct Assertion {
    location: String,
    keyword: String,
    expected: HashMap<String, Value>,
}

fn collect_annotations(
    evaluation: &jsonschema::Evaluation,
) -> HashMap<(String, String), Vec<Value>> {
    let mut result: HashMap<(String, String), Vec<Value>> = HashMap::default();

    for entry in evaluation.iter_annotations() {
        let instance_loc = entry.instance_location.as_str().to_string();
        let annotations_value = entry.annotations.value();

        if let Some(annotations_obj) = annotations_value.as_object() {
            // Metadata keywords (title, description, etc.) are bundled in an object
            for (keyword, value) in annotations_obj {
                let key = (instance_loc.clone(), keyword.clone());
                result.entry(key).or_default().push(value.clone());
            }
        } else {
            // Structural keywords (properties, format, etc.) have the keyword in the schema_location path
            let keyword = entry
                .schema_location
                .rsplit('/')
                .next()
                .unwrap_or(entry.schema_location)
                .to_string();
            let key = (instance_loc, keyword);
            result
                .entry(key)
                .or_default()
                .push(annotations_value.clone());
        }
    }

    result
}

#[test]
fn test_annotation_suite() {
    let suite_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("suite")
        .join("annotations")
        .join("tests");

    let mut failures: Vec<String> = Vec::new();

    let mut entries: Vec<_> = fs::read_dir(&suite_path)
        .expect("Failed to read annotation test directory")
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
        .collect();
    entries.sort_by_key(std::fs::DirEntry::path);

    for entry in entries {
        let filepath = entry.path();
        let filename = filepath.file_name().unwrap().to_str().unwrap();

        let content = fs::read_to_string(&filepath)
            .unwrap_or_else(|err| panic!("Failed to read {}: {err}", filepath.display()));

        let test_file: AnnotationTestFile = serde_json::from_str(&content)
            .unwrap_or_else(|err| panic!("Failed to parse {}: {err}", filepath.display()));

        for suite_case in &test_file.suite {
            let description = &suite_case.description;
            let schema = &suite_case.schema;

            let validator = match jsonschema::options().build(schema) {
                Ok(v) => v,
                Err(e) => {
                    let test_id = format!("{filename} / {description} / 0");
                    failures.push(format!(
                        "FAILED to build schema for {test_id}: {e}\nSchema: {}",
                        serde_json::to_string_pretty(schema).unwrap()
                    ));
                    continue;
                }
            };

            for (test_idx, test_case) in suite_case.tests.iter().enumerate() {
                let test_id = format!("{filename} / {description} / {test_idx}");

                let evaluation = validator.evaluate(&test_case.instance);
                let collected = collect_annotations(&evaluation);

                let mut test_failed = false;
                let mut test_errors: Vec<String> = Vec::new();

                for assertion in &test_case.assertions {
                    let location = &assertion.location;
                    let keyword = &assertion.keyword;
                    let expected = &assertion.expected;

                    let key = (location.clone(), keyword.clone());
                    let actual_values = collected.get(&key).cloned().unwrap_or_default();

                    if expected.is_empty() {
                        if !actual_values.is_empty() {
                            test_failed = true;
                            test_errors.push(format!(
                                "  Expected no annotation for keyword '{keyword}' at '{location}', but got: {actual_values:?}"
                            ));
                        }
                    } else {
                        for (schema_loc, expected_value) in expected {
                            if !actual_values.contains(expected_value) {
                                test_failed = true;
                                test_errors.push(format!(
                                    "  Keyword: {keyword:?}\n  Instance: {location:?}\n  Schema: {schema_loc:?}\n  Expected: {expected_value}\n  Got: {actual_values:?}"
                                ));
                            }
                        }
                    }
                }

                if test_failed {
                    failures.push(format!(
                        "FAILED: {test_id}\nInstance: {}\n{}",
                        serde_json::to_string(&test_case.instance).unwrap(),
                        test_errors.join("\n")
                    ));
                }
            }
        }
    }

    assert!(
        failures.is_empty(),
        "\n{} annotation test(s) failed:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
