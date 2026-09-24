use std::{collections::HashMap, sync::LazyLock};

use jsonschema::{
    json::{Magnus, RbNode},
    paths::Location,
    Keyword, ValidationError,
};
use magnus::{
    function, rb_sys::FromRawValue, value::ReprValue, Error, Float, Integer, RArray, Ruby, Value,
};
use serde_json::{Map, Value as Json};

pub struct SuiteEntry {
    pub id: &'static str,
    pub is_valid: fn(&Value) -> Result<bool, Error>,
    pub validate: fn(&Value) -> Result<Option<String>, Error>,
    pub iter_errors: fn(&Value) -> Result<Vec<String>, Error>,
}

type KeywordResult<'a> = Result<Box<dyn for<'i> Keyword<'i, Magnus>>, ValidationError<'a>>;

#[allow(unsafe_code)]
fn ruby_value(node: RbNode<'_>) -> Value {
    unsafe { Value::from_raw(node.raw()) }
}

fn as_f64(value: Value) -> Option<f64> {
    if let Some(float) = Float::from_value(value) {
        return Some(float.to_f64());
    }
    #[allow(clippy::cast_precision_loss)]
    Integer::from_value(value).and_then(|integer| integer.to_i64().ok().map(|i| i as f64))
}

// Mirror `even_validator_class`, `range_validator_class` and `all_positive_validator_class` in
// `crates/jsonschema-rb/spec/jsonschema_spec.rb`.
struct Even(bool);

impl<'i> Keyword<'i, Magnus> for Even {
    fn validate(&self, instance: RbNode<'i>) -> Result<(), ValidationError<'i>> {
        if self.is_valid(instance) {
            Ok(())
        } else {
            Err(ValidationError::custom(format!(
                "{} is not even",
                ruby_value(instance)
            )))
        }
    }

    fn is_valid(&self, instance: RbNode<'i>) -> bool {
        let value = ruby_value(instance);
        !self.0
            || Integer::from_value(value).is_none()
            || !value.funcall::<_, _, bool>("odd?", ()).unwrap_or(false)
    }
}

// The keyword factory signature returns a `Result`.
#[allow(clippy::unnecessary_wraps)]
pub(crate) fn even<'a>(
    _: &'a Map<String, Json>,
    value: &'a Json,
    _: Location,
) -> KeywordResult<'a> {
    Ok(Box::new(Even(value.as_bool() == Some(true))))
}

struct CustomRange {
    min: f64,
    max: f64,
}

impl<'i> Keyword<'i, Magnus> for CustomRange {
    fn validate(&self, instance: RbNode<'i>) -> Result<(), ValidationError<'i>> {
        match as_f64(ruby_value(instance)) {
            Some(number) if !(self.min..=self.max).contains(&number) => {
                Err(ValidationError::custom(format!(
                    "Value {number} not in range [{}, {}]",
                    self.min, self.max
                )))
            }
            _ => Ok(()),
        }
    }

    fn is_valid(&self, instance: RbNode<'i>) -> bool {
        self.validate(instance).is_ok()
    }
}

#[allow(clippy::unnecessary_wraps)]
pub(crate) fn custom_range<'a>(
    _: &'a Map<String, Json>,
    value: &'a Json,
    _: Location,
) -> KeywordResult<'a> {
    Ok(Box::new(CustomRange {
        min: value["min"].as_f64().unwrap_or(f64::NEG_INFINITY),
        max: value["max"].as_f64().unwrap_or(f64::INFINITY),
    }))
}

struct AllPositive;

impl AllPositive {
    fn negative_items(instance: RbNode<'_>) -> Vec<ValidationError<'_>> {
        let Some(items) = RArray::from_value(ruby_value(instance)) else {
            return Vec::new();
        };
        (0..items.len())
            .filter(|index| {
                isize::try_from(*index)
                    .ok()
                    .and_then(|offset| items.entry::<Value>(offset).ok())
                    .and_then(|item| item.funcall::<_, _, bool>("negative?", ()).ok())
                    .unwrap_or(false)
            })
            .map(|index| ValidationError::custom(format!("item {index} is negative")))
            .collect()
    }
}

impl<'i> Keyword<'i, Magnus> for AllPositive {
    fn validate(&self, instance: RbNode<'i>) -> Result<(), ValidationError<'i>> {
        Self::negative_items(instance)
            .into_iter()
            .next()
            .map_or(Ok(()), Err)
    }

    fn is_valid(&self, instance: RbNode<'i>) -> bool {
        Self::negative_items(instance).is_empty()
    }

    fn iter_errors(
        &self,
        instance: RbNode<'i>,
    ) -> Box<dyn Iterator<Item = ValidationError<'i>> + 'i> {
        Box::new(Self::negative_items(instance).into_iter())
    }
}

#[allow(clippy::unnecessary_wraps)]
pub(crate) fn all_positive<'a>(
    _: &'a Map<String, Json>,
    _: &'a Json,
    _: Location,
) -> KeywordResult<'a> {
    Ok(Box::new(AllPositive))
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
