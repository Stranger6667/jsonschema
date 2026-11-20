// Schema pairs whose difference `satisfiability` must decide, one file of
// `tests/canonical-difference` per test.
use std::fmt;

use jsonschema::{
    canonical::{self, CanonicalSchema, Satisfiability},
    Draft,
};
use serde::Deserialize;
use serde_json::Value;

// The bench picks its rows from these files by description; its own test runs from here.
#[allow(dead_code)]
#[path = "../benches/canonical_difference.rs"]
mod difference_bench;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    description: String,
    old: Value,
    new: Value,
    relation: Relation,
    /// Directions answering `Unknown` today.
    #[serde(default)]
    pending: Vec<Direction>,
    draft: Option<String>,
}

/// How `new` relates to `old`.
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Relation {
    Narrow,
    Widen,
    Equal,
    Incomparable,
}

#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
enum Direction {
    #[serde(rename = "old-new")]
    OldNew,
    #[serde(rename = "new-old")]
    NewOld,
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Direction::OldNew => "old-new",
            Direction::NewOld => "new-old",
        })
    }
}

impl Case {
    /// Whether the difference along `direction` holds a value.
    fn expected(&self, direction: Direction) -> Satisfiability {
        let holds = match (self.relation, direction) {
            (Relation::Narrow | Relation::Incomparable, Direction::OldNew)
            | (Relation::Widen | Relation::Incomparable, Direction::NewOld) => true,
            (Relation::Narrow | Relation::Equal, Direction::NewOld)
            | (Relation::Widen | Relation::Equal, Direction::OldNew) => false,
        };
        if holds {
            Satisfiability::Yes
        } else {
            Satisfiability::No
        }
    }

    fn draft(&self) -> Draft {
        match self.draft.as_deref() {
            None | Some("draft2020-12") => Draft::Draft202012,
            Some("draft4") => Draft::Draft4,
            Some("draft6") => Draft::Draft6,
            Some("draft7") => Draft::Draft7,
            Some("draft2019-09") => Draft::Draft201909,
            Some(other) => panic!("{}: unknown draft `{other}`", self.description),
        }
    }

    fn canonicalize(&self, schema: &Value) -> CanonicalSchema {
        canonical::options()
            .with_draft(self.draft())
            .should_validate_formats(true)
            .canonicalize(schema)
            .unwrap_or_else(|error| {
                panic!(
                    "{}: canonicalize({schema}) failed: {error}",
                    self.description
                )
            })
    }

    /// One message per direction whose answer differs from `relation`, or that `pending` lists
    /// but is decided.
    fn failures(&self) -> Vec<String> {
        let old = self.canonicalize(&self.old);
        let new = self.canonicalize(&self.new);
        let mut failures = Vec::new();
        for direction in [Direction::OldNew, Direction::NewOld] {
            let (minuend, subtrahend) = match direction {
                Direction::OldNew => (&old, &new),
                Direction::NewOld => (&new, &old),
            };
            let actual = minuend
                .subtract(subtrahend)
                .unwrap_or_else(|error| {
                    panic!("{}: {direction} subtract failed: {error}", self.description)
                })
                .satisfiability();
            let expected = self.expected(direction);
            let pending = self.pending.contains(&direction);
            if pending && actual == Satisfiability::Unknown {
                continue;
            }
            if pending && actual == expected {
                failures.push(format!(
                    "{}: {direction} is decided, drop it from pending",
                    self.description
                ));
            } else if actual != expected {
                failures.push(format!(
                    "{}: {direction}: left: {actual:?}, right: {expected:?}",
                    self.description
                ));
            }
        }
        failures
    }
}

fn run(name: &str, cases: &str) {
    let cases: Vec<Case> =
        serde_json::from_str(cases).unwrap_or_else(|error| panic!("{name}: {error}"));
    let failures: Vec<String> = cases.iter().flat_map(Case::failures).collect();
    assert!(
        failures.is_empty(),
        "{name}: {} failing directions\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn baseline() {
    run(
        "baseline.json",
        include_str!("canonical-difference/baseline.json"),
    );
}

#[test]
fn numbers() {
    run(
        "numbers.json",
        include_str!("canonical-difference/numbers.json"),
    );
}

#[test]
fn arrays() {
    run(
        "arrays.json",
        include_str!("canonical-difference/arrays.json"),
    );
}

#[test]
fn objects() {
    run(
        "objects.json",
        include_str!("canonical-difference/objects.json"),
    );
}

#[test]
fn recursive() {
    run(
        "recursive.json",
        include_str!("canonical-difference/recursive.json"),
    );
}
