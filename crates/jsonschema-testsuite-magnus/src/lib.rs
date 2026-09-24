use std::{collections::HashMap, sync::LazyLock};

use magnus::{function, Error, Ruby, Value};

pub struct SuiteEntry {
    pub id: &'static str,
    pub is_valid: fn(&Value) -> Result<bool, Error>,
    pub validate: fn(&Value) -> Result<Option<String>, Error>,
    pub iter_errors: fn(&Value) -> Result<Vec<String>, Error>,
}

testsuite::magnus_suite!(
    path = "crates/jsonschema/tests/suite",
    drafts = ["draft4", "draft6", "draft7", "draft2019-09", "draft2020-12"]
);
testsuite::magnus_schemas!("crates/jsonschema-rb/spec/codegen_schemas.json");

static BY_ID: LazyLock<HashMap<&'static str, &'static SuiteEntry>> = LazyLock::new(|| {
    SUITE_ENTRIES
        .iter()
        .chain(SCHEMA_ENTRIES)
        .map(|entry| (entry.id, entry))
        .collect()
});

fn entry(ruby: &Ruby, case_id: &str) -> Result<&'static SuiteEntry, Error> {
    BY_ID.get(case_id).copied().ok_or_else(|| {
        Error::new(
            ruby.exception_key_error(),
            format!("No compiled validator for {case_id}"),
        )
    })
}

// `function!` binds owned arguments.
#[allow(clippy::needless_pass_by_value)]
fn is_valid(ruby: &Ruby, case_id: String, instance: Value) -> Result<bool, Error> {
    (entry(ruby, &case_id)?.is_valid)(&instance)
}

#[allow(clippy::needless_pass_by_value)]
fn validate(ruby: &Ruby, case_id: String, instance: Value) -> Result<Option<String>, Error> {
    (entry(ruby, &case_id)?.validate)(&instance)
}

#[allow(clippy::needless_pass_by_value)]
fn each_error(ruby: &Ruby, case_id: String, instance: Value) -> Result<Vec<String>, Error> {
    (entry(ruby, &case_id)?.iter_errors)(&instance)
}

#[magnus::init(name = "jsonschema_testsuite_magnus")]
fn init(ruby: &Ruby) -> Result<(), Error> {
    let module = ruby.define_module("JSONSchemaTestSuite")?;
    module.define_module_function("valid?", function!(is_valid, 2))?;
    module.define_module_function("validate", function!(validate, 2))?;
    module.define_module_function("each_error", function!(each_error, 2))?;
    Ok(())
}
