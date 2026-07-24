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

    if let Some(expected_kind) = &case.error {
        assert!(
            case.expected.is_none() && case.tests.is_empty() && case.satisfiable.is_none(),
            "case `{}`: `error` cases cannot also set `expected`/`tests`/`satisfiable`",
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

    // Idempotency and convergence across inputs, at both the emitted-JSON and the IR level.
    let mut form: Option<Value> = None;
    let mut form_draft = raw_draft;
    let mut canonical: Option<CanonicalSchema> = None;
    for input in &inputs {
        let canon = case.canonicalize(input);
        let emitted = canon.to_json_schema();
        let recanonicalized = case.canonicalize(&emitted);
        let reemitted = recanonicalized.to_json_schema();
        assert_eq!(
            emitted, reemitted,
            "case `{}`: not idempotent\n  input = {input}\n  once  = {emitted}\n  twice = {reemitted}",
            case.description
        );
        assert_eq!(
            canon, recanonicalized,
            "case `{}`: emitted form is stable but the IR is not\n  input = {input}\n  emitted = {emitted}",
            case.description
        );
        match &canonical {
            None => {
                form_draft = canon.draft();
                form = Some(emitted);
                canonical = Some(canon);
            }
            Some(first) => {
                let prev = form.as_ref().expect("set together with `canonical`");
                assert_eq!(
                    prev, &emitted,
                    "case `{}`: inputs do not converge\n  a = {prev}\n  b = {emitted}",
                    case.description
                );
                assert_eq!(
                    first, &canon,
                    "case `{}`: inputs converge on emitted JSON but not on IR\n  input = {input}\n  form = {prev}",
                    case.description
                );
            }
        }
    }
    let form = form.expect("at least one input");
    let canonical = canonical.expect("at least one input");

    let satisfiable = canonical.is_satisfiable();
    if let Some(expected) = case.satisfiable {
        assert_eq!(
            satisfiable, expected,
            "case `{}`: is_satisfiable() = {satisfiable}, expected {expected}\n  form = {form}",
            case.description
        );
    }
    if case.tests.iter().any(|test| test.valid == Some(true)) {
        assert!(
            satisfiable,
            "case `{}`: admits a valid value but is_satisfiable() is false\n  form = {form}",
            case.description
        );
    }

    if let Some(expected) = &case.expected {
        assert_eq!(
            &form, expected,
            "case `{}`: canonical form mismatch\n  expected = {expected}\n  actual   = {form}",
            case.description
        );
    }

    // Raw and canonical validators must agree on every test value.
    if !case.tests.is_empty() {
        let canonical_validator = case.build(&form, form_draft);
        let raw_validators: Vec<_> = inputs
            .iter()
            .map(|input| (input, case.build(input, raw_draft)))
            .collect();
        for test in &case.tests {
            let canon_valid = canonical_validator.is_valid(&test.data);
            for (input, raw_validator) in &raw_validators {
                let raw_valid = raw_validator.is_valid(&test.data);
                assert_eq!(
                    raw_valid, canon_valid,
                    "case `{}`: parity disagrees on {}\n  raw   ({input}) = {raw_valid}\n  canon ({form}) = {canon_valid}",
                    case.description, test.data
                );
                if let Some(expected_valid) = test.valid {
                    assert_eq!(
                        raw_valid, expected_valid,
                        "case `{}`: raw verdict wrong on {}: expected {expected_valid}, got {raw_valid}",
                        case.description, test.data
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
    tests: Vec<SchemaTest>,
    /// Pins `CanonicalSchema::is_satisfiable()`; a `valid: true` test already implies satisfiable.
    #[serde(default)]
    satisfiable: Option<bool>,
    /// Force `should_validate_formats`; `None` keeps the draft default.
    #[serde(default)]
    validate_formats: Option<bool>,
    /// Every input must be rejected with this exact `CanonicalizationError` variant.
    #[serde(default)]
    error: Option<String>,
}

fn error_kind(error: &CanonicalizationError) -> &'static str {
    match error {
        CanonicalizationError::InvalidSchemaType(_) => "InvalidSchemaType",
        CanonicalizationError::ValidationError(_) => "ValidationError",
        CanonicalizationError::InvalidPattern { .. } => "InvalidPattern",
        _ => "Unknown",
    }
}

/// Plain JSON value (parity-only) or `{"data": ..., "valid": bool}` (also checks verdict).
#[derive(Debug)]
struct SchemaTest {
    data: Value,
    valid: Option<bool>,
}

impl<'de> Deserialize<'de> for SchemaTest {
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
                Ok(SchemaTest { data, valid })
            }
            _ => Ok(SchemaTest {
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
