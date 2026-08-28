#![cfg(feature = "jsonb")]
// Reports the opt-in skip the way the other live test does.
#![allow(clippy::print_stderr)]

use std::{
    env,
    fmt::Write as _,
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use jsonschema::{json::Jsonb, Draft};
use jsonschema_value::jsonb_encode::{decode_hex, encode, strip_varlena};
use serde_json::Value;

const PG_URL_VAR: &str = "JSONB_LIVE_PG_URL";

// Above this a `STORAGE PLAIN` row no longer fits on one heap page, which is how the bytes are
// read back out.
const MAX_PLAIN_BYTES: usize = 7000;

// Dollar-quoting keeps the instance out of the SQL grammar; no suite instance contains this.
const QUOTE_TAG: &str = "$jsonbsuite$";

const DRAFTS: [(&str, Draft); 5] = [
    ("draft4", Draft::Draft4),
    ("draft6", Draft::Draft6),
    ("draft7", Draft::Draft7),
    ("draft2019-09", Draft::Draft201909),
    ("draft2020-12", Draft::Draft202012),
];

// Postgres `jsonb` cannot hold these, so the suite's instance never reaches a real column.
fn representable(instance: &Value) -> bool {
    match instance {
        // A `numeric` has no infinities and no NaN, and `jsonb` rejects a NUL byte in a string.
        Value::Number(number) => number.as_f64().is_none_or(f64::is_finite),
        Value::String(string) => !string.contains('\0'),
        Value::Array(items) => items.iter().all(representable),
        Value::Object(members) => members
            .iter()
            .all(|(key, value)| !key.contains('\0') && representable(value)),
        _ => true,
    }
}

fn json_files(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut directories = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                directories.push(path);
            } else if file_type.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension == "json")
            {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn case_files(draft: &str) -> Vec<PathBuf> {
    let root = Path::new("tests/suite/tests").join(draft);
    json_files(&root).unwrap_or_else(|error| {
        panic!(
            "failed to read suite at {}: {error}; initialize the test-suite submodule",
            root.display()
        )
    })
}

fn relative_case_path<'a>(root: &Path, path: &'a Path) -> &'a Path {
    path.strip_prefix(root)
        .expect("case file is under suite draft")
}

#[test]
fn json_file_discovery_is_recursive_and_sorted() {
    let root = tempfile::tempdir().expect("temporary directory");
    let optional = root.path().join("optional");
    let format = optional.join("format");
    fs::create_dir_all(&format).expect("nested directories");
    for path in [
        root.path().join("root.json"),
        optional.join("bignum.json"),
        format.join("date-time.json"),
        optional.join("ignored.txt"),
    ] {
        fs::write(path, "[]").expect("test file");
    }

    assert_eq!(
        json_files(root.path()).expect("discover JSON files"),
        [
            optional.join("bignum.json"),
            format.join("date-time.json"),
            root.path().join("root.json"),
        ]
    );
}

#[test]
fn case_path_preserves_nested_directories() {
    let root = Path::new("tests/suite/tests/draft7");
    let path = root.join("optional/format/date-time.json");

    assert_eq!(
        relative_case_path(root, &path),
        Path::new("optional/format/date-time.json")
    );
}

#[test]
fn missing_suite_error_is_actionable() {
    let draft = "__missing_jsonb_suite_draft__";
    let panic =
        std::panic::catch_unwind(|| case_files(draft)).expect_err("missing suite draft panics");
    let message = panic.downcast_ref::<String>().expect("String panic");
    let path = Path::new("tests/suite/tests").join(draft);
    let expected_prefix = format!("failed to read suite at {}:", path.display());

    assert!(message.starts_with(&expected_prefix));
    assert!(message.ends_with("; initialize the test-suite submodule"));
}

// Runs every suite instance through both representations and reports where they disagree. The
// suite's own `valid` flag is not the oracle here: a test this crate already fails fails under
// both, while a divergence is always a `Jsonb` bug.
fn collect_divergences(draft: &str, target: Draft) -> Vec<String> {
    let mut divergences = Vec::new();
    let root = Path::new("tests/suite/tests").join(draft);
    for path in case_files(draft) {
        let file = relative_case_path(&root, &path).display();
        let cases: Vec<Value> = serde_json::from_str(
            &fs::read_to_string(&path).unwrap_or_else(|_| panic!("read {}", path.display())),
        )
        .unwrap_or_else(|_| panic!("parse {}", path.display()));

        for case in cases {
            let schema = &case["schema"];
            let serde_built = jsonschema::options().with_draft(target).build(schema);
            let jsonb_built = jsonschema::options_for::<Jsonb>()
                .with_draft(target)
                .build(schema);
            let (Ok(serde_validator), Ok(jsonb_validator)) = (&serde_built, &jsonb_built) else {
                // A schema only one of them can compile is a divergence in its own right.
                if serde_built.is_ok() != jsonb_built.is_ok() {
                    divergences.push(format!(
                        "{draft}/{file}: {}: builds as serde_json={} jsonb={}",
                        case["description"],
                        serde_built.is_ok(),
                        jsonb_built.is_ok()
                    ));
                }
                continue;
            };

            for test in case["tests"].as_array().expect("tests array") {
                let instance = &test["data"];
                if !representable(instance) {
                    continue;
                }
                let encoded = encode(instance);
                let with_serde = serde_validator.is_valid(instance);
                let with_jsonb = jsonb_validator.is_valid(Jsonb::root(&encoded));
                if with_serde != with_jsonb {
                    divergences.push(format!(
                        "{draft}/{file}: {} / {}: serde_json={with_serde} jsonb={with_jsonb}",
                        case["description"], test["description"]
                    ));
                }
            }
        }
    }
    divergences
}

// Every instance the in-process walk compares, in a stable order.
fn suite_instances() -> Vec<(String, Value)> {
    let mut instances = Vec::new();
    for (draft, _) in DRAFTS {
        let root = Path::new("tests/suite/tests").join(draft);
        for path in case_files(draft) {
            let file = relative_case_path(&root, &path).display();
            let cases: Vec<Value> = serde_json::from_str(
                &fs::read_to_string(&path).unwrap_or_else(|_| panic!("read {}", path.display())),
            )
            .unwrap_or_else(|_| panic!("parse {}", path.display()));
            for case in cases {
                for test in case["tests"].as_array().expect("tests array") {
                    let instance = &test["data"];
                    let text = instance.to_string();
                    if !representable(instance)
                        || text.len() > MAX_PLAIN_BYTES
                        || text.contains(QUOTE_TAG)
                    {
                        continue;
                    }
                    instances.push((
                        format!(
                            "{draft}/{file}: {} / {}",
                            case["description"], test["description"]
                        ),
                        instance.clone(),
                    ));
                }
            }
        }
    }
    instances
}

fn capture_live_script(instances: &[(String, Value)]) -> String {
    let mut script = String::from(
        "CREATE EXTENSION IF NOT EXISTS pageinspect;\n\
         CREATE TEMP TABLE jsonb_suite (id int4 NOT NULL, j jsonb NOT NULL);\n\
         ALTER TABLE jsonb_suite ALTER COLUMN j SET STORAGE PLAIN;\n",
    );
    for (id, (_, instance)) in instances.iter().enumerate() {
        writeln!(
            script,
            "INSERT INTO jsonb_suite VALUES ({id}, {QUOTE_TAG}{instance}{QUOTE_TAG}::jsonb);"
        )
        .expect("write to String never fails");
    }
    script.push_str(
        "SELECT encode(i.t_data, 'hex')\n\
         FROM generate_series(0, (pg_relation_size('jsonb_suite') / 8192)::int - 1) AS p,\n\
         LATERAL heap_page_items(get_raw_page('jsonb_suite', p)) AS i\n\
         WHERE i.t_data IS NOT NULL\n\
         ORDER BY p, i.lp;\n",
    );
    script
}

#[test]
fn live_capture_uses_a_temporary_table() {
    let script = capture_live_script(&[]);

    assert!(script.contains("CREATE TEMP TABLE jsonb_suite"));
    assert!(!script.contains("DROP TABLE"));
}

// Inserts every instance in one session, then reads the stored bytes back in physical order. A
// mismatch in that order shows up as a decode mismatch rather than passing silently.
fn capture_live_bytes(url: &str, instances: &[(String, Value)]) -> Vec<Vec<u8>> {
    let script = capture_live_script(instances);

    let mut child = Command::new("psql")
        .args([url, "-Atq", "-v", "ON_ERROR_STOP=1", "-f", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("psql runs");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(script.as_bytes())
        .expect("script written");
    let output = child.wait_with_output().expect("psql finishes");
    assert!(
        output.status.success(),
        "psql failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Postgres backfills earlier pages once a large row forces a new one, so physical order is
    // not insertion order; `id` leads every tuple's data, ahead of the 4-aligned `jsonb`.
    let mut rows: Vec<(u32, Vec<u8>)> = String::from_utf8(output.stdout)
        .expect("psql output is UTF-8")
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let bytes = decode_hex(line);
            let id = u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            (id, bytes[4..].to_vec())
        })
        .collect();
    rows.sort_by_key(|(id, _)| *id);
    rows.into_iter().map(|(_, bytes)| bytes).collect()
}

// Opt-in: the whole suite through a real server, so the encoder the other test relies on is
// checked against what Postgres actually stores.
#[test]
fn encoder_matches_postgres_across_the_suite() {
    let Ok(url) = env::var(PG_URL_VAR) else {
        eprintln!("set {PG_URL_VAR} to run this against a live server");
        return;
    };
    let instances = suite_instances();
    let stored = capture_live_bytes(&url, &instances);
    assert_eq!(
        stored.len(),
        instances.len(),
        "captured a different number of rows than were inserted"
    );

    let mut mismatches = Vec::new();
    for ((name, instance), row) in instances.iter().zip(&stored) {
        let theirs = strip_varlena(row);
        let ours = encode(instance);
        if ours != theirs {
            mismatches.push(format!("{name}: {instance}"));
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} instances encode differently than Postgres stores them:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
}

#[test]
fn representations_agree_across_the_suite() {
    let mut divergences = Vec::new();
    for (draft, target) in DRAFTS {
        divergences.extend(collect_divergences(draft, target));
    }
    assert!(
        divergences.is_empty(),
        "{} instances validate differently as `jsonb`:\n{}",
        divergences.len(),
        divergences.join("\n")
    );
}
