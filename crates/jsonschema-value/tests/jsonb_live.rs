#![cfg(feature = "jsonb-testkit")]

use std::{
    env,
    io::Write as _,
    process::{Command, Stdio},
    sync::Once,
};

use hegel::{extras::serde_json as json_gs, TestCase};
use jsonschema_value::{
    cmp,
    jsonb::encode::{decode_hex, encode, strip_varlena, to_hex},
    Jsonb, Node,
};
use serde_json::Value;

const PG_URL_VAR: &str = "JSONB_LIVE_PG_URL";
// One psql round trip per draw, which a local server answers in single-digit milliseconds.
const LIVE_TEST_CASES: u64 = 300;

fn psql(url: &str) -> Command {
    let mut command = Command::new("psql");
    command
        .arg(url)
        .args(["-X", "-q", "-A", "-t", "-v", "ON_ERROR_STOP=1"]);
    command
}

fn exec(mut command: Command, script: &str) -> String {
    let mut child = command
        .args(["-f", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("psql spawns");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(script.as_bytes())
        .expect("sql script writes to psql stdin");
    let output = child.wait_with_output().expect("psql runs to completion");
    assert!(
        output.status.success(),
        "psql exited with {}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("psql stdout is utf-8")
}

static PAGEINSPECT_SETUP: Once = Once::new();

fn setup(url: &str) {
    PAGEINSPECT_SETUP.call_once(|| {
        exec(psql(url), "CREATE EXTENSION IF NOT EXISTS pageinspect;\n");
    });
}

// A NUL byte cannot cross `execve`'s argv, so such draws are declined before spawning. Postgres
// rejects them too, so nothing is lost.
fn contains_nul(value: &Value) -> bool {
    match value {
        Value::String(string) => string.contains('\0'),
        Value::Array(items) => items.iter().any(contains_nul),
        Value::Object(members) => members
            .iter()
            .any(|(key, member)| key.contains('\0') || contains_nul(member)),
        _ => false,
    }
}

fn query_script() -> &'static str {
    "CREATE TEMP TABLE jsonb_live (j jsonb);\n\
     ALTER TABLE jsonb_live ALTER COLUMN j SET STORAGE PLAIN;\n\
     INSERT INTO jsonb_live VALUES (:'json'::jsonb);\n\
     SELECT j::text FROM jsonb_live;\n\
     SELECT encode(t_data, 'hex') FROM heap_page_items(get_raw_page('jsonb_live', 0)) WHERE lp_len > 0;\n"
}

// The value goes in as a psql variable, never spliced into the SQL; its rendering and bytes come back.
fn query(url: &str, value: &Value) -> (String, Vec<u8>) {
    let mut command = psql(url);
    command.arg("-v").arg(format!("json={value}"));
    let output = exec(command, query_script());
    let mut lines = output.lines();
    let text = lines.next().expect("postgres text line").to_string();
    let hex = lines.next().expect("postgres hex line");
    (text, decode_hex(hex))
}

#[hegel::standalone_function(test_cases = LIVE_TEST_CASES)]
fn run_encoder_matches_postgres_bytes(tc: TestCase, url: String) {
    let value = tc.draw(json_gs::values());
    if contains_nul(&value) {
        tc.reject();
    }
    let (_, stored) = query(&url, &value);
    let ours = encode(&value);
    let theirs = strip_varlena(&stored);
    assert_eq!(
        ours,
        theirs,
        "encoder diverges from postgres for {value:?}\n  ours:   {}\n  theirs: {}",
        to_hex(&ours),
        to_hex(theirs),
    );
}

#[test]
#[ignore = "needs a PostgreSQL server with `pageinspect` at ${JSONB_LIVE_PG_URL}"]
fn encoder_matches_postgres_bytes() {
    let url = env::var(PG_URL_VAR).expect("JSONB_LIVE_PG_URL is set");
    setup(&url);
    run_encoder_matches_postgres_bytes(url);
}

#[hegel::standalone_function(test_cases = LIVE_TEST_CASES)]
fn run_reader_matches_postgres_rendering(tc: TestCase, url: String) {
    let value = tc.draw(json_gs::values());
    if contains_nul(&value) {
        tc.reject();
    }
    let (text, stored) = query(&url, &value);
    let decoded = Jsonb::root(strip_varlena(&stored)).to_value();
    let expected: Value = serde_json::from_str(&text).expect("postgres text parses");
    assert!(
        cmp::equal(decoded.as_ref(), &expected),
        "reader diverges from postgres for drawn {value:?}\n  ours:     {decoded:?}\n  postgres: {expected:?}"
    );
}

#[test]
#[ignore = "needs a PostgreSQL server with `pageinspect` at ${JSONB_LIVE_PG_URL}"]
fn reader_matches_postgres_rendering() {
    let url = env::var(PG_URL_VAR).expect("JSONB_LIVE_PG_URL is set");
    setup(&url);
    run_reader_matches_postgres_rendering(url);
}
