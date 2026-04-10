use assert_cmd::{cargo::cargo_bin_cmd, Command};
use insta::assert_snapshot;
use serde_json::Value;
use std::{collections::HashMap, fs};
use tempfile::tempdir;

fn cli() -> Command {
    cargo_bin_cmd!("jsonschema-cli")
}

fn create_temp_file(dir: &tempfile::TempDir, name: &str, content: &str) -> String {
    let file_path = dir.path().join(name);
    fs::write(&file_path, content).unwrap();
    file_path.to_str().unwrap().to_string()
}

fn sanitize_output(output: String, file_names: &[&str]) -> String {
    let mut sanitized = output;
    for (i, name) in file_names.iter().enumerate() {
        sanitized = sanitized.replace(name, &format!("{{FILE_{}}}", i + 1));
    }
    sanitized
}

fn parse_ndjson(output: &str) -> Vec<Value> {
    output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn normalize_numbered_errors(output: &str) -> String {
    let mut lines = output.lines();
    let Some(header) = lines.next() else {
        return output.to_string();
    };
    let mut errors: Vec<_> = lines
        .filter_map(|line| {
            line.split_once(". ")
                .map(|(_, message)| message.to_string())
        })
        .collect();
    if errors.is_empty() {
        return output.to_string();
    }
    errors.sort_unstable();

    let mut normalized = String::new();
    normalized.push_str(header);
    for (idx, message) in errors.into_iter().enumerate() {
        normalized.push('\n');
        normalized.push_str(&(idx + 1).to_string());
        normalized.push_str(". ");
        normalized.push_str(&message);
    }
    normalized.push('\n');
    normalized
}

#[test]
fn test_version() {
    let mut cmd = cli();
    cmd.arg("--version");
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        concat!("Version: ", env!("CARGO_PKG_VERSION"), "\n")
    );
}

#[test]
fn test_valid_instance() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r#"{"type": "object", "properties": {"name": {"type": "string"}}}"#,
    );
    let instance = create_temp_file(&dir, "instance.json", r#"{"name": "John Doe"}"#);

    let mut cmd = cli();
    cmd.arg(&schema).arg("--instance").arg(&instance);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let sanitized = sanitize_output(
        String::from_utf8_lossy(&output.stdout).to_string(),
        &[&instance],
    );
    assert_snapshot!(sanitized);
}

#[test]
fn test_invalid_instance() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r#"{"type": "object", "properties": {"name": {"type": "string"}}}"#,
    );
    let instance = create_temp_file(&dir, "instance.json", r#"{"name": 123}"#);

    let mut cmd = cli();
    cmd.arg(&schema).arg("--instance").arg(&instance);
    let output = cmd.output().unwrap();
    assert!(!output.status.success());
    let sanitized = sanitize_output(
        String::from_utf8_lossy(&output.stdout).to_string(),
        &[&instance],
    );
    assert_snapshot!(sanitized);
}

#[test]
fn test_invalid_schema() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", r#"{"type": "invalid"}"#);
    let instance = create_temp_file(&dir, "instance.json", "{}");

    let mut cmd = cli();
    cmd.arg(&schema).arg("--instance").arg(&instance);
    let output = cmd.output().unwrap();
    assert!(!output.status.success());
    let sanitized = sanitize_output(
        String::from_utf8_lossy(&output.stdout).to_string(),
        &[&instance],
    );
    assert_snapshot!(sanitized);
}

#[test]
fn test_multiple_instances() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r#"{"type": "object", "properties": {"name": {"type": "string"}}}"#,
    );
    let instance1 = create_temp_file(&dir, "instance1.json", r#"{"name": "John Doe"}"#);
    let instance2 = create_temp_file(&dir, "instance2.json", r#"{"name": 123}"#);

    let mut cmd = cli();
    cmd.arg(&schema)
        .arg("--instance")
        .arg(&instance1)
        .arg("--instance")
        .arg(&instance2);
    let output = cmd.output().unwrap();
    assert!(!output.status.success());
    let sanitized = sanitize_output(
        String::from_utf8_lossy(&output.stdout).to_string(),
        &[&instance1, &instance2],
    );
    assert_snapshot!(sanitized);
}

#[test]
fn test_multiple_instances_single_flag() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r#"{"type": "object", "properties": {"name": {"type": "string"}}}"#,
    );
    let instance1 = create_temp_file(&dir, "instance1.json", r#"{"name": "John Doe"}"#);
    let instance2 = create_temp_file(&dir, "instance2.json", r#"{"name": 123}"#);

    let mut cmd = cli();
    cmd.arg("validate")
        .arg(&schema)
        .arg("--instance")
        .arg(&instance1)
        .arg(&instance2);
    let output = cmd.output().unwrap();
    assert!(!output.status.success());
    let sanitized = sanitize_output(
        String::from_utf8_lossy(&output.stdout).to_string(),
        &[&instance1, &instance2],
    );
    assert_snapshot!(sanitized);
}

#[test]
fn test_no_instances() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", r#"{"type": "object"}"#);

    let mut cmd = cli();
    cmd.arg(&schema);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    assert_snapshot!(String::from_utf8_lossy(&output.stdout));
}

#[test]
fn test_relative_resolution() {
    let dir = tempdir().unwrap();

    let a_schema = create_temp_file(
        &dir,
        "a.json",
        r#"
        {
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$ref": "./b.json",
            "type": "object"
        }
        "#,
    );

    let _b_schema = create_temp_file(
        &dir,
        "b.json",
        r#"
        {
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "additionalProperties": false,
            "properties": {
                "$schema": {
                    "type": "string"
                }
            }
        }
        "#,
    );

    let valid_instance = create_temp_file(
        &dir,
        "instance.json",
        r#"
        {
            "$schema": "a.json"
        }
        "#,
    );

    let mut cmd = cli();
    cmd.arg(&a_schema).arg("--instance").arg(&valid_instance);
    let output = cmd.output().unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );

    let sanitized = sanitize_output(
        String::from_utf8_lossy(&output.stdout).to_string(),
        &[&valid_instance, &a_schema],
    );
    assert_snapshot!(sanitized);

    let invalid_instance = create_temp_file(
        &dir,
        "instance.json",
        r#"
        {
            "$schema": 42
        }
        "#,
    );

    let mut cmd = cli();
    cmd.arg(&a_schema).arg("--instance").arg(&invalid_instance);
    let output = cmd.output().unwrap();

    assert!(
        !output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );

    let sanitized = sanitize_output(
        String::from_utf8_lossy(&output.stdout).to_string(),
        &[&valid_instance, &a_schema],
    );
    assert_snapshot!(sanitized);
}

#[test]
fn test_nested_ref_resolution_with_different_path_formats() {
    let temp_dir = tempdir().unwrap();
    let folder_a = temp_dir.path().join("folderA");
    let folder_b = folder_a.join("folderB");

    fs::create_dir_all(&folder_b).unwrap();

    let schema_content = r#"{
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "name": {"$ref": "folderB/subschema.json#/definitions/name"}
        }
    }"#;

    let subschema_content = r#"{
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "definitions": {
            "name": {
                "type": "string",
                "minLength": 3
            }
        }
    }"#;

    let instance_content = r#"{"name": "John"}"#;

    let schema_path = folder_a.join("schema.json");
    let subschema_path = folder_b.join("subschema.json");
    let instance_path = temp_dir.path().join("instance.json");

    fs::write(&schema_path, schema_content).unwrap();
    fs::write(&subschema_path, subschema_content).unwrap();
    fs::write(&instance_path, instance_content).unwrap();

    let mut cmd = cli();
    cmd.arg(schema_path.to_str().unwrap())
        .arg("--instance")
        .arg(instance_path.to_str().unwrap());

    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "Validation with absolute path failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );

    let rel_schema_path = "folderA/schema.json";
    let rel_instance_path = "instance.json";

    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(temp_dir.path()).unwrap();

    let mut cmd = cli();
    cmd.arg(rel_schema_path)
        .arg("--instance")
        .arg(rel_instance_path);

    let output = cmd.output().unwrap();

    assert!(output.status.success());

    std::env::set_current_dir(original_dir).unwrap();
}

#[test]
fn test_draft_enforcement_property_names() {
    let dir = tempdir().unwrap();

    // Schema uses `propertyNames`, which Draft 4 doesn’t understand (so it’s ignored)
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r#"
        {
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "propertyNames": { "pattern": "^a" }
        }
        "#,
    );

    let bad = create_temp_file(&dir, "bad.json", r#"{ "foo": 1 }"#);
    let good = create_temp_file(&dir, "good.json", r#"{ "apple": 2 }"#);

    // Draft 4: propertyNames is ignored → both should be valid
    let mut cmd = cli();
    cmd.arg(&schema)
        .arg("-d")
        .arg("4")
        .arg("--instance")
        .arg(&bad)
        .arg("--instance")
        .arg(&good);
    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "Draft 4 should ignore propertyNames:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let out = sanitize_output(
        String::from_utf8_lossy(&output.stdout).to_string(),
        &[&bad, &good],
    );
    assert_snapshot!("draft4_property_names_ignored", out);

    // Draft 2020: propertyNames enforced → “bad” fails, “good” passes
    let mut cmd = cli();
    cmd.arg(&schema)
        // omit `-d` to use default (2020), or explicitly `-d 2020`
        .arg("--instance")
        .arg(&bad)
        .arg("--instance")
        .arg(&good);
    let output = cmd.output().unwrap();
    assert!(
        !output.status.success(),
        "Draft 2020 should enforce propertyNames:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let out = sanitize_output(
        String::from_utf8_lossy(&output.stdout).to_string(),
        &[&bad, &good],
    );
    assert_snapshot!("draft2020_property_names_enforced", out);
}

#[test]
fn test_format_enforcement_via_cli_flag() {
    let dir = tempdir().unwrap();

    let schema = create_temp_file(
        &dir,
        "schema.json",
        r#"
        {
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "email": { "type": "string", "format": "email" }
            }
        }
        "#,
    );

    let invalid = create_temp_file(&dir, "invalid.json", r#"{ "email": "not-an-email" }"#);

    // Format validation disabled (default behavior)
    let mut cmd = cli();
    cmd.arg(&schema).arg("--instance").arg(&invalid);
    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "Expected success with format validation disabled:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let out = sanitize_output(
        String::from_utf8_lossy(&output.stdout).to_string(),
        &[&invalid],
    );
    assert_snapshot!("format_enforcement_disabled", out);

    // Format validation explicitly enabled via CLI flag
    let mut cmd = cli();
    cmd.arg(&schema)
        .arg("--instance")
        .arg(&invalid)
        .arg("--assert-format");
    let output = cmd.output().unwrap();
    assert!(
        !output.status.success(),
        "Expected failure with format validation enabled:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let out = sanitize_output(
        String::from_utf8_lossy(&output.stdout).to_string(),
        &[&invalid],
    );
    assert_snapshot!("format_enforcement_enabled", out);
}

#[test]
fn test_output_flag_ndjson() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r#"{"type": "object", "properties": {"name": {"type": "string"}}}"#,
    );
    let valid = create_temp_file(&dir, "valid.json", r#"{"name": "John"}"#);
    let invalid = create_temp_file(&dir, "invalid.json", r#"{"name": 123}"#);

    let mut cmd = cli();
    cmd.arg(&schema)
        .arg("--instance")
        .arg(&valid)
        .arg("--instance")
        .arg(&invalid)
        .arg("--output")
        .arg("flag");
    let output = cmd.output().unwrap();
    assert!(
        !output.status.success(),
        "flag output should fail when an instance is invalid"
    );
    let records = parse_ndjson(&String::from_utf8_lossy(&output.stdout));
    assert_eq!(records.len(), 2);
    for record in &records {
        assert_eq!(record["output"], "flag");
        assert_eq!(record["schema"], schema);
    }
    let mut by_instance = HashMap::new();
    for record in records {
        let instance = record["instance"].as_str().unwrap();
        let valid = record["payload"]["valid"].as_bool().unwrap();
        by_instance.insert(instance.to_string(), valid);
    }
    assert_eq!(by_instance.get(&valid), Some(&true));
    assert_eq!(by_instance.get(&invalid), Some(&false));
}

#[test]
fn test_output_list_ndjson() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r#"{"type": "object", "properties": {"age": {"type": "number"}}}"#,
    );
    let valid = create_temp_file(&dir, "valid.json", r#"{"age": 42}"#);
    let invalid = create_temp_file(&dir, "invalid.json", r#"{"age": "old"}"#);

    let mut cmd = cli();
    cmd.arg(&schema)
        .arg("--instance")
        .arg(&valid)
        .arg("--instance")
        .arg(&invalid)
        .arg("--output")
        .arg("list");
    let output = cmd.output().unwrap();
    assert!(
        !output.status.success(),
        "list output should fail when an instance is invalid"
    );
    let records = parse_ndjson(&String::from_utf8_lossy(&output.stdout));
    assert_eq!(records.len(), 2);
    for record in records {
        assert_eq!(record["output"], "list");
        assert_eq!(record["schema"], schema);
        assert!(
            record["payload"]["details"].is_array(),
            "list payload must contain details array"
        );
    }
}

#[test]
fn test_output_text_valid() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r#"{"type": "object", "properties": {"name": {"type": "string"}}}"#,
    );
    let valid = create_temp_file(&dir, "valid.json", r#"{"name": "Alice"}"#);

    let mut cmd = cli();
    cmd.arg(&schema)
        .arg("--instance")
        .arg(&valid)
        .arg("--output")
        .arg("text");
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let sanitized = sanitize_output(
        String::from_utf8_lossy(&output.stdout).to_string(),
        &[&valid],
    );
    assert_snapshot!(sanitized);
}

#[test]
fn test_output_text_single_error() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r#"{"type": "object", "properties": {"age": {"type": "number"}}}"#,
    );
    let invalid = create_temp_file(&dir, "invalid.json", r#"{"age": "not a number"}"#);

    let mut cmd = cli();
    cmd.arg(&schema)
        .arg("--instance")
        .arg(&invalid)
        .arg("--output")
        .arg("text");
    let output = cmd.output().unwrap();
    assert!(!output.status.success());
    let sanitized = sanitize_output(
        String::from_utf8_lossy(&output.stdout).to_string(),
        &[&invalid],
    );
    assert_snapshot!(sanitized);
}

#[test]
fn test_output_text_multiple_errors() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r#"{
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "number"},
                "email": {"type": "string"}
            },
            "required": ["name", "age", "email"]
        }"#,
    );
    let invalid = create_temp_file(
        &dir,
        "invalid.json",
        r#"{"name": 123, "age": "not a number"}"#,
    );

    let mut cmd = cli();
    cmd.arg(&schema)
        .arg("--instance")
        .arg(&invalid)
        .arg("--output")
        .arg("text");
    let output = cmd.output().unwrap();
    assert!(!output.status.success());
    let out = String::from_utf8_lossy(&output.stdout);
    let sanitized = sanitize_output(out.to_string(), &[&invalid]);

    // Verify error numbering: "1. <error>", "2. <error>", "3. <error>"
    assert!(sanitized.contains("1. "));
    assert!(sanitized.contains("2. "));
    assert!(sanitized.contains("3. "));
    assert_snapshot!(normalize_numbered_errors(&sanitized));
}

#[test]
fn test_output_text_valid_yaml() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r#"{
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer"}
            },
            "required": ["name", "age"]
        }"#,
    );
    let valid = create_temp_file(&dir, "valid.yaml", "name: Alice\nage: 30\n");

    let mut cmd = cli();
    cmd.arg(&schema)
        .arg("--instance")
        .arg(&valid)
        .arg("--output")
        .arg("text");
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let sanitized = sanitize_output(
        String::from_utf8_lossy(&output.stdout).to_string(),
        &[&valid],
    );
    assert_snapshot!(sanitized);
}

#[test]
fn test_output_text_invalid_yaml() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r#"{
            "type": "object",
            "properties": {
                "age": {"type": "integer"}
            },
            "required": ["age"]
        }"#,
    );
    let invalid = create_temp_file(&dir, "invalid.yaml", "age: not a number\n");

    let mut cmd = cli();
    cmd.arg(&schema)
        .arg("--instance")
        .arg(&invalid)
        .arg("--output")
        .arg("text");
    let output = cmd.output().unwrap();
    assert!(!output.status.success());
    let sanitized = sanitize_output(
        String::from_utf8_lossy(&output.stdout).to_string(),
        &[&invalid],
    );
    assert_snapshot!(sanitized);
}

#[test]
fn test_output_text_valid_yml() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", r#"{"type": "integer"}"#);
    let valid = create_temp_file(&dir, "valid.yml", "42\n");

    let mut cmd = cli();
    cmd.arg(&schema)
        .arg("--instance")
        .arg(&valid)
        .arg("--output")
        .arg("text");
    let output = cmd.output().unwrap();
    assert!(output.status.success());

    let sanitized = sanitize_output(
        String::from_utf8_lossy(&output.stdout).to_string(),
        &[&valid],
    );
    assert_eq!(sanitized, "{FILE_1} - VALID\n");
}

#[test]
fn test_output_text_invalid_yaml_syntax() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", r#"{"type": "object"}"#);
    let invalid = create_temp_file(&dir, "invalid.yaml", "name: [Alice\n");

    let mut cmd = cli();
    cmd.arg(&schema)
        .arg("--instance")
        .arg(&invalid)
        .arg("--output")
        .arg("text");
    let output = cmd.output().unwrap();
    assert!(!output.status.success());

    let sanitized = sanitize_output(
        String::from_utf8_lossy(&output.stdout).to_string(),
        &[&invalid],
    );
    assert!(sanitized.contains("Error: failed to read YAML from {FILE_1}:"));
}

#[test]
fn test_output_hierarchical_valid() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r#"{"type": "object", "properties": {"name": {"type": "string"}}}"#,
    );
    let valid = create_temp_file(&dir, "valid.json", r#"{"name": "Bob"}"#);

    let mut cmd = cli();
    cmd.arg(&schema)
        .arg("--instance")
        .arg(&valid)
        .arg("--output")
        .arg("hierarchical");
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let records = parse_ndjson(&String::from_utf8_lossy(&output.stdout));
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record["output"], "hierarchical");
    assert_eq!(record["schema"], schema);
    assert_eq!(record["instance"], valid);
    assert_eq!(record["payload"]["valid"], true);
}

#[test]
fn test_output_hierarchical_invalid() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r#"{
            "type": "object",
            "properties": {
                "age": {"type": "number", "minimum": 0}
            }
        }"#,
    );
    let invalid = create_temp_file(&dir, "invalid.json", r#"{"age": "invalid"}"#);

    let mut cmd = cli();
    cmd.arg(&schema)
        .arg("--instance")
        .arg(&invalid)
        .arg("--output")
        .arg("hierarchical");
    let output = cmd.output().unwrap();
    assert!(!output.status.success());
    let records = parse_ndjson(&String::from_utf8_lossy(&output.stdout));
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record["output"], "hierarchical");
    assert_eq!(record["schema"], schema);
    assert_eq!(record["instance"], invalid);
    assert_eq!(record["payload"]["valid"], false);
}

#[test]
fn test_output_hierarchical_multiple_instances() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", r#"{"type": "string", "minLength": 3}"#);
    let valid = create_temp_file(&dir, "valid.json", r#""hello""#);
    let invalid = create_temp_file(&dir, "invalid.json", r#""no""#);

    let mut cmd = cli();
    cmd.arg(&schema)
        .arg("--instance")
        .arg(&valid)
        .arg("--instance")
        .arg(&invalid)
        .arg("--output")
        .arg("hierarchical");
    let output = cmd.output().unwrap();
    assert!(!output.status.success());
    let records = parse_ndjson(&String::from_utf8_lossy(&output.stdout));
    assert_eq!(records.len(), 2);

    let mut results = HashMap::new();
    for record in &records {
        assert_eq!(record["output"], "hierarchical");
        assert_eq!(record["schema"], schema);
        let instance = record["instance"].as_str().unwrap();
        let valid = record["payload"]["valid"].as_bool().unwrap();
        results.insert(instance.to_string(), valid);
    }

    assert_eq!(results.get(&valid), Some(&true));
    assert_eq!(results.get(&invalid), Some(&false));
}

#[test]
fn test_errors_only_text_output() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", r#"{"type": "integer"}"#);
    let valid = create_temp_file(&dir, "valid.json", "42");
    let invalid = create_temp_file(&dir, "invalid.json", r#""not an integer""#);

    let mut cmd = cli();
    cmd.arg(&schema)
        .arg("--instance")
        .arg(&valid)
        .arg("--instance")
        .arg(&invalid)
        .arg("--errors-only");
    let output = cmd.output().unwrap();
    assert!(!output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should contain "INVALID"
    assert!(stdout.contains("INVALID"));
    assert!(stdout.contains(&invalid));
    // Should not show the valid file at all (should not contain " - VALID")
    assert!(!stdout.contains(" - VALID"));
}

#[test]
fn test_errors_only_structured_output() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", r#"{"type": "integer"}"#);
    let valid = create_temp_file(&dir, "valid.json", "42");
    let invalid = create_temp_file(&dir, "invalid.json", r#""not an integer""#);

    let mut cmd = cli();
    cmd.arg(&schema)
        .arg("--instance")
        .arg(&valid)
        .arg("--instance")
        .arg(&invalid)
        .arg("--output")
        .arg("flag")
        .arg("--errors-only");
    let output = cmd.output().unwrap();
    assert!(!output.status.success());

    let records = parse_ndjson(&String::from_utf8_lossy(&output.stdout));
    // Should only have 1 record (the invalid one)
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["instance"], invalid);
    assert_eq!(records[0]["payload"]["valid"], false);
}

#[test]
fn test_validate_valid_schema() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", r#"{"type": "string"}"#);

    let mut cmd = cli();
    cmd.arg(&schema);
    let output = cmd.output().unwrap();
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Schema is valid"));
}

#[test]
fn test_validate_invalid_schema() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r#"{"type": "invalid_type", "minimum": "not a number"}"#,
    );

    let mut cmd = cli();
    cmd.arg(&schema);
    let output = cmd.output().unwrap();
    assert!(!output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Schema is invalid"));
}

#[test]
fn test_instance_validation_with_invalid_schema_structured_output() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r#"{"type": "invalid_type", "minimum": "not a number"}"#,
    );
    let instance = create_temp_file(&dir, "instance.json", "42");

    let mut cmd = cli();
    cmd.arg(&schema)
        .arg("--instance")
        .arg(&instance)
        .arg("--output")
        .arg("flag");
    let output = cmd.output().unwrap();
    assert!(!output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("Should be valid JSON");

    assert_eq!(json["output"], "flag");
    assert_eq!(json["payload"]["valid"], false);
    assert!(json["schema"].as_str().unwrap().ends_with("schema.json"));
}

#[test]
fn test_instance_validation_with_invalid_schema_list_output() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r#"{"type": "invalid_type", "minimum": "not a number"}"#,
    );
    let instance = create_temp_file(&dir, "instance.json", "42");

    let mut cmd = cli();
    cmd.arg(&schema)
        .arg("--instance")
        .arg(&instance)
        .arg("--output")
        .arg("list");
    let output = cmd.output().unwrap();
    assert!(!output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("Should be valid JSON");

    assert_eq!(json["output"], "list");
    assert_eq!(json["payload"]["valid"], false);
    assert!(json["schema"].as_str().unwrap().ends_with("schema.json"));
}

#[test]
fn test_instance_validation_with_invalid_schema_hierarchical_output() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r#"{"type": "invalid_type", "minimum": "not a number"}"#,
    );
    let instance = create_temp_file(&dir, "instance.json", "42");

    let mut cmd = cli();
    cmd.arg(&schema)
        .arg("--instance")
        .arg(&instance)
        .arg("--output")
        .arg("hierarchical");
    let output = cmd.output().unwrap();
    assert!(!output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("Should be valid JSON");

    assert_eq!(json["output"], "hierarchical");
    assert_eq!(json["payload"]["valid"], false);
    assert!(json["schema"].as_str().unwrap().ends_with("schema.json"));
}

#[test]
fn test_validate_invalid_schema_list_output() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r#"{"type": "invalid_type", "minimum": "not a number"}"#,
    );

    let mut cmd = cli();
    cmd.arg(&schema).arg("--output").arg("list");
    let output = cmd.output().unwrap();
    assert!(!output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("Should be valid JSON");

    assert_eq!(json["output"], "list");
    assert_eq!(json["payload"]["valid"], false);
    assert!(json["schema"].as_str().unwrap().ends_with("schema.json"));
}

#[test]
fn test_validate_invalid_schema_hierarchical_output() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r#"{"type": "invalid_type", "minimum": "not a number"}"#,
    );

    let mut cmd = cli();
    cmd.arg(&schema).arg("--output").arg("hierarchical");
    let output = cmd.output().unwrap();
    assert!(!output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("Should be valid JSON");

    assert_eq!(json["output"], "hierarchical");
    assert_eq!(json["payload"]["valid"], false);
    assert!(json["schema"].as_str().unwrap().ends_with("schema.json"));
}

#[test]
fn test_validate_schema_with_json_parse_error() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", r#"{"type": "string"#);

    let mut cmd = cli();
    cmd.arg(&schema).arg("--output").arg("flag");
    let output = cmd.output().unwrap();
    assert!(!output.status.success());

    let sanitized = sanitize_output(
        String::from_utf8_lossy(&output.stdout).to_string(),
        &[&schema],
    );
    assert!(sanitized.contains("Error: failed to parse JSON from {FILE_1}:"));
}

#[test]
fn test_validate_instance_with_json_parse_error() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", r#"{"type": "object"}"#);
    let instance = create_temp_file(&dir, "instance.json", "not json");

    let output = cli()
        .arg("validate")
        .arg(&schema)
        .arg("--instance")
        .arg(&instance)
        .output()
        .unwrap();
    assert!(!output.status.success());

    let sanitized = sanitize_output(
        String::from_utf8_lossy(&output.stdout).to_string(),
        &[&instance],
    );
    assert!(sanitized.contains("Error: failed to read JSON from {FILE_1}:"));
}

#[test]
fn test_validate_schema_with_invalid_referenced_schema() {
    // This test verifies that when a schema references another schema via $ref,
    // and that referenced schema is invalid, the validation should fail.
    let dir = tempdir().unwrap();

    // Main schema is structurally valid
    let main_schema = create_temp_file(
        &dir,
        "main.json",
        r#"{
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object",
            "properties": {
                "user": { "$ref": "user.json" }
            }
        }"#,
    );

    // Referenced schema is structurally INVALID (bad type value)
    let _ref_schema = create_temp_file(
        &dir,
        "user.json",
        r#"{
            "type": "invalid_type_here",
            "properties": {
                "name": { "type": "string" }
            }
        }"#,
    );

    let mut cmd = cli();
    cmd.arg(&main_schema);
    let output = cmd.output().unwrap();

    // Schema validation should fail because the referenced schema is invalid
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Schema is invalid"));
}

#[test]
fn test_validate_schema_with_valid_referenced_schema() {
    // This test verifies that when all referenced schemas are valid, validation succeeds.
    let dir = tempdir().unwrap();

    let main_schema = create_temp_file(
        &dir,
        "main.json",
        r#"{
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object",
            "properties": {
                "user": { "$ref": "user.json" }
            }
        }"#,
    );

    // Referenced schema is structurally VALID
    let _ref_schema = create_temp_file(
        &dir,
        "user.json",
        r#"{
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            }
        }"#,
    );

    let mut cmd = cli();
    cmd.arg(&main_schema);
    let output = cmd.output().unwrap();

    // Schema validation should succeed because all schemas are valid
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Schema is valid"));
}

#[test]
fn test_validate_schema_with_invalid_ref_structured_output() {
    // This test verifies structured output when root schema is valid but referenced schema is invalid.
    // This exercises the code path where flag_output.valid is true, but build fails.
    let dir = tempdir().unwrap();

    let main_schema = create_temp_file(
        &dir,
        "main.json",
        r#"{
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object",
            "properties": {
                "user": { "$ref": "user.json" }
            }
        }"#,
    );

    // Referenced schema is structurally INVALID
    let _ref_schema = create_temp_file(
        &dir,
        "user.json",
        r#"{
            "type": "invalid_type_here",
            "properties": {
                "name": { "type": "string" }
            }
        }"#,
    );

    let mut cmd = cli();
    cmd.arg(&main_schema).arg("--output").arg("flag");
    let output = cmd.output().unwrap();

    // Should fail
    assert!(!output.status.success());

    // Should get an error message (not structured output since build fails before we can output)
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Error:"));
}

#[test]
fn test_http_timeout_option() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", r#"{"type": "string"}"#);

    let mut cmd = cli();
    cmd.arg(&schema).arg("--timeout").arg("30");
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Schema is valid"));
}

#[test]
fn test_http_connect_timeout_option() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", r#"{"type": "string"}"#);

    let mut cmd = cli();
    cmd.arg(&schema).arg("--connect-timeout").arg("10");
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Schema is valid"));
}

#[test]
fn test_http_insecure_option() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", r#"{"type": "string"}"#);

    let mut cmd = cli();
    cmd.arg(&schema).arg("--insecure");
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Schema is valid"));
}

#[test]
fn test_http_insecure_short_option() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", r#"{"type": "string"}"#);

    let mut cmd = cli();
    cmd.arg(&schema).arg("-k");
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Schema is valid"));
}

#[test]
fn test_http_all_options_combined() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", r#"{"type": "integer"}"#);
    let instance = create_temp_file(&dir, "instance.json", "42");

    let mut cmd = cli();
    cmd.arg(&schema)
        .arg("--instance")
        .arg(&instance)
        .arg("--timeout")
        .arg("30")
        .arg("--connect-timeout")
        .arg("10")
        .arg("--insecure");
    let output = cmd.output().unwrap();
    assert!(output.status.success());
}

#[test]
fn test_http_invalid_timeout_negative() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", r#"{"type": "string"}"#);

    let mut cmd = cli();
    cmd.arg(&schema).arg("--timeout=-1");
    let output = cmd.output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("non-negative finite"));
}

#[test]
fn test_http_invalid_timeout_not_a_number() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", r#"{"type": "string"}"#);

    let mut cmd = cli();
    cmd.arg(&schema).arg("--timeout").arg("abc");
    let output = cmd.output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not a valid number"));
}

#[test]
fn test_http_invalid_connect_timeout_negative() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", r#"{"type": "string"}"#);

    let mut cmd = cli();
    cmd.arg(&schema).arg("--connect-timeout=-5");
    let output = cmd.output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("non-negative finite"));
}

#[test]
fn test_http_cacert_nonexistent_file() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", r#"{"type": "string"}"#);

    let mut cmd = cli();
    cmd.arg(&schema)
        .arg("--cacert")
        .arg("/nonexistent/path/to/cert.pem");
    let output = cmd.output().unwrap();
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Error:"));
    assert!(stdout.contains("/nonexistent/path/to/cert.pem"));
}

#[test]
fn test_http_options_with_external_ref() {
    // Test that HTTP options are actually applied when fetching external schemas
    let dir = tempdir().unwrap();
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r#"{"$ref": "https://json-schema.org/draft/2020-12/schema"}"#,
    );
    let instance = create_temp_file(&dir, "instance.json", r#"{"type": "string"}"#);

    let mut cmd = cli();
    cmd.arg(&schema)
        .arg("--instance")
        .arg(&instance)
        .arg("--timeout")
        .arg("30");
    let output = cmd.output().unwrap();
    assert!(output.status.success());
}

#[test]
fn test_http_options_ndjson_output() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r#"{"$ref": "https://json-schema.org/draft/2020-12/schema"}"#,
    );

    let mut cmd = cli();
    cmd.arg(&schema)
        .arg("--output")
        .arg("flag")
        .arg("--timeout")
        .arg("30");
    let output = cmd.output().unwrap();
    assert!(output.status.success());
}

#[test]
fn test_bundle_no_external_refs() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", r#"{"type":"string"}"#);

    let output = cli().arg("bundle").arg(&schema).output().unwrap();
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).unwrap();
    let bundled: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(bundled.get("type"), Some(&serde_json::json!("string")));
    assert!(bundled.get("$defs").is_none());
}

#[test]
fn test_bundle_with_resource_flag() {
    let dir = tempdir().unwrap();
    let person = create_temp_file(
        &dir,
        "person.json",
        r#"{"$id":"https://example.com/person.json","type":"object","properties":{"name":{"type":"string"}},"required":["name"]}"#,
    );
    let root = create_temp_file(
        &dir,
        "root.json",
        r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","$ref":"https://example.com/person.json"}"#,
    );

    let resource_arg = format!("https://example.com/person.json={person}");
    let output = cli()
        .arg("bundle")
        .arg(&root)
        .arg("--resource")
        .arg(&resource_arg)
        .output()
        .unwrap();
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).unwrap();
    let bundled: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    // $ref must not be rewritten (spec requirement)
    assert_eq!(
        bundled["$ref"],
        serde_json::json!("https://example.com/person.json")
    );
    // resource must be embedded in $defs
    assert!(bundled["$defs"]["https://example.com/person.json"].is_object());
}

#[test]
fn test_bundle_output_to_file() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", r#"{"type":"integer"}"#);
    let out_path = dir
        .path()
        .join("bundled.json")
        .to_str()
        .unwrap()
        .to_string();

    let output = cli()
        .arg("bundle")
        .arg(&schema)
        .arg("--output")
        .arg(&out_path)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());

    let written = fs::read_to_string(&out_path).unwrap();
    let bundled: serde_json::Value = serde_json::from_str(&written).unwrap();
    assert_eq!(bundled.get("type"), Some(&serde_json::json!("integer")));
}

#[test]
fn test_bundle_supports_legacy_draft() {
    let dir = tempdir().unwrap();
    let person = create_temp_file(
        &dir,
        "person.json",
        r#"{"$schema":"http://json-schema.org/draft-07/schema#","$id":"https://example.com/person.json","type":"string"}"#,
    );
    let root = create_temp_file(
        &dir,
        "root.json",
        r#"{"$schema":"http://json-schema.org/draft-07/schema#","$ref":"https://example.com/person.json"}"#,
    );

    let resource_arg = format!("https://example.com/person.json={person}");
    let output = cli()
        .arg("bundle")
        .arg(&root)
        .arg("--resource")
        .arg(&resource_arg)
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let bundled: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(
        bundled.get("$defs").is_none(),
        "legacy bundle should not use $defs"
    );
    assert!(bundled["definitions"]["https://example.com/person.json"].is_object());
}

#[test]
fn test_validate_subcommand_explicit() {
    // `jsonschema validate schema.json -i instance.json` — new explicit subcommand form
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", r#"{"type":"string"}"#);
    let instance = create_temp_file(&dir, "instance.json", r#""hello""#);

    let output = cli()
        .arg("validate")
        .arg(&schema)
        .arg("-i")
        .arg(&instance)
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("VALID"));
}

#[test]
fn test_flat_invocation_still_works() {
    // Flat invocation (deprecated) must still run and emit a deprecation warning
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", r#"{"type":"string"}"#);
    let instance = create_temp_file(&dir, "instance.json", "42");

    let output = cli()
        .arg(&schema)
        .arg("-i")
        .arg(&instance)
        .output()
        .unwrap();
    // Exit code 1 because instance is invalid
    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("INVALID"));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("deprecated"),
        "expected deprecation warning on stderr"
    );
}

#[test]
fn test_no_args_prints_help_hint_and_fails() {
    let output = cli().output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("A schema argument is required"),
        "expected usage hint on stderr, got: {stderr}"
    );
}

#[test]
fn test_bundle_bad_schema_file_prints_error_on_stderr() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", "not-json{{");
    let output = cli().arg("bundle").arg(&schema).output().unwrap();
    assert!(!output.status.success());
    let sanitized = sanitize_output(String::from_utf8(output.stderr).unwrap(), &[&schema]);
    assert!(sanitized.contains("error: failed to parse JSON from {FILE_1}:"));
}

#[test]
fn test_bundle_missing_schema_file_prints_error_on_stderr() {
    let dir = tempdir().unwrap();
    let missing = dir.path().join("missing-schema.json");
    let missing = missing.to_string_lossy().to_string();
    let output = cli().arg("bundle").arg(&missing).output().unwrap();
    assert!(!output.status.success());
    let sanitized = sanitize_output(String::from_utf8(output.stderr).unwrap(), &[&missing]);
    assert!(sanitized.contains("error: failed to read {FILE_1}:"));
}

#[test]
fn test_dereference_no_external_refs() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", r#"{"type":"string"}"#);

    let output = cli().arg("dereference").arg(&schema).output().unwrap();
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).unwrap();
    let dereferenced: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(dereferenced, serde_json::json!({"type": "string"}));
}

#[test]
fn test_dereference_with_resource_flag() {
    let dir = tempdir().unwrap();
    let address = create_temp_file(
        &dir,
        "address.json",
        r#"{"$id":"https://example.com/address.json","type":"object","properties":{"street":{"type":"string"}},"required":["street"]}"#,
    );
    let root = create_temp_file(
        &dir,
        "root.json",
        r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","$ref":"https://example.com/address.json"}"#,
    );

    let resource_arg = format!("https://example.com/address.json={address}");
    let output = cli()
        .arg("dereference")
        .arg(&root)
        .arg("--resource")
        .arg(&resource_arg)
        .output()
        .unwrap();
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).unwrap();
    let dereferenced: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    // $ref should be replaced with the inlined content; $schema sibling is merged in
    assert_eq!(
        dereferenced,
        serde_json::json!({
            "$id": "https://example.com/address.json",
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {"street": {"type": "string"}},
            "required": ["street"]
        })
    );
}

#[test]
fn test_dereference_output_to_file() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", r#"{"type":"integer"}"#);
    let out_path = dir
        .path()
        .join("dereferenced.json")
        .to_str()
        .unwrap()
        .to_string();

    let output = cli()
        .arg("dereference")
        .arg(&schema)
        .arg("--output")
        .arg(&out_path)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());

    let written = fs::read_to_string(&out_path).unwrap();
    let dereferenced: serde_json::Value = serde_json::from_str(&written).unwrap();
    assert_eq!(dereferenced, serde_json::json!({"type": "integer"}));
}

#[test]
fn test_dereference_bad_schema_file_prints_error_on_stderr() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", "not-json{{");
    let output = cli().arg("dereference").arg(&schema).output().unwrap();
    assert!(!output.status.success());
    let sanitized = sanitize_output(String::from_utf8(output.stderr).unwrap(), &[&schema]);
    assert!(sanitized.contains("error: failed to parse JSON from {FILE_1}:"));
}

#[test]
fn test_dereference_missing_schema_file_prints_error_on_stderr() {
    let dir = tempdir().unwrap();
    let missing = dir.path().join("missing-schema.json");
    let missing = missing.to_string_lossy().to_string();
    let output = cli().arg("dereference").arg(&missing).output().unwrap();
    assert!(!output.status.success());
    let sanitized = sanitize_output(String::from_utf8(output.stderr).unwrap(), &[&missing]);
    assert!(sanitized.contains("error: failed to read {FILE_1}:"));
}

#[test]
fn test_dereference_unresolvable_ref_prints_error_on_stderr() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","$ref":"https://example.com/does-not-exist.json"}"#,
    );
    let output = cli().arg("dereference").arg(&schema).output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("does-not-exist.json"),
        "expected URI in error, got: {stderr}"
    );
}

#[test]
fn test_dereference_missing_resource_file_prints_error_on_stderr() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","$ref":"https://example.com/ext.json"}"#,
    );
    let missing = dir.path().join("missing.json");
    let missing_str = missing.to_str().unwrap().to_string();
    let resource_arg = format!("https://example.com/ext.json={missing_str}");
    let output = cli()
        .arg("dereference")
        .arg(&schema)
        .arg("--resource")
        .arg(&resource_arg)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let sanitized = sanitize_output(String::from_utf8(output.stderr).unwrap(), &[&missing_str]);
    assert!(sanitized.contains("error: failed to read {FILE_1}:"));
}

#[test]
fn test_dereference_with_insecure_flag_succeeds() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", r#"{"type":"string"}"#);
    let output = cli()
        .arg("dereference")
        .arg(&schema)
        .arg("--insecure")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(result, serde_json::json!({"type": "string"}));
}

#[test]
fn test_dereference_invalid_resource_uri_prints_error_on_stderr() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", r#"{"type":"string"}"#);
    let resource = create_temp_file(&dir, "resource.json", r#"{"type":"number"}"#);
    let output = cli()
        .arg("dereference")
        .arg(&schema)
        .arg("--resource")
        .arg(format!(":::invalid={resource}"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.is_empty(), "expected error on stderr");
}

#[test]
fn test_dereference_resource_with_transitive_unresolvable_ref_prints_error_on_stderr() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","$ref":"https://example.com/middle.json"}"#,
    );
    let middle = create_temp_file(
        &dir,
        "middle.json",
        r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://example.com/middle.json","$ref":"https://example.com/leaf.json"}"#,
    );
    let output = cli()
        .arg("dereference")
        .arg(&schema)
        .arg("--resource")
        .arg(format!("https://example.com/middle.json={middle}"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.is_empty(), "expected error on stderr");
}
