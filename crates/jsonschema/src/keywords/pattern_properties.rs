use std::{borrow::Cow, sync::Arc};

use crate::{
    compiler,
    error::{no_error, ErrorIterator, ValidationError},
    evaluation::Annotations,
    keywords::CompilationResult,
    node::SchemaNode,
    options::PatternEngineOptions,
    paths::{LazyEvaluationPath, LazyLocation, Location, RefTracker},
    regex::{analyze_pattern, LiteralMatcher, PatternOptimization, RegexEngine},
    types::JsonType,
    validator::{EvaluationResult, Validate, ValidationContext},
    Json, JsonNode, JsonObjectAccess, SerdeJson,
};
use serde_json::{Map, Value};

/// Validator for multiple patterns using compiled regex.
pub(crate) struct PatternPropertiesValidator<R, F: Json = SerdeJson> {
    patterns: Vec<(Arc<R>, SchemaNode<F>)>,
}

impl<F: Json, R: RegexEngine> Validate<F> for PatternPropertiesValidator<R, F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        if let Some(object) = instance.as_object() {
            for (re, node) in &self.patterns {
                for (key, value) in object.members() {
                    if re.is_match(key.as_ref()).unwrap_or(false) && !node.is_valid(&value, ctx) {
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
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Some(object) = instance.as_object() {
            for (key, value) in object.members() {
                for (re, node) in &self.patterns {
                    if re.is_match(key.as_ref()).unwrap_or(false) {
                        node.validate(&value, &location.push(key.as_ref()), tracker, ctx)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn iter_errors<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Some(object) = instance.as_object() {
            let mut errors = Vec::new();
            for (re, node) in &self.patterns {
                for (key, value) in object.members() {
                    if re.is_match(key.as_ref()).unwrap_or(false) {
                        errors.extend(node.iter_errors(
                            &value,
                            &location.push(key.as_ref()),
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
        instance: &F::Node<'_>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Some(object) = instance.as_object() {
            let mut matched_propnames = Vec::with_capacity(object.len());
            let mut children = Vec::new();
            for (pattern, node) in &self.patterns {
                for (key, value) in object.members() {
                    if pattern.is_match(key.as_ref()).unwrap_or(false) {
                        matched_propnames.push(key.as_ref().to_owned());
                        children.push(node.evaluate_instance(
                            &value,
                            &location.push(key.as_ref()),
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

pub(crate) struct SingleValuePatternPropertiesValidator<R, F: Json = SerdeJson> {
    regex: Arc<R>,
    node: SchemaNode<F>,
}

impl<F: Json, R: RegexEngine> Validate<F> for SingleValuePatternPropertiesValidator<R, F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        if let Some(object) = instance.as_object() {
            for (key, value) in object.members() {
                if self.regex.is_match(key.as_ref()).unwrap_or(false)
                    && !self.node.is_valid(&value, ctx)
                {
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
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Some(object) = instance.as_object() {
            for (key, value) in object.members() {
                if self.regex.is_match(key.as_ref()).unwrap_or(false) {
                    self.node
                        .validate(&value, &location.push(key.as_ref()), tracker, ctx)?;
                }
            }
        }
        Ok(())
    }

    fn iter_errors<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Some(object) = instance.as_object() {
            let mut errors = Vec::new();
            for (key, value) in object.members() {
                if self.regex.is_match(key.as_ref()).unwrap_or(false) {
                    errors.extend(self.node.iter_errors(
                        &value,
                        &location.push(key.as_ref()),
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
        instance: &F::Node<'_>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Some(object) = instance.as_object() {
            let mut matched_propnames = Vec::with_capacity(object.len());
            let mut children = Vec::new();
            for (key, value) in object.members() {
                if self.regex.is_match(key.as_ref()).unwrap_or(false) {
                    matched_propnames.push(key.as_ref().to_owned());
                    children.push(self.node.evaluate_instance(
                        &value,
                        &location.push(key.as_ref()),
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
pub(crate) fn compile<'a, F: Json>(
    ctx: &compiler::Context<F>,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a, F>> {
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
            Cow::Borrowed(schema),
            JsonType::Object,
        )));
    };
    let ctx = ctx.new_at_location("patternProperties");

    // Try to compile all patterns as literal matches first (optimized path)
    if let Some(validator) = try_compile_as_literals(&ctx, map) {
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
                        as Box<dyn Validate<F>>
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
                        as Box<dyn Validate<F>>
                })
            })
        }
    };
    Some(result)
}

/// Try to compile all patterns as literal matches (prefix or exact).
/// Returns `Some` if ALL patterns are optimizable, `None` if any requires a full regex.
fn try_compile_as_literals<'a, F: Json>(
    ctx: &compiler::Context<F>,
    map: &'a Map<String, Value>,
) -> Option<CompilationResult<'a, F>> {
    let mut entries = Vec::with_capacity(map.len());
    for (pattern, subschema) in map {
        let pctx = ctx.new_at_location(pattern.as_str());
        let matcher = match analyze_pattern(pattern)? {
            PatternOptimization::Prefix(literal) => LiteralMatcher::Prefix { literal },
            PatternOptimization::Exact(exact) => LiteralMatcher::Exact { exact },
            PatternOptimization::Alternation(alternatives) => {
                LiteralMatcher::Alternation { alternatives }
            }
            PatternOptimization::NoWhitespace => LiteralMatcher::NoWhitespace,
        };
        let node = match compiler::compile(&pctx, pctx.as_resource_ref(subschema)) {
            Ok(node) => node,
            Err(e) => return Some(Err(e)),
        };
        entries.push((Arc::new(matcher), node));
    }
    Some(Ok(build_validator_from_entries(entries, |regex, node| {
        Box::new(SingleValuePatternPropertiesValidator { regex, node }) as Box<dyn Validate<F>>
    })))
}

fn invalid_regex<'a, F: Json>(
    ctx: &compiler::Context<F>,
    schema: &'a Value,
) -> ValidationError<'a> {
    ValidationError::format(
        ctx.location().clone(),
        LazyEvaluationPath::SameAsSchemaPath,
        Location::new(),
        Cow::Borrowed(schema),
        "regex",
    )
}

type CompiledPatterns<R, F> = Vec<(Arc<R>, SchemaNode<F>)>;

/// Compile every `(pattern, subschema)` pair into `(regex, node)` tuples.
fn compile_pattern_entries<'a, R, C, F: Json>(
    ctx: &compiler::Context<F>,
    map: &'a Map<String, Value>,
    mut compile_regex: C,
) -> Result<CompiledPatterns<R, F>, ValidationError<'a>>
where
    C: FnMut(&compiler::Context<F>, &str, &'a Value) -> Result<Arc<R>, ValidationError<'a>>,
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
fn build_validator_from_entries<R, F: Json>(
    mut entries: Vec<(Arc<R>, SchemaNode<F>)>,
    single_factory: impl FnOnce(Arc<R>, SchemaNode<F>) -> Box<dyn Validate<F>>,
) -> Box<dyn Validate<F>>
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
    use crate::{
        regex::{analyze_pattern, PatternOptimization},
        tests_util,
    };
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

    #[test]
    fn test_analyze_pattern() {
        use PatternOptimization::{Exact, Prefix};
        assert_eq!(analyze_pattern("^foo"), Some(Prefix("foo".into())));
        assert_eq!(analyze_pattern("^x-"), Some(Prefix("x-".into())));
        assert_eq!(analyze_pattern("^eo_band"), Some(Prefix("eo_band".into())));
        assert_eq!(analyze_pattern("^path/to"), Some(Prefix("path/to".into())));
        assert_eq!(analyze_pattern("^ABC123"), Some(Prefix("ABC123".into())));
        assert_eq!(analyze_pattern("^\\/"), Some(Prefix("/".into())));
        assert_eq!(analyze_pattern("^foo$"), Some(Exact("foo".into())));
        assert_eq!(analyze_pattern("^\\$ref$"), Some(Exact("$ref".into())));
        assert_eq!(analyze_pattern("foo"), None);
        assert_eq!(analyze_pattern("^foo.*"), None);
        assert_eq!(analyze_pattern("^foo+"), None);
        assert_eq!(analyze_pattern("^foo?"), None);
        assert_eq!(analyze_pattern("^[a-z]"), None);
        assert_eq!(analyze_pattern("^foo|bar"), None);
        assert_eq!(analyze_pattern("^foo(bar)"), None);
        assert_eq!(analyze_pattern("^foo\\d"), None);
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
                "^y-": {"type": "number"},
                "^z$": {"type": "boolean"},
            }
        });
        let validator = crate::validator_for(&schema).unwrap();

        // All valid
        let instance = json!({"x-a": "s", "y-b": 1, "z": true});
        let result = validator.evaluate(&instance);
        assert!(result.flag().valid);

        // One invalid
        let instance = json!({"x-a": 123});
        let result = validator.evaluate(&instance);
        assert!(!result.flag().valid);
    }
}
