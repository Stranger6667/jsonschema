//! Rust annotation test harness for the JSON Schema Annotation Test Suite.
//!
//! This test harness reads annotation test files from `suite/annotations/tests/`
//! and verifies that the jsonschema library produces the expected annotations.

#![cfg(not(target_arch = "wasm32"))]

use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

/// Root structure of an annotation test file.
#[derive(Debug, Deserialize)]
struct AnnotationTestFile {
    suite: Vec<SuiteCase>,
}

/// A test suite case containing a schema and multiple tests.
#[derive(Debug, Deserialize)]
struct SuiteCase {
    description: String,
    schema: Value,
    tests: Vec<TestCase>,
}

/// A single test case with an instance and assertions.
#[derive(Debug, Deserialize)]
struct TestCase {
    instance: Value,
    assertions: Vec<Assertion>,
}

/// An assertion about expected annotations.
#[derive(Debug, Deserialize)]
struct Assertion {
    /// Instance location (JSON pointer)
    location: String,
    /// Annotation keyword (e.g., "title", "description")
    keyword: String,
    /// Expected values: schema_location -> expected_value
    /// Empty map means no annotation expected at this location
    expected: HashMap<String, Value>,
}

/// Collect annotations from an evaluation result.
///
/// Returns a map from (instance_location, keyword) to a list of annotation values.
fn collect_annotations(
    evaluation: &jsonschema::Evaluation,
) -> HashMap<(String, String), Vec<Value>> {
    let mut result: HashMap<(String, String), Vec<Value>> = HashMap::new();

    for entry in evaluation.iter_annotations() {
        let instance_loc = entry.instance_location.as_str().to_string();
        let annotations_value = entry.annotations.value();

        // Annotations are stored as an object with keyword -> value pairs
        if let Some(annotations_obj) = annotations_value.as_object() {
            for (keyword, value) in annotations_obj {
                let key = (instance_loc.clone(), keyword.clone());
                result.entry(key).or_default().push(value.clone());
            }
        }
    }

    result
}

/// Known failing test IDs that should be skipped.
/// Format: "filename / description / test_index"
fn get_xfail_ids() -> HashSet<String> {
    let mut xfail = HashSet::new();

    // applicators.json failures
    xfail.insert("applicators.json / `anyOf` / 0".to_string());

    // content.json failures
    xfail.insert(
        "content.json / `contentMediaType` is an annotation for string instances / 1".to_string(),
    );
    xfail.insert(
        "content.json / `contentEncoding` is an annotation for string instances / 1".to_string(),
    );
    xfail.insert(
        "content.json / `contentSchema` is an annotation for string instances / 1".to_string(),
    );
    xfail.insert(
        "content.json / `contentSchema` requires `contentMediaType` / 0".to_string(),
    );

    // format.json failures
    xfail.insert("format.json / `format` is an annotation / 0".to_string());

    // unevaluatedProperties failures (all except "with `additionalProperties`")
    xfail.insert("unevaluated.json / `unevaluatedProperties` alone / 0".to_string());
    xfail.insert("unevaluated.json / `unevaluatedProperties` with `properties` / 0".to_string());
    xfail.insert(
        "unevaluated.json / `unevaluatedProperties` with `patternProperties` / 0".to_string(),
    );
    xfail.insert(
        "unevaluated.json / `unevaluatedProperties` with `dependentSchemas` / 0".to_string(),
    );
    xfail.insert(
        "unevaluated.json / `unevaluatedProperties` with `if`, `then`, and `else` / 0".to_string(),
    );
    xfail.insert(
        "unevaluated.json / `unevaluatedProperties` with `if`, `then`, and `else` / 1".to_string(),
    );
    xfail.insert("unevaluated.json / `unevaluatedProperties` with `allOf` / 0".to_string());
    xfail.insert("unevaluated.json / `unevaluatedProperties` with `anyOf` / 0".to_string());
    xfail.insert("unevaluated.json / `unevaluatedProperties` with `oneOf` / 0".to_string());
    xfail.insert("unevaluated.json / `unevaluatedProperties` with `not` / 0".to_string());

    // unevaluatedItems failures (all)
    xfail.insert("unevaluated.json / `unevaluatedItems` alone / 0".to_string());
    xfail.insert("unevaluated.json / `unevaluatedItems` with `prefixItems` / 0".to_string());
    xfail.insert("unevaluated.json / `unevaluatedItems` with `contains` / 0".to_string());
    xfail.insert(
        "unevaluated.json / `unevaluatedItems` with `if`, `then`, and `else` / 0".to_string(),
    );
    xfail.insert(
        "unevaluated.json / `unevaluatedItems` with `if`, `then`, and `else` / 1".to_string(),
    );
    xfail.insert("unevaluated.json / `unevaluatedItems` with `allOf` / 0".to_string());
    xfail.insert("unevaluated.json / `unevaluatedItems` with `anyOf` / 0".to_string());
    xfail.insert("unevaluated.json / `unevaluatedItems` with `oneOf` / 0".to_string());
    xfail.insert("unevaluated.json / `unevaluatedItems` with `not` / 0".to_string());

    xfail
}

#[test]
fn test_annotation_suite() {
    let suite_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("suite")
        .join("annotations")
        .join("tests");

    let xfail_ids = get_xfail_ids();
    let mut passed = 0;
    let mut skipped = 0;
    let mut failed = 0;
    let mut failures: Vec<String> = Vec::new();

    let mut entries: Vec<_> = fs::read_dir(&suite_path)
        .expect("Failed to read annotation test directory")
        .filter_map(Result::ok)
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext == "json")
        })
        .collect();
    entries.sort_by_key(|e| e.path());

    for entry in entries {
        let filepath = entry.path();
        let filename = filepath.file_name().unwrap().to_str().unwrap();

        let content = fs::read_to_string(&filepath)
            .unwrap_or_else(|e| panic!("Failed to read {}: {}", filepath.display(), e));

        let test_file: AnnotationTestFile = serde_json::from_str(&content)
            .unwrap_or_else(|e| panic!("Failed to parse {}: {}", filepath.display(), e));

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
                    failed += 1;
                    continue;
                }
            };

            for (test_idx, test_case) in suite_case.tests.iter().enumerate() {
                let test_id = format!("{filename} / {description} / {test_idx}");

                // Skip known failing tests
                if xfail_ids.contains(&test_id) {
                    skipped += 1;
                    continue;
                }

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
                        // Empty expected means no annotation should exist at this location
                        if !actual_values.is_empty() {
                            test_failed = true;
                            test_errors.push(format!(
                                "  Expected no annotation for keyword '{keyword}' at '{location}', but got: {actual_values:?}"
                            ));
                        }
                    } else {
                        // Check that each expected value is present
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
                    failed += 1;
                    failures.push(format!(
                        "FAILED: {test_id}\nInstance: {}\n{}",
                        serde_json::to_string(&test_case.instance).unwrap(),
                        test_errors.join("\n")
                    ));
                } else {
                    passed += 1;
                }
            }
        }
    }

    println!(
        "\nAnnotation Suite Results: {} passed, {} skipped (xfail), {} failed",
        passed, skipped, failed
    );

    if !failures.is_empty() {
        panic!(
            "\n{} annotation test(s) failed:\n\n{}",
            failures.len(),
            failures.join("\n\n")
        );
    }
}
