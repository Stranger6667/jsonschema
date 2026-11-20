#![allow(clippy::float_cmp, clippy::cast_sign_loss)]

use crate::LazyInstance;
use std::borrow::Cow;

use serde_json::{Map, Value};

use crate::{
    compiler::{self, DeferredAbsoluteLocation},
    evaluation::{
        format_keyword_location, Annotations, ChildList, ErrorDescription, EvaluationNode,
        InstancePosition,
    },
    paths::{LazyLocation, Location, RefTracker},
    types::JsonType,
    validator::ValidationContext,
    Json, Node, ValidationError,
};
use referencing::Uri;
use std::sync::{Arc, OnceLock};

/// A non-negative bound on a collection size, saturating at `u64::MAX`.
///
/// Sizes are `usize`-bounded, so a bound at or past `u64::MAX` rules out nothing that `u64::MAX`
/// already allows: `max*` still accepts every instance that can exist, `min*` still rejects every
/// one. Saturating keeps the runtime check a plain `u64` compare rather than a bignum.
///
/// `None` for anything that is not a non-negative integer under this draft; the caller turns that
/// into a compilation error.
pub(crate) fn size_limit<F: Json>(ctx: &compiler::Context<F>, schema: &Value) -> Option<u64> {
    if let Some(limit) = schema.as_u64() {
        return Some(limit);
    }
    if is_integer_past_u64(schema) {
        return Some(u64::MAX);
    }
    // Draft 4 writes a bound as an integer literal; later drafts also read integer-valued floats.
    if ctx.supports_integer_valued_numbers() {
        // Anything past `u64` was handled above, so the cast cannot lose the value.
        #[allow(clippy::cast_possible_truncation)]
        if let Some(limit) = schema
            .as_f64()
            .filter(|limit| *limit >= 0.0 && limit.trunc() == *limit)
        {
            return Some(limit as u64);
        }
    }
    None
}

/// Whether the value is an integer too large for `u64`.
#[cfg(feature = "arbitrary-precision")]
fn is_integer_past_u64(schema: &Value) -> bool {
    // The raw JSON text survives here, so a bare digit run is exactly an integer literal, and
    // `as_u64` already claimed every one that fits.
    match schema {
        Value::Number(number) => number.as_str().bytes().all(|byte| byte.is_ascii_digit()),
        _ => false,
    }
}

/// Whether the value is an integer too large for `u64`.
#[cfg(not(feature = "arbitrary-precision"))]
fn is_integer_past_u64(schema: &Value) -> bool {
    // Without the raw text such a value is an `f64`, and every `f64` at or past `2^64` is an exact
    // integer; Draft 4's integer-literal rule cannot be checked here.
    schema
        .as_f64()
        .is_some_and(|value| value >= crate::canonical::json::U64_UPPER_EXCLUSIVE_F64)
}

/// Extract a u64 value from a schema map, returning a compilation error if invalid.
///
/// Reached when meta-validation is off or the schema is a registry resource.
#[inline]
pub(crate) fn map_get_u64<'a, F: Json>(
    m: &'a Map<String, Value>,
    ctx: &compiler::Context<F>,
    keyword: &str,
) -> Option<Result<u64, ValidationError<'a>>> {
    let schema_value = m.get(keyword)?;
    Some(match size_limit(ctx, schema_value) {
        Some(limit) => Ok(limit),
        None => Err(fail_on_non_positive_integer(
            schema_value,
            ctx.location().join(keyword),
        )),
    })
}

/// Create a compilation error for schema values that must be non-negative integers.
///
/// Reached when meta-validation is off or the schema is a registry resource.
pub(crate) fn fail_on_non_positive_integer(
    schema_value: &Value,
    schema_path: Location,
) -> ValidationError<'_> {
    if schema_value.is_i64() {
        // Negative integer
        ValidationError::minimum(
            schema_path.clone(),
            schema_path,
            Location::new(),
            LazyInstance::Ready(Cow::Borrowed(schema_value)),
            0.into(),
        )
    } else {
        // Wrong type (string, object, etc.)
        ValidationError::single_type_error(
            schema_path.clone(),
            schema_path,
            Location::new(),
            LazyInstance::Ready(Cow::Borrowed(schema_value)),
            JsonType::Integer,
        )
    }
}

/// A schema position with the locations its output reports, rendered on first use.
pub(crate) struct SchemaSite {
    pub(crate) location: Location,
    base: Option<DeferredAbsoluteLocation>,
    absolute_location: OnceLock<Option<Arc<Uri<String>>>>,
    schema_location: OnceLock<Arc<str>>,
}

impl SchemaSite {
    pub(crate) fn new<F: Json>(ctx: &compiler::Context<F>, location: Location) -> Self {
        SchemaSite {
            location,
            base: ctx.deferred_absolute_location(),
            absolute_location: OnceLock::new(),
            schema_location: OnceLock::new(),
        }
    }

    pub(crate) fn absolute_location(&self) -> Option<&Arc<Uri<String>>> {
        self.absolute_location
            .get_or_init(|| self.base.as_ref().map(|base| base.resolve(&self.location)))
            .as_ref()
    }

    pub(crate) fn schema_location(&self) -> Arc<str> {
        Arc::clone(
            self.schema_location
                .get_or_init(|| format_keyword_location(&self.location, self.absolute_location())),
        )
    }
}

/// A leaf keyword's output unit: valid and empty, or failing with `error`.
pub(crate) fn unit(
    site: &SchemaSite,
    error: Option<ErrorDescription>,
    instance_location: InstancePosition,
    tracker: Option<&RefTracker>,
    ctx: &mut ValidationContext,
) -> EvaluationNode {
    unit_over(
        site,
        error.into_iter().collect(),
        None,
        ChildList::default(),
        instance_location,
        tracker,
        ctx,
    )
}

/// A `false` subschema failing at the instance at `path`, as a unit at `site`.
pub(crate) fn false_schema_unit<F: Json>(
    ctx: &mut ValidationContext,
    site: &SchemaSite,
    value: &F::Node<'_>,
    path: &LazyLocation,
    tracker: Option<&RefTracker>,
) -> EvaluationNode {
    let error = ErrorDescription::from_validation_error(&ValidationError::false_schema(
        site.location.clone(),
        crate::paths::capture_evaluation_path(tracker, &site.location),
        path.into(),
        value.lazy_value(),
    ));
    let position = ctx.instance_position(path);
    unit_over(
        site,
        vec![error],
        None,
        ChildList::default(),
        position,
        tracker,
        ctx,
    )
}

/// A keyword's output unit over `children`, failing with `errors` or with a failing child.
pub(crate) fn unit_over(
    site: &SchemaSite,
    errors: Vec<ErrorDescription>,
    annotations: Option<Annotations>,
    children: ChildList,
    instance_location: InstancePosition,
    tracker: Option<&RefTracker>,
    ctx: &mut ValidationContext,
) -> EvaluationNode {
    let evaluation_path = crate::paths::evaluation_path(tracker, &site.location, ctx);
    if errors.is_empty() && children.all_valid() {
        EvaluationNode::valid(
            evaluation_path,
            site.absolute_location().cloned(),
            site.schema_location(),
            instance_location,
            annotations,
            children,
        )
    } else {
        EvaluationNode::invalid(
            evaluation_path,
            site.absolute_location().cloned(),
            site.schema_location(),
            instance_location,
            annotations,
            errors,
            children,
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use serde_json::{json, Value};
    use test_case::test_case;

    #[test_case(&json!({"maxItems": -1}), "-1 is less than the minimum of 0", "/maxItems"; "max items negative")]
    #[test_case(&json!({"maxItems": "a"}), "\"a\" is not of type \"integer\"", "/maxItems"; "max items not integer")]
    #[test_case(&json!({"minItems": -1}), "-1 is less than the minimum of 0", "/minItems"; "min items negative")]
    #[test_case(&json!({"maxLength": -1}), "-1 is less than the minimum of 0", "/maxLength"; "max length negative")]
    #[test_case(&json!({"minLength": "a"}), "\"a\" is not of type \"integer\"", "/minLength"; "min length not integer")]
    #[test_case(&json!({"maxProperties": -1}), "-1 is less than the minimum of 0", "/maxProperties"; "max properties negative")]
    #[test_case(&json!({"minProperties": 1.5}), "1.5 is not of type \"integer\"", "/minProperties"; "min properties fractional")]
    #[test_case(&json!({"contains": {}, "minContains": -1}), "-1 is less than the minimum of 0", "/minContains"; "min contains negative")]
    #[test_case(&json!({"contains": {}, "maxContains": "a"}), "\"a\" is not of type \"integer\"", "/maxContains"; "max contains not integer")]
    fn malformed_size_limit(schema: &Value, message: &str, location: &str) {
        tests_util::assert_compile_error(schema, message, location);
    }
}
