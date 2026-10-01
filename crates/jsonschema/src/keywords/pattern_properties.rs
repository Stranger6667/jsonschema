use crate::LazyInstance;
use std::{borrow::Cow, sync::Arc};

use crate::{
    compiler,
    error::ValidationError,
    evaluation::{Annotations, ChildList},
    keywords::CompilationResult,
    node::SchemaNode,
    options::PatternEngineOptions,
    paths::{LazyEvaluationPath, LazyLocation, Location, RefTracker},
    regex::{analyze_pattern, LiteralMatcher, PatternOptimization, RegexEngine},
    types::JsonType,
    validator::{EvaluationResult, Validate, ValidationContext},
    Json, Node, Object, SerdeJson,
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

    fn collect_errors<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
        errors: &mut Vec<ValidationError<'i>>,
    ) {
        let Some(object) = instance.as_object() else {
            return;
        };
        for (re, node) in &self.patterns {
            for (key, value) in object.members() {
                if re.is_match(key.as_ref()).unwrap_or(false) {
                    node.collect_errors(&value, &location.push(key.as_ref()), tracker, ctx, errors);
                }
            }
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
            let mut children = ChildList::default();
            for (pattern, node) in &self.patterns {
                for (key, value) in object.members() {
                    if pattern.is_match(key.as_ref()).unwrap_or(false) {
                        matched_propnames.push(key.as_ref().to_owned());
                        let child = node.evaluate_instance_below(
                            &value,
                            &location.push(key.as_ref()),
                            tracker,
                            ctx,
                        );
                        children.push(&mut ctx.arena, child);
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

    fn collect_errors<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
        errors: &mut Vec<ValidationError<'i>>,
    ) {
        let Some(object) = instance.as_object() else {
            return;
        };
        for (key, value) in object.members() {
            if self.regex.is_match(key.as_ref()).unwrap_or(false) {
                self.node.collect_errors(
                    &value,
                    &location.push(key.as_ref()),
                    tracker,
                    ctx,
                    errors,
                );
            }
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
            let mut children = ChildList::default();
            for (key, value) in object.members() {
                if self.regex.is_match(key.as_ref()).unwrap_or(false) {
                    matched_propnames.push(key.as_ref().to_owned());
                    let child = self.node.evaluate_instance_below(
                        &value,
                        &location.push(key.as_ref()),
                        tracker,
                        ctx,
                    );
                    children.push(&mut ctx.arena, child);
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
    ) && !ctx.is_keyword_overridden("additionalProperties")
    {
        // This type of `additionalProperties` validator handles `patternProperties` logic
        return None;
    }

    let Value::Object(map) = schema else {
        let location = ctx.location().join("patternProperties");
        return Some(Err(ValidationError::single_type_error(
            location.clone(),
            location,
            Location::new(),
            LazyInstance::Ready(Cow::Borrowed(schema)),
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
            compile_pattern_entries(&ctx, map, |pctx, pattern| {
                pctx.get_or_compile_regex(pattern)
                    .map_err(|()| invalid_regex(pctx, pattern))
            })
            .map(|patterns| {
                build_validator_from_entries(patterns, |regex, node| {
                    Box::new(SingleValuePatternPropertiesValidator { regex, node })
                        as Box<dyn Validate<F>>
                })
            })
        }
        PatternEngineOptions::Regex { .. } => {
            compile_pattern_entries(&ctx, map, |pctx, pattern| {
                pctx.get_or_compile_standard_regex(pattern)
                    .map_err(|()| invalid_regex(pctx, pattern))
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

/// Build error for a `patternProperties` key that is not a valid regex; `ctx` points at the key.
pub(crate) fn invalid_regex<F: Json>(
    ctx: &compiler::Context<F>,
    pattern: &str,
) -> ValidationError<'static> {
    ValidationError::format(
        ctx.location().clone(),
        LazyEvaluationPath::SameAsSchemaPath,
        Location::new(),
        LazyInstance::Ready(Cow::Owned(Value::String(pattern.to_owned()))),
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
    C: FnMut(&compiler::Context<F>, &str) -> Result<Arc<R>, ValidationError<'a>>,
{
    let mut patterns = Vec::with_capacity(map.len());
    for (pattern, subschema) in map {
        let pctx = ctx.new_at_location(pattern.as_str());
        let regex = compile_regex(&pctx, pattern)?;
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
        properties::HASHMAP_THRESHOLD,
        regex::{analyze_pattern, PatternOptimization},
        tests_util, PatternOptions,
    };
    use serde_json::{json, Map, Value};
    use test_case::test_case;

    #[test_case(&json!({"patternProperties": {"^f": {"type": "string"}}}), &json!({"f": 42}), "/patternProperties/^f/type")]
    #[test_case(&json!({"patternProperties": {"^f": {"type": "string"}, "^x": {"type": "string"}}}), &json!({"f": 42}), "/patternProperties/^f/type")]
    fn location(schema: &Value, instance: &Value, expected: &str) {
        tests_util::assert_schema_location(schema, instance, expected);
    }

    #[derive(Clone, Copy)]
    enum Engine {
        FancyRegex,
        Regex,
    }

    fn with_many_properties(mut schema: Value) -> Value {
        let properties = (0..HASHMAP_THRESHOLD)
            .map(|idx| (format!("p{idx}"), json!({})))
            .collect::<Map<String, Value>>();
        schema["properties"] = Value::Object(properties);
        schema
    }

    const LOOKBEHIND: &str = "(?<=a)b";
    const UNBALANCED: &str = "a(";

    #[test_case(Engine::Regex, &json!({"patternProperties": {LOOKBEHIND: {"type": "string"}}}), LOOKBEHIND, "/patternProperties/(?<=a)b"; "regex plain")]
    #[test_case(Engine::Regex, &json!({"patternProperties": {"^x": {}, LOOKBEHIND: {"type": "string"}}}), LOOKBEHIND, "/patternProperties/(?<=a)b"; "regex plain many")]
    #[test_case(Engine::Regex, &json!({"additionalProperties": true, "patternProperties": {LOOKBEHIND: {"type": "string"}}}), LOOKBEHIND, "/patternProperties/(?<=a)b"; "regex additional true")]
    #[test_case(Engine::Regex, &json!({"additionalProperties": false, "patternProperties": {LOOKBEHIND: {"type": "string"}}}), LOOKBEHIND, "/patternProperties/(?<=a)b"; "regex additional false")]
    #[test_case(Engine::Regex, &json!({"additionalProperties": {"type": "integer"}, "patternProperties": {LOOKBEHIND: {"type": "string"}}}), LOOKBEHIND, "/patternProperties/(?<=a)b"; "regex additional schema")]
    #[test_case(Engine::Regex, &json!({"properties": {"foo": {}}, "additionalProperties": false, "patternProperties": {LOOKBEHIND: {"type": "string"}}}), LOOKBEHIND, "/patternProperties/(?<=a)b"; "regex properties additional false")]
    #[test_case(Engine::Regex, &json!({"properties": {"foo": {}}, "additionalProperties": {"type": "integer"}, "patternProperties": {LOOKBEHIND: {"type": "string"}}}), LOOKBEHIND, "/patternProperties/(?<=a)b"; "regex properties additional schema")]
    #[test_case(Engine::Regex, &with_many_properties(json!({"additionalProperties": false, "patternProperties": {LOOKBEHIND: {"type": "string"}}})), LOOKBEHIND, "/patternProperties/(?<=a)b"; "regex many properties additional false")]
    #[test_case(Engine::Regex, &with_many_properties(json!({"additionalProperties": {"type": "integer"}, "patternProperties": {LOOKBEHIND: {"type": "string"}}})), LOOKBEHIND, "/patternProperties/(?<=a)b"; "regex many properties additional schema")]
    #[test_case(Engine::Regex, &json!({"unevaluatedProperties": false, "patternProperties": {LOOKBEHIND: {"type": "string"}}}), LOOKBEHIND, "/patternProperties/(?<=a)b"; "regex unevaluated")]
    #[test_case(Engine::Regex, &json!({"patternProperties": {"a/b~(?<=c)": {}}}), "a/b~(?<=c)", "/patternProperties/a~1b~0(?<=c)"; "regex escaped key")]
    #[test_case(Engine::Regex, &json!({"additionalProperties": false, "patternProperties": {"a/b~(?<=c)": {}}}), "a/b~(?<=c)", "/patternProperties/a~1b~0(?<=c)"; "regex escaped key additional false")]
    #[test_case(Engine::Regex, &json!({"propertyNames": {"pattern": LOOKBEHIND}}), LOOKBEHIND, "/propertyNames/pattern"; "regex property names")]
    #[test_case(Engine::Regex, &json!({"pattern": LOOKBEHIND}), LOOKBEHIND, "/pattern"; "regex pattern")]
    #[test_case(Engine::FancyRegex, &json!({"patternProperties": {UNBALANCED: {"type": "string"}}}), UNBALANCED, "/patternProperties/a("; "fancy plain")]
    #[test_case(Engine::FancyRegex, &json!({"patternProperties": {"^x": {}, UNBALANCED: {"type": "string"}}}), UNBALANCED, "/patternProperties/a("; "fancy plain many")]
    #[test_case(Engine::FancyRegex, &json!({"additionalProperties": true, "patternProperties": {UNBALANCED: {"type": "string"}}}), UNBALANCED, "/patternProperties/a("; "fancy additional true")]
    #[test_case(Engine::FancyRegex, &json!({"additionalProperties": false, "patternProperties": {UNBALANCED: {"type": "string"}}}), UNBALANCED, "/patternProperties/a("; "fancy additional false")]
    #[test_case(Engine::FancyRegex, &json!({"additionalProperties": {"type": "integer"}, "patternProperties": {UNBALANCED: {"type": "string"}}}), UNBALANCED, "/patternProperties/a("; "fancy additional schema")]
    #[test_case(Engine::FancyRegex, &json!({"properties": {"foo": {}}, "additionalProperties": false, "patternProperties": {UNBALANCED: {"type": "string"}}}), UNBALANCED, "/patternProperties/a("; "fancy properties additional false")]
    #[test_case(Engine::FancyRegex, &json!({"properties": {"foo": {}}, "additionalProperties": {"type": "integer"}, "patternProperties": {UNBALANCED: {"type": "string"}}}), UNBALANCED, "/patternProperties/a("; "fancy properties additional schema")]
    #[test_case(Engine::FancyRegex, &with_many_properties(json!({"additionalProperties": false, "patternProperties": {UNBALANCED: {"type": "string"}}})), UNBALANCED, "/patternProperties/a("; "fancy many properties additional false")]
    #[test_case(Engine::FancyRegex, &with_many_properties(json!({"additionalProperties": {"type": "integer"}, "patternProperties": {UNBALANCED: {"type": "string"}}})), UNBALANCED, "/patternProperties/a("; "fancy many properties additional schema")]
    #[test_case(Engine::FancyRegex, &json!({"unevaluatedProperties": false, "patternProperties": {UNBALANCED: {"type": "string"}}}), UNBALANCED, "/patternProperties/a("; "fancy unevaluated")]
    #[test_case(Engine::FancyRegex, &json!({"patternProperties": {"a/b~(": {}}}), "a/b~(", "/patternProperties/a~1b~0("; "fancy escaped key")]
    #[test_case(Engine::FancyRegex, &json!({"additionalProperties": false, "patternProperties": {"a/b~(": {}}}), "a/b~(", "/patternProperties/a~1b~0("; "fancy escaped key additional false")]
    #[test_case(Engine::FancyRegex, &json!({"propertyNames": {"pattern": UNBALANCED}}), UNBALANCED, "/propertyNames/pattern"; "fancy property names")]
    #[test_case(Engine::FancyRegex, &json!({"pattern": UNBALANCED}), UNBALANCED, "/pattern"; "fancy pattern")]
    fn invalid_regex(engine: Engine, schema: &Value, pattern: &str, schema_path: &str) {
        let options = crate::options();
        let error = match engine {
            Engine::FancyRegex => options
                .with_pattern_options(PatternOptions::fancy_regex())
                .build(schema),
            Engine::Regex => options
                .with_pattern_options(PatternOptions::regex())
                .build(schema),
        }
        .expect_err("Should fail to compile");
        assert_eq!(
            (
                error.to_string(),
                error.instance().as_ref(),
                error.schema_path().as_str(),
                error.evaluation_path().as_str(),
            ),
            (
                format!("{} is not a \"regex\"", json!(pattern)),
                &json!(pattern),
                schema_path,
                schema_path,
            )
        );
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

    #[test]
    fn malformed() {
        tests_util::assert_compile_error(
            &json!({"patternProperties": 5}),
            "5 is not of type \"object\"",
            "/patternProperties",
        );
    }
}
