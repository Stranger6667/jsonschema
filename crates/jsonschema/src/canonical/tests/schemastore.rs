use std::{
    fmt,
    fs::File,
    io::{BufReader, Read, Seek, Write},
    path::PathBuf,
    time::{Duration, Instant},
};

use referencing::{Retrieve, Uri};
use serde::de::{Deserializer as _, MapAccess, Visitor};
use serde_json::Value;

use crate::canonical::{ir::Schema, CanonicalSchema, CanonicalizationError};

const CORPUS_PATH: &str = "tests/fixtures/schemastore/corpus-schemastore-catalog.json";
const GIT_LFS_POINTER_PREFIX: &[u8] = b"version https://git-lfs.github.com/spec/v1";

fn corpus_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(CORPUS_PATH)
}

/// Refuses every external fetch so unresolvable refs stay symbolic. Live retrieval would make the
/// outcome network-dependent - a fetch flipping between passes shows up as spurious round-trip/idempotence failure.
struct NoExternalRetrieval;

impl Retrieve for NoExternalRetrieval {
    fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        Err(format!("external retrieval is disabled in the corpus test: {uri}").into())
    }
}

fn canonicalize(schema: &Value) -> Result<CanonicalSchema, CanonicalizationError> {
    crate::canonical::options()
        .with_retriever(NoExternalRetrieval)
        .canonicalize(schema)
}

fn open_corpus() -> File {
    let path = corpus_path();
    let mut file = File::open(&path).unwrap_or_else(|error| {
        panic!(
            "schemastore fixture must be present at {}: {error}",
            path.display()
        )
    });
    let mut prefix = [0_u8; GIT_LFS_POINTER_PREFIX.len()];
    let bytes_read = file
        .read(&mut prefix)
        .expect("schemastore fixture prefix is readable");
    assert!(
        bytes_read < GIT_LFS_POINTER_PREFIX.len() || prefix != GIT_LFS_POINTER_PREFIX,
        "schemastore fixture at {} is a Git LFS pointer; run `git lfs pull`",
        path.display(),
    );
    file.rewind()
        .expect("schemastore fixture can be rewound after LFS check");
    file
}

struct Report {
    total: usize,
    parse_failed: Vec<String>,
    canonicalize_panicked: Vec<String>,
    round_trip_failed: Vec<String>,
    non_idempotent: Vec<String>,
    round_trip_skipped_recursive: usize,
    succeeded: usize,
}

impl Report {
    fn new() -> Self {
        Self {
            total: 0,
            parse_failed: Vec::new(),
            canonicalize_panicked: Vec::new(),
            round_trip_failed: Vec::new(),
            non_idempotent: Vec::new(),
            round_trip_skipped_recursive: 0,
            succeeded: 0,
        }
    }
}

fn contains_recursive(schema: &Schema) -> bool {
    matches!(schema, Schema::Recursive(_))
        || schema
            .children()
            .iter()
            .any(|child| contains_recursive(child.as_schema()))
}

fn check_schema<W: Write>(
    index: usize,
    name: String,
    schema: &Value,
    report: &mut Report,
    stderr: &mut W,
) {
    let schema_started = Instant::now();
    let _ = writeln!(stderr, "schemastore: {index} {name}");
    let canonical_result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| canonicalize(schema)));
    let canonical = match canonical_result {
        Err(_) => {
            report.canonicalize_panicked.push(name);
            return;
        }
        Ok(Err(error)) => {
            report.parse_failed.push(format!("{name}: {error}"));
            return;
        }
        Ok(Ok(c)) => c,
    };
    if contains_recursive(canonical.as_schema()) {
        report.round_trip_skipped_recursive += 1;
        return;
    }
    let second_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        canonicalize(&canonical.to_json_schema())
    }));
    let Ok(Ok(second)) = second_result else {
        report.round_trip_failed.push(name);
        return;
    };
    if canonical == second {
        report.succeeded += 1;
    } else {
        report.non_idempotent.push(name);
        return;
    }
    let elapsed = schema_started.elapsed();
    if elapsed >= Duration::from_secs(2) {
        let _ = writeln!(stderr, "schemastore slow schema: {elapsed:?} {name}");
    }
}

struct CorpusVisitor<'a, W> {
    stderr: &'a mut W,
}

impl<'de, W: Write> Visitor<'de> for CorpusVisitor<'_, W> {
    type Value = Report;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON object mapping SchemaStore names to schemas")
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let mut report = Report::new();
        while let Some((name, schema)) = map.next_entry::<String, Value>()? {
            report.total += 1;
            check_schema(report.total, name, &schema, &mut report, self.stderr);
        }
        Ok(report)
    }
}

fn check_corpus<W: Write>(stderr: &mut W) -> Report {
    let file = open_corpus();
    let reader = BufReader::new(file);
    let mut deserializer = serde_json::Deserializer::from_reader(reader);
    deserializer
        .deserialize_map(CorpusVisitor { stderr })
        .expect("corpus is a JSON object")
}

// Run:
//
//     cargo test --release --features arbitrary-precision -p jsonschema --lib canonical::tests::schemastore::corpus --
//         --ignored --exact --nocapture
#[test]
#[ignore = "heavy: release-only corpus test"]
fn corpus() {
    let mut stderr = std::io::stderr();
    let total_started = Instant::now();
    let report = check_corpus(&mut stderr);

    let parse_fail = report.parse_failed.len();
    let panic = report.canonicalize_panicked.len();
    let round_trip_fail = report.round_trip_failed.len();
    let idempotence_fail = report.non_idempotent.len();
    let _ = writeln!(
        stderr,
        "schemastore: total={} elapsed={:?} succeeded={} parse_failed={} recursive_round_trip_skipped={} canonicalize_panicked={} round_trip_failed={} non_idempotent={}",
        report.total,
        total_started.elapsed(),
        report.succeeded,
        parse_fail,
        report.round_trip_skipped_recursive,
        panic,
        round_trip_fail,
        idempotence_fail,
    );
    if !report.canonicalize_panicked.is_empty() {
        let _ = writeln!(stderr, "PANICKED ON: {:#?}", report.canonicalize_panicked);
    }
    if !report.round_trip_failed.is_empty() {
        let _ = writeln!(
            stderr,
            "ROUND-TRIP FAILED ON: {:#?}",
            report.round_trip_failed
        );
    }
    if !report.non_idempotent.is_empty() {
        let _ = writeln!(
            stderr,
            "NON-IDEMPOTENT: {:#?}",
            &report.non_idempotent[..report.non_idempotent.len().min(20)]
        );
    }
    if !report.parse_failed.is_empty() {
        let _ = writeln!(
            stderr,
            "PARSE FAILURES (first 20): {:#?}",
            &report.parse_failed[..report.parse_failed.len().min(20)],
        );
    }

    assert_eq!(
        report.canonicalize_panicked.len(),
        0,
        "canonicalize panicked on {} schemas (see stderr)",
        report.canonicalize_panicked.len(),
    );
    assert_eq!(
        report.round_trip_failed.len(),
        0,
        "canonicalize(to_value()) failed on {} schemas (see stderr)",
        report.round_trip_failed.len(),
    );
    assert_eq!(
        report.non_idempotent.len(),
        0,
        "canonicalize(to_value()) was non-idempotent for {} schemas (see stderr)",
        report.non_idempotent.len(),
    );
}
