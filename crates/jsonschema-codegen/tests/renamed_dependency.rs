use std::{fs, path::Path, process::Command};

/// In this crate's own test targets `jsonschema` is always in the extern prelude,
/// masking unresolved bare `jsonschema::` paths in generated code.
#[test]
fn works_with_renamed_dependency() {
    let scratch = Path::new(env!("CARGO_TARGET_TMPDIR")).join("renamed-dependency-probe");
    fs::create_dir_all(scratch.join("src")).unwrap();

    // `canonicalize` yields a `\\?\` UNC path on Windows, which Cargo rejects in `path =`.
    let jsonschema_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("jsonschema");
    fs::write(
        scratch.join("Cargo.toml"),
        format!(
            r#"[package]
name = "renamed-dependency-probe"
version = "0.0.0"
edition = "2021"

[dependencies]
js = {{ package = "jsonschema", path = "{}", default-features = false, features = ["macros"] }}
serde_json = "1"

[workspace]
"#,
            jsonschema_dir.display().to_string().replace('\\', "/"),
        ),
    )
    .unwrap();
    fs::write(
        scratch.join("src/lib.rs"),
        r##"#[js::validator(schema = r#"{"type":"string","minLength":2}"#)]
pub struct Probe;

pub fn check(instance: &serde_json::Value) -> Result<(), js::ValidationError<'_>> {
    assert_eq!(Probe::is_valid(instance), Probe::validate(instance).is_ok());
    Probe::validate(instance)
}
"##,
    )
    .unwrap();

    let target_dir = Path::new(env!("CARGO_TARGET_TMPDIR")).parent().unwrap();
    let output = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .arg("check")
        .current_dir(&scratch)
        .env("CARGO_TARGET_DIR", target_dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "probe with renamed `jsonschema` dependency failed to compile:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
}
