#![cfg(feature = "jsonb-testkit")]

use std::{
    env,
    io::Write as _,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        Once,
    },
};

use hegel::{extras::serde_json as json_gs, TestCase};
use jsonschema_value::{cmp, Jsonb, Node};
use serde_json::Value;

mod common;
use common::{decode_hex, encode, strip_varlena, to_hex};

const PG_URL_VAR: &str = "JSONB_LIVE_PG_URL";
// One psql round trip per draw, which a local server answers in single-digit milliseconds.
const LIVE_TEST_CASES: u64 = 300;

fn skip_message() -> String {
    format!(
        "skipped: set {PG_URL_VAR}=<postgres-connection-url> to run the live jsonb differential test"
    )
}

fn psql(url: &str) -> Command {
    let mut command = Command::new("psql");
    command
        .arg(url)
        .args(["-X", "-q", "-A", "-t", "-v", "ON_ERROR_STOP=1"]);
    command
}

// Panics with the captured stderr rather than skipping: a live run is opted into explicitly.
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

static ENCODER_DRAWS: AtomicU64 = AtomicU64::new(0);
static ENCODER_REJECTS: AtomicU64 = AtomicU64::new(0);
static READER_DRAWS: AtomicU64 = AtomicU64::new(0);
static READER_REJECTS: AtomicU64 = AtomicU64::new(0);

#[test]
fn each_query_uses_a_temporary_table() {
    let script = query_script();

    assert!(script.contains("CREATE TEMP TABLE jsonb_live"));
    assert!(!script.contains("TRUNCATE"));
    assert!(!script.contains("DROP TABLE"));
}

#[hegel::standalone_function(test_cases = LIVE_TEST_CASES)]
fn run_encoder_matches_postgres_bytes(tc: TestCase, url: String) {
    let value = tc.draw(json_gs::values());
    ENCODER_DRAWS.fetch_add(1, Ordering::Relaxed);
    if contains_nul(&value) {
        ENCODER_REJECTS.fetch_add(1, Ordering::Relaxed);
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

// Announces a skip (unset env var) or a draw count on success, so a run is never silent.
#[allow(clippy::print_stdout, clippy::print_stderr)]
#[test]
fn encoder_matches_postgres_bytes() {
    let Ok(url) = env::var(PG_URL_VAR) else {
        eprintln!("{}", skip_message());
        return;
    };
    setup(&url);
    run_encoder_matches_postgres_bytes(url);
    println!(
        "encoder_matches_postgres_bytes: {} draws, {} rejected",
        ENCODER_DRAWS.load(Ordering::Relaxed),
        ENCODER_REJECTS.load(Ordering::Relaxed)
    );
}

#[hegel::standalone_function(test_cases = LIVE_TEST_CASES)]
fn run_reader_matches_postgres_rendering(tc: TestCase, url: String) {
    let value = tc.draw(json_gs::values());
    READER_DRAWS.fetch_add(1, Ordering::Relaxed);
    if contains_nul(&value) {
        READER_REJECTS.fetch_add(1, Ordering::Relaxed);
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

// Announces a skip (unset env var) or a draw count on success, so a run is never silent.
#[allow(clippy::print_stdout, clippy::print_stderr)]
#[test]
fn reader_matches_postgres_rendering() {
    let Ok(url) = env::var(PG_URL_VAR) else {
        eprintln!("{}", skip_message());
        return;
    };
    setup(&url);
    run_reader_matches_postgres_rendering(url);
    println!(
        "reader_matches_postgres_rendering: {} draws, {} rejected",
        READER_DRAWS.load(Ordering::Relaxed),
        READER_REJECTS.load(Ordering::Relaxed)
    );
}
