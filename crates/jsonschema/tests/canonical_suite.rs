#![cfg(feature = "canonical")]
#![allow(clippy::needless_pass_by_value)]

use jsonschema::{
    canonical::{options as canonical_options, CanonicalSchema, CanonicalizationError},
    Draft, Validator,
};
use serde::Deserialize;
use serde_json::Value;
use testsuite::canonical_suite;

#[canonical_suite(path = "crates/jsonschema/tests/canonical-suite")]
fn run_case(case: CanonicalCase) {
    case.check();
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CanonicalCase {
    description: String,
    #[serde(default)]
    schema: Option<Value>,
    #[serde(default)]
    schemas: Vec<Value>,
    #[serde(default)]
    draft: Option<String>,
    #[serde(default)]
    expected: Option<Value>,
    #[serde(default)]
    witnesses: Vec<Witness>,
    /// Pins `CanonicalSchema::is_satisfiable()`. Independent of `witnesses`, but a `valid: true` witness
    /// already implies satisfiable (checked automatically).
    #[serde(default)]
    satisfiable: Option<bool>,
    /// Override `should_validate_formats` for canonicalization and witness validation. `None` keeps the
    /// draft default (assertions on for draft 4/6/7, off otherwise); `Some` forces it either way. Lets a
    /// case exercise format-disjointness satisfiability and format-assertion-policy preservation.
    #[serde(default)]
    validate_formats: Option<bool>,
    /// Cap on inlining referenced definitions. `0` keeps every `$ref` symbolic, so a case can pin how
    /// references and `$defs` are canonicalized instead of inlined away.
    #[serde(default)]
    inline_budget: Option<usize>,
    /// Expectation that every input is rejected by canonicalize. `true` is shorthand for a
    /// meta-schema `ValidationError`; a string names the exact `CanonicalizationError` variant
    /// (e.g. `"InfiniteRecursion"`, `"InvalidSchemaType"`).
    #[serde(default, deserialize_with = "deserialize_error_expectation")]
    error: Option<String>,
}

fn deserialize_error_expectation<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<String>, D::Error> {
    match Value::deserialize(d)? {
        Value::Bool(false) => Ok(None),
        Value::Bool(true) => Ok(Some("ValidationError".to_owned())),
        Value::String(name) => Ok(Some(name)),
        other => Err(serde::de::Error::custom(format!(
            "`error` must be a boolean or a variant-name string, got {other}"
        ))),
    }
}

/// The `CanonicalizationError` variant name, for matching against an `ErrorExpectation::Kind`.
fn error_kind(error: &CanonicalizationError) -> &'static str {
    match error {
        CanonicalizationError::InvalidSchemaType(_) => "InvalidSchemaType",
        CanonicalizationError::ValidationError(_) => "ValidationError",
        CanonicalizationError::UnguardedRecursion(_) => "UnguardedRecursion",
        CanonicalizationError::InfiniteRecursion(_) => "InfiniteRecursion",
        CanonicalizationError::InvalidPattern { .. } => "InvalidPattern",
        CanonicalizationError::InvalidJsonValue(_) => "InvalidJsonValue",
        _ => "Unknown",
    }
}

/// Plain JSON value (parity-only) or `{"data": ..., "valid": bool}` (also checks verdict).
/// Unknown keys in `data`-keyed objects are rejected to catch typos.
#[derive(Debug)]
struct Witness {
    data: Value,
    valid: Option<bool>,
}

impl<'de> Deserialize<'de> for Witness {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        if let Value::Object(ref obj) = v {
            if obj.contains_key("data") {
                let unknown: Vec<_> = obj
                    .keys()
                    .filter(|k| *k != "data" && *k != "valid")
                    .collect();
                if !unknown.is_empty() {
                    return Err(serde::de::Error::custom(format!(
                        "witness object has unknown key(s): {}",
                        unknown
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )));
                }
                let data = obj["data"].clone();
                let valid = match obj.get("valid") {
                    Some(Value::Bool(b)) => Some(*b),
                    Some(other) => {
                        return Err(serde::de::Error::custom(format!(
                            "witness `valid` must be a boolean, got {other}"
                        )))
                    }
                    None => None,
                };
                return Ok(Witness { data, valid });
            }
        }
        Ok(Witness {
            data: v,
            valid: None,
        })
    }
}

impl CanonicalCase {
    fn inputs(&self) -> Vec<&Value> {
        match (&self.schema, self.schemas.as_slice()) {
            (Some(schema), []) => vec![schema],
            (None, list) if !list.is_empty() => list.iter().collect(),
            (Some(_), _) => panic!(
                "case `{}`: set either `schema` or `schemas`, not both",
                self.description
            ),
            (None, _) => panic!(
                "case `{}`: must set `schema` or `schemas`",
                self.description
            ),
        }
    }

    fn draft(&self) -> Option<Draft> {
        self.draft.as_deref().map(|name| match name {
            "draft4" => Draft::Draft4,
            "draft6" => Draft::Draft6,
            "draft7" => Draft::Draft7,
            "draft2019-09" => Draft::Draft201909,
            "draft2020-12" => Draft::Draft202012,
            other => panic!("case `{}`: unknown draft `{other}`", self.description),
        })
    }

    fn canonicalize(&self, schema: &Value) -> CanonicalSchema {
        let mut opts = canonical_options();
        if let Some(draft) = self.draft() {
            opts = opts.with_draft(draft);
        }
        if let Some(validate) = self.validate_formats {
            opts = opts.should_validate_formats(validate);
        }
        if let Some(budget) = self.inline_budget {
            opts = opts.with_inline_budget(budget);
        }
        opts.canonicalize(schema).unwrap_or_else(|e| {
            panic!(
                "case `{}`: canonicalize({schema}) failed: {e}",
                self.description
            )
        })
    }

    fn build(&self, schema: &Value, draft: Draft) -> Validator {
        let mut opts = jsonschema::options().with_draft(draft);
        if let Some(validate) = self.validate_formats {
            opts = opts.should_validate_formats(validate);
        }
        opts.build(schema)
            .unwrap_or_else(|e| panic!("case `{}`: build({schema}) failed: {e}", self.description))
    }

    fn check(&self) {
        let inputs = self.inputs();

        // Negative cases: every input must be rejected with the expected `CanonicalizationError` variant.
        if let Some(expected_kind) = &self.error {
            assert!(
                self.expected.is_none() && self.witnesses.is_empty() && self.satisfiable.is_none(),
                "case `{}`: `error` cases cannot also set `expected`/`witnesses`/`satisfiable`",
                self.description
            );
            for input in &inputs {
                let mut opts = canonical_options();
                if let Some(draft) = self.draft() {
                    opts = opts.with_draft(draft);
                }
                if let Some(validate) = self.validate_formats {
                    opts = opts.should_validate_formats(validate);
                }
                if let Some(budget) = self.inline_budget {
                    opts = opts.with_inline_budget(budget);
                }
                match opts.canonicalize(input) {
                    Err(error) if error_kind(&error) == expected_kind => {}
                    other => panic!(
                        "case `{}`: expected {expected_kind} for {input}, got {other:?}",
                        self.description
                    ),
                }
            }
            return;
        }

        let raw_draft = self
            .draft()
            .unwrap_or_else(|| Draft::default().detect(inputs[0]));

        // Canonicalize every input; check idempotency and convergence.
        let mut form: Option<Value> = None;
        let mut form_draft = raw_draft;
        let mut canonical: Option<CanonicalSchema> = None;
        for input in &inputs {
            let canon = self.canonicalize(input);
            let emitted = canon.to_json_schema();
            let reemitted = self.canonicalize(&emitted).to_json_schema();
            assert_eq!(
                emitted, reemitted,
                "case `{}`: not idempotent\n  input = {input}\n  once  = {emitted}\n  twice = {reemitted}",
                self.description
            );
            match &form {
                None => {
                    form_draft = canon.draft();
                    form = Some(emitted);
                    canonical = Some(canon);
                }
                Some(prev) => assert_eq!(
                    prev, &emitted,
                    "case `{}`: inputs do not converge\n  a = {prev}\n  b = {emitted}",
                    self.description
                ),
            }
        }
        let form = form.expect("at least one input");
        let canonical = canonical.expect("at least one input");

        // Satisfiability: the membership oracle must agree with reality. A schema admitting any witness
        // cannot be provably empty; an explicit `satisfiable` pins the verdict either way.
        let satisfiable = canonical.is_satisfiable();
        if let Some(expected) = self.satisfiable {
            assert_eq!(
                satisfiable, expected,
                "case `{}`: is_satisfiable() = {satisfiable}, expected {expected}\n  form = {form}",
                self.description
            );
        }
        if self
            .witnesses
            .iter()
            .any(|witness| witness.valid == Some(true))
        {
            assert!(
                satisfiable,
                "case `{}`: admits a witness but is_satisfiable() is false\n  form = {form}",
                self.description
            );
        }

        // Exact canonical form, if pinned.
        if let Some(expected) = &self.expected {
            assert_eq!(
                &form, expected,
                "case `{}`: canonical form mismatch\n  expected = {expected}\n  actual   = {form}",
                self.description
            );
        }

        // Witness parity: raw validator(s) and the canonical validator must agree.
        if !self.witnesses.is_empty() {
            let canon_validator = self.build(&form, form_draft);
            for witness in &self.witnesses {
                let canon_valid = canon_validator.is_valid(&witness.data);
                for input in &inputs {
                    let raw_valid = self.build(input, raw_draft).is_valid(&witness.data);
                    assert_eq!(
                        raw_valid, canon_valid,
                        "case `{}`: parity disagrees on {}\n  raw   ({input}) = {raw_valid}\n  canon ({form}) = {canon_valid}",
                        self.description, witness.data
                    );
                    if let Some(expected_valid) = witness.valid {
                        assert_eq!(
                            raw_valid, expected_valid,
                            "case `{}`: raw verdict wrong on {}: expected {expected_valid}, got {raw_valid}",
                            self.description, witness.data
                        );
                    }
                }
            }
        }
    }
}
