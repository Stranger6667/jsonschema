use assert_cmd::{cargo::cargo_bin_cmd, Command};
use insta::assert_snapshot;
use serde_json::Value;
use std::{collections::HashMap, fs};
use tempfile::tempdir;
use test_case::test_case;

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
fn test_offline_refuses_remote_reference() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r#"{"$ref": "https://example.com/schema.json"}"#,
    );
    let instance = create_temp_file(&dir, "instance.json", "{}");

    let mut cmd = cli();
    cmd.arg("validate")
        .arg("--offline")
        .arg(&schema)
        .arg("--instance")
        .arg(&instance);
    let output = cmd.output().unwrap();
    assert!(!output.status.success());
    let sanitized = sanitize_output(
        String::from_utf8_lossy(&output.stdout).to_string(),
        &[&schema, &instance],
    );
    assert_snapshot!(sanitized);
}

// `--offline` refuses only what is not registered; a `--resource` still resolves.
#[test]
fn test_offline_resolves_a_registered_resource() {
    let dir = tempdir().unwrap();
    let resource = create_temp_file(&dir, "person.json", r#"{"type": "object"}"#);
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r#"{"$ref": "https://example.com/person.json"}"#,
    );

    let mut cmd = cli();
    cmd.arg("bundle")
        .arg("--offline")
        .arg(&schema)
        .arg("--resource")
        .arg(format!("https://example.com/person.json={resource}"));
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let bundled: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        bundled["$defs"]["https://example.com/person.json"]["type"],
        serde_json::json!("object")
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

#[test]
fn test_canonicalize_basic() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", r#"{"type":"string","minLength":3}"#);

    let output = cli().arg("canonicalize").arg(&schema).output().unwrap();
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).unwrap();
    let canonical: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        canonical,
        serde_json::json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "string",
            "minLength": 3
        })
    );
}

#[test]
fn test_canonicalize_output_to_file() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", r#"{"type":"integer"}"#);
    let out_path = dir.path().join("canonical.json");

    let output = cli()
        .arg("canonicalize")
        .arg(&schema)
        .arg("--output")
        .arg(&out_path)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());

    let written = fs::read_to_string(&out_path).unwrap();
    let canonical: Value = serde_json::from_str(&written).unwrap();
    assert_eq!(
        canonical,
        serde_json::json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "type": "integer"})
    );
}

#[test]
fn test_canonicalize_reads_yaml() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(
        &dir,
        "schema.yaml",
        "allOf:\n  - {type: string, minLength: 2}\n  - {maxLength: 5}\n",
    );

    let output = cli().arg("canonicalize").arg(&schema).output().unwrap();
    assert!(output.status.success());

    let canonical: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        canonical,
        serde_json::json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "string",
            "minLength": 2,
            "maxLength": 5
        })
    );
}

#[test]
fn test_canonicalize_invalid_schema_errors() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", r#"{"type":123}"#);

    let output = cli().arg("canonicalize").arg(&schema).output().unwrap();
    assert!(!output.status.success());

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.is_empty(), "expected error on stderr");
}

const PET_DOCUMENT: &str = r##"{
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "$defs": {
        "Named": {"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]},
        "Pet": {"allOf": [
            {"$ref": "#/$defs/Named"},
            {"type": "object", "properties": {"age": {"type": "integer", "minimum": 0}}}
        ]},
        "Tree": {"type": "object", "properties": {
            "kids": {"type": "array", "items": {"$ref": "#/$defs/Tree"}},
            "leaf": {"$ref": "#/$defs/Named"}
        }}
    },
    "type": "object",
    "properties": {"pet": {"$ref": "#/$defs/Pet"}}
}"##;

fn canonicalize_at(schema: &str, pointer: &str) -> Value {
    let output = cli()
        .arg("canonicalize")
        .arg(schema)
        .arg("--at")
        .arg(pointer)
        .output()
        .unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(output.status.success(), "{stderr}");
    serde_json::from_str(&String::from_utf8(output.stdout).unwrap()).unwrap()
}

// A `$ref` is usually copied out of the document with its leading `#`.
#[test_case("/$defs/Pet"; "pointer")]
#[test_case("#/$defs/Pet"; "pasted reference")]
#[test_case("/properties/pet"; "through a reference of its own")]
fn test_canonicalize_at_resolves_document_references(pointer: &str) {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", PET_DOCUMENT);

    assert_eq!(
        canonicalize_at(&schema, pointer),
        serde_json::json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer", "minimum": 0}
            },
            "required": ["name"]
        })
    );
}

#[test]
fn test_canonicalize_at_names_recursive_definitions_readably() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", PET_DOCUMENT);

    // The selection is the root of what comes back, so its own recursion spells `#`.
    let canonical = canonicalize_at(&schema, "/$defs/Tree");
    assert_eq!(
        canonical["properties"]["kids"]["items"],
        serde_json::json!({"$ref": "#"})
    );
    assert_eq!(
        canonical["$defs"]["Named"],
        serde_json::json!({
            "type": "object",
            "properties": {"name": {"type": "string"}},
            "required": ["name"]
        })
    );
    // Targets reached through the registry carry percent-encoded URIs until renamed.
    assert!(
        !canonical.to_string().contains('%'),
        "encoded URIs leaked into the output: {canonical}"
    );
}

#[test]
fn test_canonicalize_at_reports_a_contradiction() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r#"{"$defs": {"Dead": {"allOf": [{"type": "integer", "minimum": 5}, {"maximum": 3}]}}}"#,
    );

    assert_eq!(
        canonicalize_at(&schema, "/$defs/Dead"),
        serde_json::json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "not": {}
        })
    );
}

#[test]
fn test_canonicalize_at_root_matches_the_whole_document() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", PET_DOCUMENT);

    let whole = cli().arg("canonicalize").arg(&schema).output().unwrap();
    assert!(whole.status.success());
    assert_eq!(
        canonicalize_at(&schema, ""),
        serde_json::from_slice::<Value>(&whole.stdout).unwrap()
    );
}

#[test]
fn test_canonicalize_at_resolves_references_of_an_identified_document() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r##"{"$id": "https://example.com/pets.json", "$defs": {
            "Short": {"type": "string", "maxLength": 5},
            "Word": {"allOf": [{"$ref": "#/$defs/Short"}, {"minLength": 2}]}
        }}"##,
    );

    assert_eq!(
        canonicalize_at(&schema, "/$defs/Word"),
        serde_json::json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "string",
            "minLength": 2,
            "maxLength": 5
        })
    );
}

// The selection wrapper carries no `$schema`, so the draft has to come from the document: under
// the latest draft a draft-7 document's `definitions` are not a reference target at all.
#[test_case(r#"{"$schema": "http://json-schema.org/draft-07/schema#"}"#; "draft 7")]
#[test_case(r##"{"$schema": "http://json-schema.org/draft-07/schema#", "$id": "#local"}"##; "draft 7 with a fragment-only id")]
#[test_case(r#"{"$schema": "http://json-schema.org/draft-04/schema#", "id": "https://example.com/legacy.json"}"#; "draft 4 with a legacy id")]
fn test_canonicalize_at_resolves_references_of_older_drafts(header: &str) {
    let dir = tempdir().unwrap();
    let mut document: Value = serde_json::from_str(header).unwrap();
    document["definitions"] = serde_json::json!({
        "Short": {"type": "string", "maxLength": 5},
        "Word": {"allOf": [{"$ref": "#/definitions/Short"}, {"minLength": 2}]}
    });
    let schema = create_temp_file(&dir, "schema.json", &document.to_string());

    let canonical = canonicalize_at(&schema, "/definitions/Word");
    assert_eq!(canonical["type"], serde_json::json!("string"));
    assert_eq!(canonical["minLength"], serde_json::json!(2));
    assert_eq!(canonical["maxLength"], serde_json::json!(5));
}

#[test]
fn test_canonicalize_at_suffixes_colliding_definition_names() {
    let dir = tempdir().unwrap();
    // Two recursive targets whose locations end in the same segment: both want the name `Pet`.
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r##"{
            "$defs": {"Pet": {"type": "object", "properties": {"next": {"$ref": "#/$defs/Pet"}}}},
            "components": {"schemas": {"Pet": {
                "type": "object", "properties": {"next": {"$ref": "#/components/schemas/Pet"}}
            }}},
            "Target": {"allOf": [
                {"$ref": "#/$defs/Pet"}, {"$ref": "#/components/schemas/Pet"}
            ]}
        }"##,
    );

    let canonical = canonicalize_at(&schema, "/Target");
    let names: Vec<&String> = canonical["$defs"].as_object().unwrap().keys().collect();
    assert_eq!(names, ["Pet", "Pet_2"]);
    assert!(!canonical.to_string().contains('%'), "{canonical}");
}

// Generated definitions land under whichever keyword the draft uses, so both get renamed.
#[test]
fn test_canonicalize_at_names_definitions_readably_in_older_drafts() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r##"{
            "$schema": "http://json-schema.org/draft-07/schema#",
            "components": {"schemas": {"Tree": {
                "type": "object", "properties": {"next": {"$ref": "#/components/schemas/Tree"}}
            }}},
            "Target": {"allOf": [
                {"$ref": "#/components/schemas/Tree"}, {"type": "object"}
            ]}
        }"##,
    );

    let canonical = canonicalize_at(&schema, "/Target");
    assert!(canonical["definitions"]["Tree"].is_object(), "{canonical}");
    assert!(
        !canonical.to_string().contains('%'),
        "encoded URIs leaked into the output: {canonical}"
    );
}

#[test]
fn test_canonicalize_at_honours_the_draft_flag() {
    let dir = tempdir().unwrap();
    // No `$schema` to detect, so only `--draft` can say that `id` is an identifier here.
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r##"{"id": "https://example.com/legacy.json", "definitions": {
            "Short": {"type": "string", "maxLength": 5},
            "Word": {"allOf": [{"$ref": "#/definitions/Short"}, {"minLength": 2}]}
        }}"##,
    );

    let output = cli()
        .args(["canonicalize"])
        .arg(&schema)
        .args(["--at", "/definitions/Word", "-d", "4"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let canonical: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(canonical["type"], serde_json::json!("string"));
    assert_eq!(canonical["minLength"], serde_json::json!(2));
    assert_eq!(canonical["maxLength"], serde_json::json!(5));
}

// A schema the canonical form cannot model is passed through, the same way a whole document is.
#[test]
fn test_canonicalize_at_passes_an_unmodelled_selection_through() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(
        &dir,
        "schema.json",
        r##"{"$defs": {
            "A": {"dependencies": {}, "unevaluatedProperties": false, "$ref": "#/$defs/B"},
            "B": {"type": "object"}
        }}"##,
    );

    assert_eq!(
        canonicalize_at(&schema, "/$defs/A"),
        serde_json::json!({"dependencies": {}, "unevaluatedProperties": false, "$ref": "#/$defs/B"})
    );
}

#[test_case(
    r#"{"$id": "http://[bad", "$defs": {"A": {"type": "string"}}}"#,
    "Invalid URI reference";
    "malformed root id"
)]
#[test_case(
    r#"{"$defs": {"A": {"$ref": "https://example.invalid/nope.json"}}}"#,
    "is not present in a registry";
    "unretrievable reference"
)]
fn test_canonicalize_at_reports_a_document_that_cannot_be_registered(
    document: &str,
    expected: &str,
) {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", document);

    let output = cli()
        .arg("canonicalize")
        .arg(&schema)
        .args(["--at", "/$defs/A"])
        .output()
        .unwrap();
    assert!(!output.status.success());

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains(expected), "got: {stderr}");
}

#[test]
fn test_canonicalize_at_reads_yaml() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(
        &dir,
        "openapi.yaml",
        "components:\n  schemas:\n    Named:\n      type: object\n      required: [name]\n      properties: {name: {type: string}}\n    Pet:\n      allOf:\n        - $ref: '#/components/schemas/Named'\n        - properties: {age: {type: integer, minimum: 0}}\n",
    );

    assert_eq!(
        canonicalize_at(&schema, "/components/schemas/Pet"),
        serde_json::json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer", "minimum": 0}
            },
            "required": ["name"]
        })
    );
}

// Which segment is wrong is the whole question when a pointer is typed by hand.
#[test_case("/$defs/Pett", "'/$defs' has no 'Pett'"; "mistyped leaf")]
#[test_case("/property/pet", "the document root has no 'property'"; "mistyped root member")]
#[test_case("$defs/Pet", "is not a JSON Pointer"; "missing leading slash")]
#[test_case("/$defs/Named/required/0", "is of type string, not a schema"; "not a schema")]
fn test_canonicalize_at_rejects_a_bad_pointer(pointer: &str, expected: &str) {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", PET_DOCUMENT);

    let output = cli()
        .arg("canonicalize")
        .arg(&schema)
        .arg("--at")
        .arg(pointer)
        .output()
        .unwrap();
    assert!(!output.status.success());

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains(expected), "got: {stderr}");
}

const NAME_SCHEMA: &str = r#"{"type": "object", "properties": {"name": {"type": "string"}}}"#;

#[test]
fn test_self_describing_valid_instance() {
    let dir = tempdir().unwrap();
    create_temp_file(&dir, "schema.json", NAME_SCHEMA);
    let instance = create_temp_file(
        &dir,
        "instance.json",
        r#"{"$schema": "./schema.json", "name": "John Doe"}"#,
    );

    let output = cli()
        .arg("validate")
        .arg("-i")
        .arg(&instance)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("VALID"));
}

#[test]
fn test_self_describing_invalid_instance() {
    let dir = tempdir().unwrap();
    create_temp_file(&dir, "schema.json", NAME_SCHEMA);
    let instance = create_temp_file(
        &dir,
        "instance.json",
        r#"{"$schema": "./schema.json", "name": 123}"#,
    );

    let output = cli()
        .arg("validate")
        .arg("-i")
        .arg(&instance)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("INVALID. Errors:"), "{stdout}");
}

#[test]
fn test_self_describing_relative_to_instance_not_cwd() {
    let dir = tempdir().unwrap();
    create_temp_file(
        &dir,
        "schema.json",
        r#"{"type": "object", "properties": {"name": {"type": "number"}}}"#,
    );

    let nested = dir.path().join("nested");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("schema.json"), NAME_SCHEMA).unwrap();
    let instance = nested.join("instance.json");
    fs::write(
        &instance,
        r#"{"$schema": "./schema.json", "name": "John Doe"}"#,
    )
    .unwrap();

    let output = cli()
        .current_dir(dir.path())
        .arg("validate")
        .arg("-i")
        .arg(&instance)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "resolved against the CWD instead of the instance: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn test_self_describing_missing_schema_property() {
    let dir = tempdir().unwrap();
    let instance = create_temp_file(&dir, "instance.json", r#"{"name": "John Doe"}"#);

    let output = cli()
        .arg("validate")
        .arg("-i")
        .arg(&instance)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("no `$schema` property"), "{stdout}");
    assert!(stdout.contains("jsonschema validate SCHEMA -i"), "{stdout}");
}

#[test]
fn test_self_describing_non_object_instance() {
    let dir = tempdir().unwrap();
    let instance = create_temp_file(&dir, "instance.json", "[1, 2, 3]");

    let output = cli()
        .arg("validate")
        .arg("-i")
        .arg(&instance)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("no `$schema` property"));
}

/// One unusable instance must not stop the rest of the batch.
#[test]
fn test_self_describing_mixed_batch() {
    let dir = tempdir().unwrap();
    create_temp_file(&dir, "schema.json", NAME_SCHEMA);
    let good = create_temp_file(
        &dir,
        "good.json",
        r#"{"$schema": "./schema.json", "name": "John Doe"}"#,
    );
    let bad = create_temp_file(&dir, "bad.json", r#"{"name": "John Doe"}"#);

    let output = cli()
        .arg("validate")
        .arg("-i")
        .arg(&bad)
        .arg(&good)
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "a missing $schema must fail the run"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("no `$schema` property"), "{stdout}");
    assert!(stdout.contains(&format!("{good} - VALID")), "{stdout}");
}

#[test]
fn test_explicit_schema_overrides_instance_schema() {
    let dir = tempdir().unwrap();
    let explicit = create_temp_file(&dir, "explicit.json", NAME_SCHEMA);
    // Would reject the instance if it were ever consulted.
    create_temp_file(
        &dir,
        "other.json",
        r#"{"type": "object", "properties": {"name": {"type": "number"}}}"#,
    );
    let instance = create_temp_file(
        &dir,
        "instance.json",
        r#"{"$schema": "./other.json", "name": "John Doe"}"#,
    );

    let output = cli()
        .arg("validate")
        .arg(&explicit)
        .arg("-i")
        .arg(&instance)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "instance `$schema` must be ignored when SCHEMA is given: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn test_self_describing_metaschema_is_offline() {
    let dir = tempdir().unwrap();
    let valid = create_temp_file(
        &dir,
        "valid-schema.json",
        r#"{"$schema": "https://json-schema.org/draft/2020-12/schema", "type": "string"}"#,
    );
    let output = cli()
        .arg("validate")
        .arg("-i")
        .arg(&valid)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );

    let invalid = create_temp_file(
        &dir,
        "invalid-schema.json",
        r#"{"$schema": "https://json-schema.org/draft/2020-12/schema", "type": 42}"#,
    );
    let output = cli()
        .arg("validate")
        .arg("-i")
        .arg(&invalid)
        .output()
        .unwrap();
    assert!(!output.status.success());
}

/// The validator is compiled once and reused across both.
#[test]
fn test_self_describing_shared_schema() {
    let dir = tempdir().unwrap();
    create_temp_file(&dir, "schema.json", NAME_SCHEMA);
    let first = create_temp_file(
        &dir,
        "first.json",
        r#"{"$schema": "./schema.json", "name": "John"}"#,
    );
    let second = create_temp_file(
        &dir,
        "second.json",
        r#"{"$schema": "./schema.json", "name": 123}"#,
    );

    let output = cli()
        .arg("validate")
        .arg("-i")
        .arg(&first)
        .arg(&second)
        .arg("--output")
        .arg("flag")
        .output()
        .unwrap();
    assert!(!output.status.success());

    let records = parse_ndjson(&String::from_utf8_lossy(&output.stdout));
    assert_eq!(records.len(), 2);
    let by_instance: HashMap<_, _> = records
        .iter()
        .map(|record| {
            (
                record["instance"].as_str().unwrap().to_string(),
                record["payload"]["valid"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(by_instance.get(&first), Some(&true));
    assert_eq!(by_instance.get(&second), Some(&false));
    // Both resolved to the same schema URI.
    assert_eq!(records[0]["schema"], records[1]["schema"]);
    assert!(records[0]["schema"]
        .as_str()
        .unwrap()
        .starts_with("file://"));
}

/// In particular `evaluationPath` must not gain a `/$ref` hop.
#[test]
fn test_self_describing_output_matches_explicit_schema() {
    let dir = tempdir().unwrap();
    let schema = create_temp_file(&dir, "schema.json", NAME_SCHEMA);
    let instance = create_temp_file(
        &dir,
        "instance.json",
        r#"{"$schema": "./schema.json", "name": 123}"#,
    );

    let run = |args: Vec<&str>| {
        let output = cli().args(args).output().unwrap();
        let records = parse_ndjson(&String::from_utf8_lossy(&output.stdout));
        assert_eq!(records.len(), 1);
        records[0]["payload"].clone()
    };

    let explicit = run(vec![
        "validate", &schema, "-i", &instance, "--output", "list",
    ]);
    let self_describing = run(vec!["validate", "-i", &instance, "--output", "list"]);

    assert_eq!(explicit, self_describing);
}

#[test]
fn test_self_describing_yaml_instance() {
    let dir = tempdir().unwrap();
    create_temp_file(&dir, "schema.json", NAME_SCHEMA);
    let instance = create_temp_file(
        &dir,
        "instance.yaml",
        "$schema: ./schema.json\nname: John Doe\n",
    );

    let output = cli()
        .arg("validate")
        .arg("-i")
        .arg(&instance)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn test_validate_without_schema_or_instance() {
    let output = cli().arg("validate").output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--instance"), "{stderr}");
}

/// The pointed-at subschema `$ref`s a sibling `$defs` entry, so it must still see the document.
#[test]
fn test_self_describing_schema_pointer_fragment() {
    let dir = tempdir().unwrap();
    create_temp_file(
        &dir,
        "defs.json",
        r##"{"$defs": {
            "Name": {"type": "object", "properties": {"name": {"$ref": "#/$defs/Str"}}},
            "Str": {"type": "string"}
        }}"##,
    );
    let valid = create_temp_file(
        &dir,
        "valid.json",
        r#"{"$schema": "./defs.json#/$defs/Name", "name": "John"}"#,
    );
    let invalid = create_temp_file(
        &dir,
        "invalid.json",
        r#"{"$schema": "./defs.json#/$defs/Name", "name": 123}"#,
    );

    let output = cli()
        .arg("validate")
        .arg("-i")
        .arg(&valid)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );

    let output = cli()
        .arg("validate")
        .arg("-i")
        .arg(&invalid)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains(r#"is not of type "string""#));
}

#[test]
fn test_self_describing_missing_schema_structured_output() {
    let dir = tempdir().unwrap();
    let instance = create_temp_file(&dir, "instance.json", r#"{"name": "John Doe"}"#);

    let output = cli()
        .arg("validate")
        .arg("-i")
        .arg(&instance)
        .arg("--output")
        .arg("flag")
        .output()
        .unwrap();
    assert!(!output.status.success());

    let records = parse_ndjson(&String::from_utf8_lossy(&output.stdout));
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["output"], "flag");
    assert_eq!(records[0]["instance"], instance);
    assert!(
        records[0]["error"]
            .as_str()
            .unwrap()
            .contains("no `$schema` property"),
        "{:?}",
        records[0]
    );
    assert!(records[0]["payload"].is_null());
}

#[test]
fn test_self_describing_unresolvable_schema() {
    let dir = tempdir().unwrap();
    create_temp_file(&dir, "schema.json", NAME_SCHEMA);
    let broken = create_temp_file(
        &dir,
        "broken.json",
        r#"{"$schema": "./missing.json", "name": "John"}"#,
    );
    let good = create_temp_file(
        &dir,
        "good.json",
        r#"{"$schema": "./schema.json", "name": "John"}"#,
    );

    let output = cli()
        .arg("validate")
        .arg("-i")
        .arg(&broken)
        .arg(&good)
        .output()
        .unwrap();
    assert!(!output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("failed to retrieve"), "{stdout}");
    assert!(stdout.contains(&format!("{good} - VALID")), "{stdout}");
}

#[test]
fn test_self_describing_honors_draft_flag() {
    let dir = tempdir().unwrap();
    create_temp_file(
        &dir,
        "schema.json",
        r#"{
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "propertyNames": {"pattern": "^(\\$schema|a)"}
        }"#,
    );
    let instance = create_temp_file(
        &dir,
        "instance.json",
        r#"{"$schema": "./schema.json", "foo": 1}"#,
    );

    let output = cli()
        .arg("validate")
        .arg("-i")
        .arg(&instance)
        .output()
        .unwrap();
    assert!(!output.status.success(), "2020-12 enforces propertyNames");

    let output = cli()
        .arg("validate")
        .arg("-i")
        .arg(&instance)
        .arg("-d")
        .arg("4")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Draft 4 ignores propertyNames: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn test_self_describing_honors_assert_format() {
    let dir = tempdir().unwrap();
    create_temp_file(
        &dir,
        "schema.json",
        r#"{"type": "object", "properties": {"when": {"type": "string", "format": "date"}}}"#,
    );
    let instance = create_temp_file(
        &dir,
        "instance.json",
        r#"{"$schema": "./schema.json", "when": "not-a-date"}"#,
    );

    let output = cli()
        .arg("validate")
        .arg("-i")
        .arg(&instance)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "`format` is an annotation by default"
    );

    let output = cli()
        .arg("validate")
        .arg("-i")
        .arg(&instance)
        .arg("--assert-format")
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "--assert-format enforces `format`"
    );
}

/// HTTP options are threaded through even when the resolved `$schema` is a local file.
#[test]
fn test_self_describing_with_http_options() {
    let dir = tempdir().unwrap();
    create_temp_file(&dir, "schema.json", NAME_SCHEMA);
    let instance = create_temp_file(
        &dir,
        "instance.json",
        r#"{"$schema": "./schema.json", "name": 123}"#,
    );

    let output = cli()
        .arg("validate")
        .arg("-i")
        .arg(&instance)
        .arg("--timeout")
        .arg("30")
        .arg("--connect-timeout")
        .arg("5")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("INVALID. Errors:"));
}

const MADE_UP_VOCABULARY_DIALECT: &str = r#"{"$schema": "https://json-schema.org/draft/2020-12/schema", "$vocabulary": {"https://json-schema.org/draft/2020-12/vocab/core": true, "https://json-schema.org/draft/2020-12/vocab/validation": true, "https://example.com/vocab/made-up": true}}"#;

// Tempdir paths carry no characters that need percent-encoding.
fn file_uri(path: &str) -> String {
    let path = path.replace('\\', "/");
    if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{path}")
    }
}

// `{schema}` and `{instance}` are substituted with the temp file paths.
#[test_case(&["{schema}", "-i", "{instance}"], "vocabulary_undeclared_with_instance" ; "schema with instance")]
#[test_case(&["{schema}"], "vocabulary_undeclared_schema_only" ; "schema only")]
#[test_case(&["-i", "{instance}"], "vocabulary_undeclared_self_describing" ; "self-describing instance")]
fn test_vocabulary_flag_declares_support(args: &[&str], snapshot: &str) {
    let dir = tempdir().unwrap();
    let dialect = create_temp_file(&dir, "dialect.json", MADE_UP_VOCABULARY_DIALECT);
    let schema = create_temp_file(
        &dir,
        "schema.json",
        &format!(
            r#"{{"$schema": "{}", "type": "object"}}"#,
            file_uri(&dialect)
        ),
    );
    let instance = create_temp_file(
        &dir,
        "instance.json",
        r#"{"$schema": "./schema.json", "name": "text"}"#,
    );
    let args: Vec<String> = args
        .iter()
        .map(|arg| {
            arg.replace("{schema}", &schema)
                .replace("{instance}", &instance)
        })
        .collect();

    let output = cli().arg("validate").args(&args).output().unwrap();
    assert!(!output.status.success());
    let out = sanitize_output(
        String::from_utf8_lossy(&output.stdout).to_string(),
        &[&schema, &instance],
    );
    assert_snapshot!(snapshot, out);

    let output = cli()
        .arg("validate")
        .args(&args)
        .arg("--vocabulary")
        .arg("https://example.com/vocab/made-up")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}
