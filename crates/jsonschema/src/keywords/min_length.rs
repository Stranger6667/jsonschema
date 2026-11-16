use crate::{
    compiler,
    error::ValidationError,
    evaluation::{ChildList, ErrorDescription, InstancePosition},
    keywords::{
        helpers::{fail_on_non_positive_integer, size_limit, unit, SchemaSite},
        CompilationResult,
    },
    paths::{LazyLocation, Location, RefTracker},
    validator::{evaluate_assertion, EvaluationResult, Validate, ValidationContext},
    Json, Node,
};
use serde_json::{Map, Value};

pub(crate) struct MinLengthValidator {
    limit: u64,
    location: Location,
}

impl<F: Json> Validate<F> for MinLengthValidator {
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        instance.string_length_within(self.limit, u64::MAX) != Some(false)
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if instance.string_length_within(self.limit, u64::MAX) == Some(false) {
            return Err(ValidationError::min_length(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance.lazy_value(),
                self.limit,
            ));
        }
        Ok(())
    }

    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        _location: &LazyLocation,
        _tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        evaluate_assertion::<F, _>(self, instance, ctx)
    }
}

/// `minLength` and `maxLength` together. A length is a count of code points, so measuring it
/// walks the whole string; two validators walk it twice.
pub(crate) struct LengthRangeValidator {
    minimum: u64,
    maximum: u64,
    min_site: SchemaSite,
    max_site: SchemaSite,
}

impl LengthRangeValidator {
    fn min_error<'i, F: Json>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
    ) -> ValidationError<'i> {
        ValidationError::min_length(
            self.min_site.location.clone(),
            crate::paths::capture_evaluation_path(tracker, &self.min_site.location),
            location.into(),
            instance.lazy_value(),
            self.minimum,
        )
    }

    fn max_error<'i, F: Json>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
    ) -> ValidationError<'i> {
        let site = &self.max_site;
        ValidationError::max_length(
            site.location.clone(),
            crate::paths::capture_evaluation_path(tracker, &site.location),
            location.into(),
            instance.lazy_value(),
            self.maximum,
        )
        .with_absolute_keyword_location(site.absolute_location().cloned())
    }
}

impl<F: Json> Validate<F> for LengthRangeValidator {
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        instance.string_length_within(self.minimum, self.maximum) != Some(false)
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Some(length) = instance.string_length() {
            // `minLength` sorts first
            if length < self.minimum {
                return Err(self.min_error::<F>(instance, location, tracker));
            }
            if length > self.maximum {
                return Err(self.max_error::<F>(instance, location, tracker));
            }
        }
        Ok(())
    }

    fn collect_errors<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
        errors: &mut Vec<ValidationError<'i>>,
    ) {
        if let Some(length) = instance.string_length() {
            // `minLength` above `maxLength` fails both at once, as two validators would
            if length < self.minimum {
                errors.push(self.min_error::<F>(instance, location, tracker));
            }
            if length > self.maximum {
                errors.push(self.max_error::<F>(instance, location, tracker));
            }
        }
    }

    fn reports_units(&self) -> bool {
        true
    }

    fn evaluate_units(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        instance_location: InstancePosition,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
        children: &mut ChildList,
    ) -> bool {
        let length = instance.string_length();
        // A description depends on the error kind and the instance alone; see `Validate::evaluate`.
        let min_error = length.is_some_and(|length| length < self.minimum).then(|| {
            ErrorDescription::from_validation_error(
                &self.min_error::<F>(instance, location, tracker),
            )
        });
        let max_error = length.is_some_and(|length| length > self.maximum).then(|| {
            ErrorDescription::from_validation_error(
                &self.max_error::<F>(instance, location, tracker),
            )
        });
        let invalid = min_error.is_some() || max_error.is_some();
        let node = unit(&self.min_site, min_error, instance_location, tracker, ctx);
        children.push(ctx.arena(), node);
        let node = unit(&self.max_site, max_error, instance_location, tracker, ctx);
        children.push(ctx.arena(), node);
        invalid
    }
}

#[inline]
pub(crate) fn compile<'a, F: Json>(
    ctx: &compiler::Context<F>,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a, F>> {
    let location = ctx.location().join("minLength");
    let Some(minimum) = size_limit(ctx, schema) else {
        return Some(Err(fail_on_non_positive_integer(schema, location)));
    };
    // `max_length::compile` steps aside when it sees this pair
    let maximum = parent
        .get("maxLength")
        .filter(|_| !ctx.is_keyword_overridden("maxLength"))
        .and_then(|max| size_limit(ctx, max));
    if let Some(maximum) = maximum {
        return Some(Ok(Box::new(LengthRangeValidator {
            minimum,
            maximum,
            min_site: SchemaSite::new(ctx, location),
            max_site: SchemaSite::new(ctx, ctx.location().join("maxLength")),
        })));
    }
    Some(Ok(Box::new(MinLengthValidator {
        limit: minimum,
        location,
    })))
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use serde_json::{json, Value};
    use test_case::test_case;

    #[test]
    fn location() {
        tests_util::assert_schema_location(&json!({"minLength": 1}), &json!(""), "/minLength");
    }

    #[test]
    fn fused_locations() {
        let schema = json!({"minLength": 2, "maxLength": 4});
        tests_util::assert_schema_location(&schema, &json!("a"), "/minLength");
        tests_util::assert_schema_location(&schema, &json!("abcde"), "/maxLength");
    }

    #[test]
    fn fused_bounds_are_inclusive() {
        let schema = json!({"minLength": 2, "maxLength": 4});
        tests_util::is_valid(&schema, &json!("ab"));
        tests_util::is_valid(&schema, &json!("abcd"));
        tests_util::is_not_valid(&schema, &json!("a"));
        tests_util::is_not_valid(&schema, &json!("abcde"));
    }

    #[test]
    fn fused_ignores_non_strings() {
        let schema = json!({"minLength": 2, "maxLength": 4});
        tests_util::is_valid(&schema, &json!(1));
        tests_util::is_valid(&schema, &json!(null));
        tests_util::is_valid(&schema, &json!(["a"]));
    }

    #[test]
    fn fused_counts_code_points() {
        // 5 code points in 6 bytes; counting bytes would put it over
        tests_util::is_valid(&json!({"minLength": 5, "maxLength": 5}), &json!("héllo"));
        // 2 code points in 8 bytes
        tests_util::is_valid(&json!({"minLength": 2, "maxLength": 2}), &json!("💩💩"));
        tests_util::is_not_valid(&json!({"minLength": 3, "maxLength": 4}), &json!("💩💩"));
    }

    #[test]
    fn unsatisfiable_range_reports_both() {
        // Not `assert_locations`: its `zip` would pass on a single error.
        let validator = crate::validator_for(&json!({"minLength": 5, "maxLength": 3})).unwrap();
        let mut locations: Vec<String> = validator
            .iter_errors(&json!("abcd"))
            .map(|error| error.schema_path().as_str().to_string())
            .collect();
        locations.sort();
        assert_eq!(locations, ["/maxLength", "/minLength"]);
    }

    #[test]
    fn invalid_sibling_still_reported() {
        // An unusable bound keeps its own error instead of being folded away.
        assert!(crate::validator_for(&json!({"minLength": 1, "maxLength": -1})).is_err());
        assert!(crate::validator_for(&json!({"minLength": -1, "maxLength": 1})).is_err());
    }

    #[test]
    fn fused_absolute_keyword_locations() {
        tests_util::assert_absolute_keyword_locations(
            &json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "$id": "https://example.com/s.json",
                "minLength": 20,
                "maxLength": 5
            }),
            &json!("secretvalue"),
            &[
                ("minLength", "https://example.com/s.json#/minLength"),
                ("maxLength", "https://example.com/s.json#/maxLength"),
            ],
        );
    }

    #[test]
    fn fused_evaluate_keyword_locations() {
        let validator = crate::validator_for(&json!({"minLength": 20, "maxLength": 5}))
            .expect("Invalid schema");
        let instance = json!("secretvalue");
        tests_util::assert_keyword_location(&validator, &instance, "", "/minLength");
        tests_util::assert_keyword_location(&validator, &instance, "", "/maxLength");
    }

    #[test]
    fn fused_error_locations() {
        let schema = json!({"minLength": 2, "maxLength": 3});
        tests_util::assert_error_locations(
            &schema,
            &json!("a"),
            &[("\"a\" is shorter than 2 characters", "", "/minLength")],
        );
        tests_util::assert_error_locations(
            &schema,
            &json!("abcd"),
            &[("\"abcd\" is longer than 3 characters", "", "/maxLength")],
        );
        tests_util::assert_error_locations(
            &json!({"minLength": 20, "maxLength": 5}),
            &json!("secretvalue"),
            &[
                (
                    "\"secretvalue\" is shorter than 20 characters",
                    "",
                    "/minLength",
                ),
                (
                    "\"secretvalue\" is longer than 5 characters",
                    "",
                    "/maxLength",
                ),
            ],
        );
    }

    // Both units, as `minLength` and `maxLength` report them on their own.
    #[test_case(&json!({"minLength": 2, "maxLength": 3}), &json!("ab"), None, None ; "valid")]
    #[test_case(&json!({"minLength": 2, "maxLength": 3}), &json!("a"), Some("\"a\" is shorter than 2 characters"), None ; "too short")]
    #[test_case(&json!({"minLength": 2, "maxLength": 3}), &json!("abcd"), None, Some("\"abcd\" is longer than 3 characters") ; "too long")]
    #[test_case(&json!({"minLength": 5, "maxLength": 3}), &json!("abcd"), Some("\"abcd\" is shorter than 5 characters"), Some("\"abcd\" is longer than 3 characters") ; "unsatisfiable")]
    #[test_case(&json!({"minLength": 2, "maxLength": 3}), &json!(5), None, None ; "not a string")]
    fn fused_evaluate_reports_sibling_units(
        schema: &Value,
        instance: &Value,
        min_error: Option<&str>,
        max_error: Option<&str>,
    ) {
        let unit = |keyword: &str, error: Option<&str>| {
            let mut unit = json!({
                "evaluationPath": format!("/{keyword}"),
                "instanceLocation": "",
                "schemaLocation": format!("/{keyword}"),
                "valid": error.is_none(),
            });
            if let Some(error) = error {
                unit["errors"] = json!({ keyword: error });
            }
            unit
        };
        let valid = min_error.is_none() && max_error.is_none();
        let evaluation = crate::validator_for(schema)
            .expect("valid schema")
            .evaluate(instance);
        let root = json!({"evaluationPath": "", "instanceLocation": "", "schemaLocation": "", "valid": valid});
        let mut list_details = vec![root.clone()];
        list_details.push(unit("minLength", min_error));
        list_details.push(unit("maxLength", max_error));
        assert_eq!(
            serde_json::to_value(evaluation.list()).expect("list output"),
            json!({"valid": valid, "details": list_details}),
        );
        let mut hierarchical = root;
        hierarchical["details"] =
            json!([unit("minLength", min_error), unit("maxLength", max_error)]);
        assert_eq!(
            serde_json::to_value(evaluation.hierarchical()).expect("hierarchical output"),
            hierarchical,
        );
    }
}
