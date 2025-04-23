use assert_cmd::Command;
use insta::assert_snapshot;
use std::fs;
use tempfile::tempdir;

fn cli() -> Command {
    Command::cargo_bin("jsonschema-cli").unwrap()
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
