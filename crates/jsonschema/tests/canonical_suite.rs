#![allow(clippy::needless_pass_by_value)]

use jsonschema::{
    canonical::{self, CanonicalSchema, CanonicalizationError, CanonicalizeOptions},
    Draft, Validator,
};
use serde::Deserialize;
use serde_json::Value;
use testsuite::canonical_suite;

#[canonical_suite(path = "crates/jsonschema/tests/canonical-suite")]
fn run_case(case: CanonicalCase) {
    let inputs = case.inputs();

    // Negative cases: every input must be rejected with the expected `CanonicalizationError` variant.
    if let Some(expected_kind) = &case.error {
        assert!(
            case.expected.is_none() && case.witnesses.is_empty() && case.satisfiable.is_none(),
            "case `{}`: `error` cases cannot also set `expected`/`witnesses`/`satisfiable`",
            case.description
        );
        for input in &inputs {
            match case.options().canonicalize(input) {
                Err(error) if error_kind(&error) == expected_kind => {}
                other => panic!(
                    "case `{}`: expected {expected_kind} for {input}, got {other:?}",
                    case.description
                ),
            }
        }
        return;
    }

    let raw_draft = case
        .draft()
        .unwrap_or_else(|| Draft::default().detect(inputs[0]));

    // Canonicalize every input; check idempotency and convergence.
    let mut form: Option<Value> = None;
    let mut form_draft = raw_draft;
    let mut canonical: Option<CanonicalSchema> = None;
    for input in &inputs {
        let canon = case.canonicalize(input);
        let emitted = canon.to_json_schema();
        let reemitted = case.canonicalize(&emitted).to_json_schema();
        assert_eq!(
            emitted, reemitted,
            "case `{}`: not idempotent\n  input = {input}\n  once  = {emitted}\n  twice = {reemitted}",
            case.description
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
                case.description
            ),
        }
    }
    let form = form.expect("at least one input");
    let canonical = canonical.expect("at least one input");

    // Satisfiability: the membership oracle must agree with reality. A schema admitting any witness
    // cannot be provably empty; an explicit `satisfiable` pins the verdict either way.
    let satisfiable = canonical.is_satisfiable();
    if let Some(expected) = case.satisfiable {
        assert_eq!(
            satisfiable, expected,
            "case `{}`: is_satisfiable() = {satisfiable}, expected {expected}\n  form = {form}",
            case.description
        );
    }
    if case
        .witnesses
        .iter()
        .any(|witness| witness.valid == Some(true))
    {
        assert!(
            satisfiable,
            "case `{}`: admits a witness but is_satisfiable() is false\n  form = {form}",
            case.description
        );
    }

    // Exact canonical form, if pinned.
    if let Some(expected) = &case.expected {
        assert_eq!(
            &form, expected,
            "case `{}`: canonical form mismatch\n  expected = {expected}\n  actual   = {form}",
            case.description
        );
    }

    // Witness parity: raw validator(s) and the canonical validator must agree.
    if !case.witnesses.is_empty() {
        let canonical_validator = case.build(&form, form_draft);
        let raw_validators: Vec<_> = inputs
            .iter()
            .map(|input| (input, case.build(input, raw_draft)))
            .collect();
        for witness in &case.witnesses {
            let canon_valid = canonical_validator.is_valid(&witness.data);
            for (input, raw_validator) in &raw_validators {
                let raw_valid = raw_validator.is_valid(&witness.data);
                assert_eq!(
                    raw_valid, canon_valid,
                    "case `{}`: parity disagrees on {}\n  raw   ({input}) = {raw_valid}\n  canon ({form}) = {canon_valid}",
                    case.description, witness.data
                );
                if let Some(expected_valid) = witness.valid {
                    assert_eq!(
                        raw_valid, expected_valid,
                        "case `{}`: raw verdict wrong on {}: expected {expected_valid}, got {raw_valid}",
                        case.description, witness.data
                    );
                }
            }
        }
    }
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
    /// Force `should_validate_formats`; `None` keeps the draft default.
    #[serde(default)]
    validate_formats: Option<bool>,
    /// Cap on inlining referenced definitions. `0` keeps every `$ref` symbolic, so a case can pin how
    /// references and `$defs` are canonicalized instead of inlined away.
    #[serde(default)]
    inline_budget: Option<usize>,
    /// Every input must be rejected with this exact `CanonicalizationError` variant.
    #[serde(default)]
    error: Option<String>,
}

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
#[derive(Debug)]
struct Witness {
    data: Value,
    valid: Option<bool>,
}

impl<'de> Deserialize<'de> for Witness {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Tagged {
            data: Value,
            #[serde(default)]
            valid: Option<bool>,
        }
        let value = Value::deserialize(d)?;
        match &value {
            Value::Object(object) if object.contains_key("data") => {
                let Tagged { data, valid } =
                    serde_json::from_value(value).map_err(serde::de::Error::custom)?;
                Ok(Witness { data, valid })
            }
            _ => Ok(Witness {
                data: value,
                valid: None,
            }),
        }
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

    fn options(&self) -> CanonicalizeOptions<'static> {
        let mut options = canonical::options();
        if let Some(draft) = self.draft() {
            options = options.with_draft(draft);
        }
        if let Some(validate) = self.validate_formats {
            options = options.should_validate_formats(validate);
        }
        if let Some(budget) = self.inline_budget {
            options = options.with_inline_budget(budget);
        }
        options
    }

    fn canonicalize(&self, schema: &Value) -> CanonicalSchema {
        self.options().canonicalize(schema).unwrap_or_else(|e| {
            panic!(
                "case `{}`: canonicalize({schema}) failed: {e}",
                self.description
            )
        })
    }

    fn build(&self, schema: &Value, draft: Draft) -> Validator {
        let mut options = jsonschema::options().with_draft(draft);
        if let Some(validate) = self.validate_formats {
            options = options.should_validate_formats(validate);
        }
        options
            .build(schema)
            .unwrap_or_else(|e| panic!("case `{}`: build({schema}) failed: {e}", self.description))
    }
}
