use std::sync::Arc;

use crate::{
    compiler,
    error::{no_error, ErrorIterator, ValidationError},
    evaluation::Annotations,
    keywords::CompilationResult,
    node::SchemaNode,
    options::PatternEngineOptions,
    paths::{LazyEvaluationPath, LazyLocation, Location, RefTracker},
    regex::{pattern_as_prefix, RegexEngine},
    types::JsonType,
    validator::{EvaluationResult, Validate, ValidationContext},
};
use serde_json::{Map, Value};

/// Validator for multiple patterns using compiled regex.
pub(crate) struct PatternPropertiesValidator<R> {
    patterns: Vec<(Arc<R>, SchemaNode)>,
}

/// Validator for patterns that are all simple prefixes (optimized path).
pub(crate) struct PrefixPatternPropertiesValidator {
    prefixes: Vec<(Arc<str>, SchemaNode)>,
}

impl Validate for PrefixPatternPropertiesValidator {
    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        if let Value::Object(item) = instance {
            for (prefix, node) in &self.prefixes {
                for (key, value) in item {
                    if key.starts_with(prefix.as_ref()) && !node.is_valid(value, ctx) {
                        return false;
                    }
                }
            }
            true
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Object(item) = instance {
            for (key, value) in item {
                for (prefix, node) in &self.prefixes {
                    if key.starts_with(prefix.as_ref()) {
                        node.validate(value, &location.push(key), tracker, ctx)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn iter_errors<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Value::Object(item) = instance {
            let mut errors = Vec::new();
            for (prefix, node) in &self.prefixes {
                for (key, value) in item {
                    if key.starts_with(prefix.as_ref()) {
                        errors.extend(node.iter_errors(
                            value,
                            &location.push(key.as_str()),
                            tracker,
                            ctx,
                        ));
                    }
                }
            }
            ErrorIterator::from_iterator(errors.into_iter())
        } else {
            no_error()
        }
    }

    fn evaluate(
        &self,
        instance: &Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Value::Object(item) = instance {
            let mut matched_propnames = Vec::with_capacity(item.len());
            let mut children = Vec::new();
            for (prefix, node) in &self.prefixes {
                for (key, value) in item {
                    if key.starts_with(prefix.as_ref()) {
                        matched_propnames.push(key.clone());
                        children.push(node.evaluate_instance(
                            value,
                            &location.push(key.as_str()),
                            tracker,
                            ctx,
                        ));
                    }
                }
            }
            let mut result = EvaluationResult::from_children(children);
            result.annotate(Annotations::new(Value::from(matched_propnames)));
            result
        } else {
            EvaluationResult::valid_empty()
        }
    }
}

/// Validator for a single prefix pattern (optimized path).
pub(crate) struct SinglePrefixPatternPropertiesValidator {
    prefix: Arc<str>,
    node: SchemaNode,
}

impl Validate for SinglePrefixPatternPropertiesValidator {
    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        if let Value::Object(item) = instance {
            for (key, value) in item {
                if key.starts_with(self.prefix.as_ref()) && !self.node.is_valid(value, ctx) {
                    return false;
                }
            }
            true
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Object(item) = instance {
            for (key, value) in item {
                if key.starts_with(self.prefix.as_ref()) {
                    self.node
                        .validate(value, &location.push(key), tracker, ctx)?;
                }
            }
        }
        Ok(())
    }

    fn iter_errors<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Value::Object(item) = instance {
            let mut errors = Vec::new();
            for (key, value) in item {
                if key.starts_with(self.prefix.as_ref()) {
                    errors.extend(self.node.iter_errors(
                        value,
                        &location.push(key.as_str()),
                        tracker,
                        ctx,
                    ));
                }
            }
            ErrorIterator::from_iterator(errors.into_iter())
        } else {
            no_error()
        }
    }

    fn evaluate(
        &self,
        instance: &Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Value::Object(item) = instance {
            let mut matched_propnames = Vec::with_capacity(item.len());
            let mut children = Vec::new();
            for (key, value) in item {
                if key.starts_with(self.prefix.as_ref()) {
                    matched_propnames.push(key.clone());
                    children.push(self.node.evaluate_instance(
                        value,
                        &location.push(key.as_str()),
                        tracker,
                        ctx,
                    ));
                }
            }
            let mut result = EvaluationResult::from_children(children);
            result.annotate(Annotations::new(Value::from(matched_propnames)));
            result
        } else {
            EvaluationResult::valid_empty()
        }
    }
}

impl<R: RegexEngine> Validate for PatternPropertiesValidator<R> {
    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        if let Value::Object(item) = instance {
            for (re, node) in &self.patterns {
                for (key, value) in item {
                    if re.is_match(key).unwrap_or(false) && !node.is_valid(value, ctx) {
                        return false;
                    }
                }
            }
            true
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Object(item) = instance {
            for (key, value) in item {
                for (re, node) in &self.patterns {
                    if re.is_match(key).unwrap_or(false) {
                        node.validate(value, &location.push(key), tracker, ctx)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn iter_errors<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Value::Object(item) = instance {
            let mut errors = Vec::new();
            for (re, node) in &self.patterns {
                for (key, value) in item {
                    if re.is_match(key).unwrap_or(false) {
                        errors.extend(node.iter_errors(
                            value,
                            &location.push(key.as_str()),
                            tracker,
                            ctx,
                        ));
                    }
                }
            }
            ErrorIterator::from_iterator(errors.into_iter())
        } else {
            no_error()
        }
    }

    fn evaluate(
        &self,
        instance: &Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Value::Object(item) = instance {
            let mut matched_propnames = Vec::with_capacity(item.len());
            let mut children = Vec::new();
            for (pattern, node) in &self.patterns {
                for (key, value) in item {
                    if pattern.is_match(key).unwrap_or(false) {
                        matched_propnames.push(key.clone());
                        children.push(node.evaluate_instance(
                            value,
                            &location.push(key.as_str()),
                            tracker,
                            ctx,
                        ));
                    }
                }
            }
            let mut result = EvaluationResult::from_children(children);
            result.annotate(Annotations::new(Value::from(matched_propnames)));
            result
        } else {
            EvaluationResult::valid_empty()
        }
    }
}

pub(crate) struct SingleValuePatternPropertiesValidator<R> {
    regex: Arc<R>,
    node: SchemaNode,
}

impl<R: RegexEngine> Validate for SingleValuePatternPropertiesValidator<R> {
    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        if let Value::Object(item) = instance {
            for (key, value) in item {
                if self.regex.is_match(key).unwrap_or(false) && !self.node.is_valid(value, ctx) {
                    return false;
                }
            }
            true
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Object(item) = instance {
            for (key, value) in item {
                if self.regex.is_match(key).unwrap_or(false) {
                    self.node
                        .validate(value, &location.push(key), tracker, ctx)?;
                }
            }
        }
        Ok(())
    }

    fn iter_errors<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Value::Object(item) = instance {
            let mut errors = Vec::new();
            for (key, value) in item {
                if self.regex.is_match(key).unwrap_or(false) {
                    errors.extend(self.node.iter_errors(
                        value,
                        &location.push(key.as_str()),
                        tracker,
                        ctx,
                    ));
                }
            }
            ErrorIterator::from_iterator(errors.into_iter())
        } else {
            no_error()
        }
    }

    fn evaluate(
        &self,
        instance: &Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Value::Object(item) = instance {
            let mut matched_propnames = Vec::with_capacity(item.len());
            let mut children = Vec::new();
            for (key, value) in item {
                if self.regex.is_match(key).unwrap_or(false) {
                    matched_propnames.push(key.clone());
                    children.push(self.node.evaluate_instance(
                        value,
                        &location.push(key.as_str()),
                        tracker,
                        ctx,
                    ));
                }
            }
            let mut result = EvaluationResult::from_children(children);
            result.annotate(Annotations::new(Value::from(matched_propnames)));
            result
        } else {
            EvaluationResult::valid_empty()
        }
    }
}

#[inline]
pub(crate) fn compile<'a>(
    ctx: &compiler::Context,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>> {
    if matches!(
        parent.get("additionalProperties"),
        Some(Value::Bool(false) | Value::Object(_))
    ) {
        // This type of `additionalProperties` validator handles `patternProperties` logic
        return None;
    }

    let Value::Object(map) = schema else {
        let location = ctx.location().join("patternProperties");
        return Some(Err(ValidationError::single_type_error(
            location.clone(),
            location,
            Location::new(),
            schema,
            JsonType::Object,
        )));
    };
    let ctx = ctx.new_at_location("patternProperties");

    // Try to compile all patterns as prefixes first (optimized path)
    if let Some(validator) = try_compile_as_prefixes(&ctx, map) {
        return Some(validator);
    }

    // Fall back to regex compilation
    let result = match ctx.config().pattern_options() {
        PatternEngineOptions::FancyRegex { .. } => {
            compile_pattern_entries(&ctx, map, |pctx, pattern, subschema| {
                pctx.get_or_compile_regex(pattern)
                    .map_err(|()| invalid_regex(pctx, subschema))
            })
            .map(|patterns| {
                build_validator_from_entries(patterns, |regex, node| {
                    Box::new(SingleValuePatternPropertiesValidator { regex, node })
                        as Box<dyn Validate>
                })
            })
        }
        PatternEngineOptions::Regex { .. } => {
            compile_pattern_entries(&ctx, map, |pctx, pattern, subschema| {
                pctx.get_or_compile_standard_regex(pattern)
                    .map_err(|()| invalid_regex(pctx, subschema))
            })
            .map(|patterns| {
                build_validator_from_entries(patterns, |regex, node| {
                    Box::new(SingleValuePatternPropertiesValidator { regex, node })
                        as Box<dyn Validate>
                })
            })
        }
    };
    Some(result)
}

/// Try to compile all patterns as simple prefixes.
/// Returns `Some` if ALL patterns are simple prefixes, `None` otherwise.
fn try_compile_as_prefixes<'a>(
    ctx: &compiler::Context,
    map: &'a Map<String, Value>,
) -> Option<CompilationResult<'a>> {
    let mut prefixes = Vec::with_capacity(map.len());

    for (pattern, subschema) in map {
        let prefix = pattern_as_prefix(pattern)?;
        let pctx = ctx.new_at_location(pattern.as_str());
        let node = match compiler::compile(&pctx, pctx.as_resource_ref(subschema)) {
            Ok(node) => node,
            Err(e) => return Some(Err(e)),
        };
        prefixes.push((Arc::from(prefix), node));
    }

    let validator: Box<dyn Validate> = if prefixes.len() == 1 {
        let (prefix, node) = prefixes.pop().expect("len checked");
        Box::new(SinglePrefixPatternPropertiesValidator { prefix, node })
    } else {
        Box::new(PrefixPatternPropertiesValidator { prefixes })
    };

    Some(Ok(validator))
}

fn invalid_regex<'a>(ctx: &compiler::Context, schema: &'a Value) -> ValidationError<'a> {
    ValidationError::format(
        ctx.location().clone(),
        LazyEvaluationPath::SameAsSchemaPath,
        Location::new(),
        schema,
        "regex",
    )
}

/// Compile every `(pattern, subschema)` pair into `(regex, node)` tuples.
fn compile_pattern_entries<'a, R, F>(
    ctx: &compiler::Context,
    map: &'a Map<String, Value>,
    mut compile_regex: F,
) -> Result<Vec<(Arc<R>, SchemaNode)>, ValidationError<'a>>
where
    F: FnMut(&compiler::Context, &str, &'a Value) -> Result<Arc<R>, ValidationError<'a>>,
{
    let mut patterns = Vec::with_capacity(map.len());
    for (pattern, subschema) in map {
        let pctx = ctx.new_at_location(pattern.as_str());
        let regex = compile_regex(&pctx, pattern, subschema)?;
        let node = compiler::compile(&pctx, pctx.as_resource_ref(subschema))?;
        patterns.push((regex, node));
    }
    Ok(patterns)
}

/// Pick the optimal validator representation for the compiled pattern entries.
fn build_validator_from_entries<R>(
    mut entries: Vec<(Arc<R>, SchemaNode)>,
    single_factory: impl FnOnce(Arc<R>, SchemaNode) -> Box<dyn Validate>,
) -> Box<dyn Validate>
where
    R: RegexEngine + 'static,
{
    if entries.len() == 1 {
        let (regex, node) = entries.pop().expect("len checked");
        single_factory(regex, node)
    } else {
        Box::new(PatternPropertiesValidator { patterns: entries })
    }
}

#[cfg(test)]
mod tests {
    use crate::{regex::pattern_as_prefix, tests_util};
    use serde_json::{json, Value};
    use test_case::test_case;

    #[test_case(&json!({"patternProperties": {"^f": {"type": "string"}}}), &json!({"f": 42}), "/patternProperties/^f/type")]
    #[test_case(&json!({"patternProperties": {"^f": {"type": "string"}, "^x": {"type": "string"}}}), &json!({"f": 42}), "/patternProperties/^f/type")]
    fn location(schema: &Value, instance: &Value, expected: &str) {
        tests_util::assert_schema_location(schema, instance, expected);
    }

    // Invalid regex in `patternProperties` without `additionalProperties`
    #[test_case(&json!({"patternProperties": {"[invalid": {"type": "string"}}}))]
    // Invalid regex with `additionalProperties: true` (default behavior)
    #[test_case(&json!({"additionalProperties": true, "patternProperties": {"[invalid": {"type": "string"}}}))]
    fn invalid_regex_fancy_regex(schema: &Value) {
        let error = crate::validator_for(schema).expect_err("Should fail to compile");
        assert!(error.to_string().contains("regex"));
    }

    #[test_case(&json!({"patternProperties": {"[invalid": {"type": "string"}}}))]
    #[test_case(&json!({"additionalProperties": true, "patternProperties": {"[invalid": {"type": "string"}}}))]
    fn invalid_regex_standard_regex(schema: &Value) {
        use crate::PatternOptions;

        let error = crate::options()
            .with_pattern_options(PatternOptions::regex())
            .build(schema)
            .expect_err("Should fail to compile");
        assert!(error.to_string().contains("regex"));
    }

    #[test_case("^foo", Some("foo"))]
    #[test_case("^x-", Some("x-"))]
    #[test_case("^eo_band", Some("eo_band"))]
    #[test_case("^path/to", Some("path/to"))]
    #[test_case("^ABC123", Some("ABC123"))]
    #[test_case("foo", None; "no anchor")]
    #[test_case("^foo$", None; "end anchor")]
    #[test_case("^foo.*", None; "contains dot")]
    #[test_case("^foo+", None; "contains plus")]
    #[test_case("^foo?", None; "contains question")]
    #[test_case("^[a-z]", None; "contains bracket")]
    #[test_case("^foo|bar", None; "contains pipe")]
    #[test_case("^foo(bar)", None; "contains parens")]
    #[test_case("^foo\\d", None; "contains backslash")]
    fn test_pattern_as_prefix(pattern: &str, expected: Option<&str>) {
        assert_eq!(pattern_as_prefix(pattern), expected);
    }

    // Test that prefix optimization works correctly for validation
    #[test_case("^x-", "x-custom", true)]
    #[test_case("^x-", "custom", false)]
    #[test_case("^eo_", "eo_bands", true)]
    #[test_case("^eo_", "proj_epsg", false)]
    fn test_prefix_pattern_validation(pattern: &str, key: &str, should_match: bool) {
        let schema = json!({
            "patternProperties": {
                pattern: {"type": "string"}
            }
        });
        let validator = crate::validator_for(&schema).unwrap();

        // If key matches pattern, value must be string
        let valid_instance = json!({ key: "value" });
        assert!(validator.is_valid(&valid_instance));

        let invalid_instance = json!({ key: 42 });
        assert_eq!(validator.is_valid(&invalid_instance), !should_match);
    }

    // Test multiple prefix patterns
    #[test]
    fn test_multiple_prefix_patterns() {
        let schema = json!({
            "patternProperties": {
                "^x-": {"type": "string"},
                "^y-": {"type": "number"}
            }
        });
        let validator = crate::validator_for(&schema).unwrap();

        assert!(validator.is_valid(&json!({"x-foo": "bar", "y-baz": 42})));
        assert!(!validator.is_valid(&json!({"x-foo": 123}))); // x- must be string
        assert!(!validator.is_valid(&json!({"y-baz": "str"}))); // y- must be number
    }

    // iter_errors tests for prefix patterns
    #[test]
    fn test_prefix_iter_errors_valid() {
        let schema = json!({
            "patternProperties": {
                "^x-": {"type": "string"}
            }
        });
        let validator = crate::validator_for(&schema).unwrap();

        // Valid: no errors
        let instance = json!({"x-foo": "bar"});
        let errors: Vec<_> = validator.iter_errors(&instance).collect();
        assert!(errors.is_empty());

        // Valid: non-matching key is ignored
        let instance = json!({"other": 42});
        let errors: Vec<_> = validator.iter_errors(&instance).collect();
        assert!(errors.is_empty());
    }

    #[test]
    fn test_prefix_iter_errors_invalid() {
        let schema = json!({
            "patternProperties": {
                "^x-": {"type": "string"}
            }
        });
        let validator = crate::validator_for(&schema).unwrap();

        // Invalid: wrong type
        let instance = json!({"x-foo": 42});
        let errors: Vec<_> = validator.iter_errors(&instance).collect();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].to_string().contains("not of type"));
    }

    #[test]
    fn test_prefix_iter_errors_multiple_failures() {
        let schema = json!({
            "patternProperties": {
                "^x-": {"type": "string"},
                "^y-": {"type": "number"}
            }
        });
        let validator = crate::validator_for(&schema).unwrap();

        // Multiple errors
        let instance = json!({"x-a": 1, "y-b": "str"});
        let errors: Vec<_> = validator.iter_errors(&instance).collect();
        assert_eq!(errors.len(), 2);
    }

    // evaluate tests for prefix patterns
    #[test]
    fn test_prefix_evaluate_valid() {
        let schema = json!({
            "patternProperties": {
                "^x-": {"type": "string"}
            }
        });
        let validator = crate::validator_for(&schema).unwrap();

        let instance = json!({"x-foo": "bar"});
        let result = validator.evaluate(&instance);
        assert!(result.flag().valid);
    }

    #[test]
    fn test_prefix_evaluate_invalid() {
        let schema = json!({
            "patternProperties": {
                "^x-": {"type": "string"}
            }
        });
        let validator = crate::validator_for(&schema).unwrap();

        let instance = json!({"x-foo": 42});
        let result = validator.evaluate(&instance);
        assert!(!result.flag().valid);
    }

    #[test]
    fn test_prefix_evaluate_annotations() {
        let schema = json!({
            "patternProperties": {
                "^x-": {"type": "string"}
            }
        });
        let validator = crate::validator_for(&schema).unwrap();

        // Valid case should have annotations for matched properties
        let instance = json!({"x-foo": "bar", "x-baz": "qux", "other": 123});
        let result = validator.evaluate(&instance);
        assert!(result.flag().valid);

        // Check annotations exist
        let annotations: Vec<_> = result.iter_annotations().collect();
        assert!(!annotations.is_empty());
    }

    #[test]
    fn test_prefix_multiple_patterns_evaluate() {
        let schema = json!({
            "patternProperties": {
                "^x-": {"type": "string"},
                "^y-": {"type": "number"}
            }
        });
        let validator = crate::validator_for(&schema).unwrap();

        // All valid
        let instance = json!({"x-a": "s", "y-b": 1});
        let result = validator.evaluate(&instance);
        assert!(result.flag().valid);

        // One invalid
        let instance = json!({"x-a": 123});
        let result = validator.evaluate(&instance);
        assert!(!result.flag().valid);
    }
}
