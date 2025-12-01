use crate::{
    compiler,
    error::{no_error, ErrorIterator, ValidationError},
    evaluation::{format_schema_location, Annotations, ErrorDescription, EvaluationNode},
    keywords::CompilationResult,
    node::SchemaNode,
    paths::{LazyLocation, Location, RefTracker},
    properties::HASHMAP_THRESHOLD,
    types::JsonType,
    validator::{EvaluationResult, Validate, ValidationContext},
};
use ahash::AHashMap;
use referencing::Uri;
use serde_json::{Map, Value};
use std::sync::Arc;

pub(crate) struct SmallPropertiesValidator {
    pub(crate) properties: Vec<(String, SchemaNode)>,
    location: Location,
}

pub(crate) struct BigPropertiesValidator {
    pub(crate) properties: AHashMap<String, SchemaNode>,
    location: Location,
}

/// Fused validator for `properties` + `required: [2 items]` (no `additionalProperties: false`).
/// Eliminates separate required validation pass and duplicate `BTreeMap` lookups.
pub(crate) struct SmallPropertiesWithRequired2Validator {
    pub(crate) properties: Vec<(String, SchemaNode)>,
    first: String,
    second: String,
    required_location: Location,
    required_absolute_location: Option<Arc<Uri<String>>>,
    location: Location,
}

impl SmallPropertiesValidator {
    #[inline]
    pub(crate) fn compile<'a>(
        ctx: &compiler::Context,
        map: &'a Map<String, Value>,
    ) -> CompilationResult<'a> {
        let ctx = ctx.new_at_location("properties");
        let mut properties = Vec::with_capacity(map.len());
        for (key, subschema) in map {
            let ctx = ctx.new_at_location(key.as_str());
            properties.push((
                key.clone(),
                compiler::compile(&ctx, ctx.as_resource_ref(subschema))?,
            ));
        }
        Ok(Box::new(SmallPropertiesValidator {
            properties,
            location: ctx.location().clone(),
        }))
    }
}

impl BigPropertiesValidator {
    #[inline]
    pub(crate) fn compile<'a>(
        ctx: &compiler::Context,
        map: &'a Map<String, Value>,
    ) -> CompilationResult<'a> {
        let ctx = ctx.new_at_location("properties");
        let mut properties = AHashMap::with_capacity(map.len());
        for (key, subschema) in map {
            let pctx = ctx.new_at_location(key.as_str());
            properties.insert(
                key.clone(),
                compiler::compile(&pctx, pctx.as_resource_ref(subschema))?,
            );
        }
        Ok(Box::new(BigPropertiesValidator {
            properties,
            location: ctx.location().clone(),
        }))
    }
}

impl SmallPropertiesWithRequired2Validator {
    #[inline]
    pub(crate) fn compile<'a>(
        ctx: &compiler::Context,
        map: &'a Map<String, Value>,
        first: String,
        second: String,
    ) -> CompilationResult<'a> {
        let pctx = ctx.new_at_location("properties");
        let mut properties = Vec::with_capacity(map.len());
        for (key, subschema) in map {
            let kctx = pctx.new_at_location(key.as_str());
            properties.push((
                key.clone(),
                compiler::compile(&kctx, kctx.as_resource_ref(subschema))?,
            ));
        }
        let required_location = ctx.location().join("required");
        let required_absolute_location = ctx.absolute_location(&required_location);
        Ok(Box::new(SmallPropertiesWithRequired2Validator {
            properties,
            first,
            second,
            required_location,
            required_absolute_location,
            location: pctx.location().clone(),
        }))
    }
}

impl Validate for SmallPropertiesValidator {
    fn schema_path(&self) -> &Location {
        &self.location
    }

    fn matches_type(&self, instance: &Value) -> bool {
        matches!(instance, Value::Object(_))
    }

    fn trace(
        &self,
        instance: &Value,
        instance_path: &LazyLocation,
        callback: crate::tracing::TracingCallback<'_>,
        ctx: &mut ValidationContext,
    ) -> bool {
        if let Value::Object(object) = instance {
            let mut is_valid = true;
            let mut at_least_one = false;
            for (name, node) in &self.properties {
                let path = instance_path.push(name);
                let schema_path = node.schema_path();
                if let Some(item) = object.get(name) {
                    at_least_one = true;
                    let schema_is_valid = node.trace(item, &path, callback, ctx);
                    crate::tracing::TracingContext::new(
                        instance_path,
                        schema_path,
                        schema_is_valid,
                    )
                    .call(callback);
                    is_valid &= schema_is_valid;
                } else {
                    crate::tracing::TracingContext::new(instance_path, schema_path, None)
                        .call(callback);
                }
            }
            let rv = if at_least_one { Some(is_valid) } else { None };
            crate::tracing::TracingContext::new(instance_path, self.schema_path(), rv)
                .call(callback);
            is_valid
        } else {
            crate::tracing::TracingContext::new(instance_path, self.schema_path(), None)
                .call(callback);
            true
        }
    }

    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        if let Value::Object(item) = instance {
            for (name, node) in &self.properties {
                if let Some(prop) = item.get(name) {
                    if !node.is_valid(prop, ctx) {
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
            for (name, node) in &self.properties {
                if let Some(item) = item.get(name) {
                    node.validate(item, &location.push(name), tracker, ctx)?;
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::needless_collect)]
    fn iter_errors<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Value::Object(item) = instance {
            let mut errors = Vec::new();
            for (name, node) in &self.properties {
                if let Some(prop) = item.get(name) {
                    let instance_path = location.push(name.as_str());
                    errors.extend(node.iter_errors(prop, &instance_path, tracker, ctx));
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
        if let Value::Object(props) = instance {
            let mut matched_props = Vec::with_capacity(props.len());
            let mut children = Vec::new();
            for (prop_name, node) in &self.properties {
                if let Some(prop) = props.get(prop_name) {
                    let path = location.push(prop_name.as_str());
                    matched_props.push(prop_name.clone());
                    children.push(node.evaluate_instance(prop, &path, tracker, ctx));
                }
            }
            let mut application = EvaluationResult::from_children(children);
            application.annotate(Annotations::new(Value::from(matched_props)));
            application
        } else {
            EvaluationResult::valid_empty()
        }
    }
}

impl Validate for SmallPropertiesWithRequired2Validator {
    fn schema_path(&self) -> &Location {
        &self.location
    }

    fn matches_type(&self, instance: &Value) -> bool {
        matches!(instance, Value::Object(_))
    }

    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        if let Value::Object(item) = instance {
            // Check required first (fast fail)
            if item.len() < 2 || !item.contains_key(&self.first) || !item.contains_key(&self.second)
            {
                return false;
            }
            // Validate properties
            for (name, node) in &self.properties {
                if let Some(prop) = item.get(name) {
                    if !node.is_valid(prop, ctx) {
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
            // Check required first
            if !item.contains_key(&self.first) {
                return Err(ValidationError::required(
                    self.required_location.clone(),
                    crate::paths::capture_evaluation_path(tracker, &self.required_location),
                    location.into(),
                    instance,
                    Value::String(self.first.clone()),
                ));
            }
            if !item.contains_key(&self.second) {
                return Err(ValidationError::required(
                    self.required_location.clone(),
                    crate::paths::capture_evaluation_path(tracker, &self.required_location),
                    location.into(),
                    instance,
                    Value::String(self.second.clone()),
                ));
            }
            // Validate properties
            for (name, node) in &self.properties {
                if let Some(prop) = item.get(name) {
                    node.validate(prop, &location.push(name), tracker, ctx)?;
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::needless_collect)]
    fn iter_errors<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Value::Object(item) = instance {
            let mut errors = Vec::new();
            // Check required
            let eval_path = crate::paths::capture_evaluation_path(tracker, &self.required_location);
            if !item.contains_key(&self.first) {
                errors.push(ValidationError::required(
                    self.required_location.clone(),
                    eval_path.clone(),
                    location.into(),
                    instance,
                    Value::String(self.first.clone()),
                ));
            }
            if !item.contains_key(&self.second) {
                errors.push(ValidationError::required(
                    self.required_location.clone(),
                    eval_path,
                    location.into(),
                    instance,
                    Value::String(self.second.clone()),
                ));
            }
            // Validate properties
            for (name, node) in &self.properties {
                if let Some(prop) = item.get(name) {
                    let instance_path = location.push(name.as_str());
                    errors.extend(node.iter_errors(prop, &instance_path, tracker, ctx));
                }
            }
            if !errors.is_empty() {
                return ErrorIterator::from_iterator(errors.into_iter());
            }
        }
        no_error()
    }

    fn evaluate(
        &self,
        instance: &Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Value::Object(props) = instance {
            let mut matched_props = Vec::with_capacity(props.len());
            let mut children = Vec::new();
            for (prop_name, node) in &self.properties {
                if let Some(prop) = props.get(prop_name) {
                    let path = location.push(prop_name.as_str());
                    matched_props.push(prop_name.clone());
                    children.push(node.evaluate_instance(prop, &path, tracker, ctx));
                }
            }
            // `required` is fused into this validator, so its failures are emitted as a child node at
            // the `required` keyword location to keep the correct `schemaLocation` in structured output.
            let mut required_errors = Vec::new();
            let eval_path = crate::paths::capture_evaluation_path(tracker, &self.required_location);
            if !props.contains_key(&self.first) {
                required_errors.push(ErrorDescription::from_validation_error(
                    &ValidationError::required(
                        self.required_location.clone(),
                        eval_path.clone(),
                        location.into(),
                        instance,
                        Value::String(self.first.clone()),
                    ),
                ));
            }
            if !props.contains_key(&self.second) {
                required_errors.push(ErrorDescription::from_validation_error(
                    &ValidationError::required(
                        self.required_location.clone(),
                        eval_path,
                        location.into(),
                        instance,
                        Value::String(self.second.clone()),
                    ),
                ));
            }
            if !required_errors.is_empty() {
                children.push(EvaluationNode::invalid(
                    crate::paths::evaluation_path(tracker, &self.required_location),
                    self.required_absolute_location.clone(),
                    format_schema_location(
                        &self.required_location,
                        self.required_absolute_location.as_ref(),
                    ),
                    location.into(),
                    None,
                    required_errors,
                    Vec::new(),
                ));
            }
            let mut application = EvaluationResult::from_children(children);
            application.annotate(Annotations::new(Value::from(matched_props)));
            application
        } else {
            EvaluationResult::valid_empty()
        }
    }
}

impl Validate for BigPropertiesValidator {
    fn schema_path(&self) -> &Location {
        &self.location
    }

    fn matches_type(&self, instance: &Value) -> bool {
        matches!(instance, Value::Object(_))
    }

    fn trace(
        &self,
        instance: &Value,
        instance_path: &LazyLocation,
        callback: crate::tracing::TracingCallback<'_>,
        ctx: &mut ValidationContext,
    ) -> bool {
        if let Value::Object(object) = instance {
            let mut is_valid = true;
            let mut at_least_one = false;
            for (name, node) in &self.properties {
                let path = instance_path.push(name);
                let schema_path = node.schema_path();
                if let Some(item) = object.get(name) {
                    at_least_one = true;
                    let schema_is_valid = node.trace(item, &path, callback, ctx);
                    crate::tracing::TracingContext::new(
                        instance_path,
                        schema_path,
                        schema_is_valid,
                    )
                    .call(callback);
                    is_valid &= schema_is_valid;
                } else {
                    crate::tracing::TracingContext::new(instance_path, schema_path, None)
                        .call(callback);
                }
            }
            let rv = if at_least_one { Some(is_valid) } else { None };
            crate::tracing::TracingContext::new(instance_path, self.schema_path(), rv)
                .call(callback);
            is_valid
        } else {
            crate::tracing::TracingContext::new(instance_path, self.schema_path(), None)
                .call(callback);
            true
        }
    }

    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        if let Value::Object(item) = instance {
            // Iterate over instance properties and look up in schema's HashMap
            for (name, prop) in item {
                if let Some(node) = self.properties.get(name) {
                    if !node.is_valid(prop, ctx) {
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
            for (name, value) in item {
                if let Some(node) = self.properties.get(name) {
                    node.validate(value, &location.push(name), tracker, ctx)?;
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::needless_collect)]
    fn iter_errors<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Value::Object(item) = instance {
            let mut errors = Vec::new();
            for (name, prop) in item {
                if let Some(node) = self.properties.get(name) {
                    let instance_path = location.push(name.as_str());
                    errors.extend(node.iter_errors(prop, &instance_path, tracker, ctx));
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
        if let Value::Object(props) = instance {
            let mut matched_props = Vec::with_capacity(props.len());
            let mut children = Vec::new();
            for (prop_name, prop) in props {
                if let Some(node) = self.properties.get(prop_name) {
                    let path = location.push(prop_name.as_str());
                    matched_props.push(prop_name.clone());
                    children.push(node.evaluate_instance(prop, &path, tracker, ctx));
                }
            }
            let mut application = EvaluationResult::from_children(children);
            application.annotate(Annotations::new(Value::from(matched_props)));
            application
        } else {
            EvaluationResult::valid_empty()
        }
    }
}

/// Check if we can use fused properties+required validator.
/// Conditions: properties < threshold, required: [2 strings], no patternProperties.
fn extract_required2(parent: &Map<String, Value>) -> Option<(String, String)> {
    // No patternProperties (uses separate validator paths)
    if parent.contains_key("patternProperties") {
        return None;
    }
    if let Some(Value::Array(items)) = parent.get("required") {
        if items.len() == 2 {
            if let (Some(Value::String(first)), Some(Value::String(second))) =
                (items.first(), items.get(1))
            {
                return Some((first.clone(), second.clone()));
            }
        }
    }
    None
}

#[inline]
pub(crate) fn compile<'a>(
    ctx: &compiler::Context,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>> {
    match parent.get("additionalProperties") {
        // This type of `additionalProperties` validator handles `properties` logic
        Some(Value::Bool(false) | Value::Object(_)) => None,
        _ => {
            if let Value::Object(map) = schema {
                if map.len() < HASHMAP_THRESHOLD {
                    // Try fused validator for properties + required: [2 items]
                    if let Some((first, second)) = extract_required2(parent) {
                        Some(SmallPropertiesWithRequired2Validator::compile(
                            ctx, map, first, second,
                        ))
                    } else {
                        Some(SmallPropertiesValidator::compile(ctx, map))
                    }
                } else {
                    Some(BigPropertiesValidator::compile(ctx, map))
                }
            } else {
                let location = ctx.location().join("properties");
                Some(Err(ValidationError::single_type_error(
                    location.clone(),
                    location,
                    Location::new(),
                    schema,
                    JsonType::Object,
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use serde_json::{json, Value};
    use test_case::test_case;

    #[test]
    fn location() {
        tests_util::assert_schema_location(
            &json!({"properties": {"foo": {"properties": {"bar": {"required": ["spam"]}}}}}),
            &json!({"foo": {"bar": {}}}),
            "/properties/foo/properties/bar/required",
        );
    }

    // SmallPropertiesWithRequired2Validator tests
    fn fused_schema() -> Value {
        // No additionalProperties: false, so uses SmallPropertiesWithRequired2Validator
        json!({
            "properties": {
                "a": {"type": "integer"},
                "b": {"type": "string"}
            },
            "required": ["a", "b"]
        })
    }

    #[test_case(&json!({"a": 1, "b": "x"}), true)]
    #[test_case(&json!({"a": 1, "b": "x", "c": 3}), true)]
    #[test_case(&json!({"a": 1}), false)] // missing b
    #[test_case(&json!({"b": "x"}), false)] // missing a
    #[test_case(&json!({}), false)]
    #[test_case(&json!("string"), true)] // non-object passes
    fn fused_properties_required2_is_valid(instance: &Value, expected: bool) {
        let validator = crate::validator_for(&fused_schema()).unwrap();
        assert_eq!(validator.is_valid(instance), expected);
    }

    #[test]
    fn fused_properties_required2_validate_missing_first() {
        let validator = crate::validator_for(&fused_schema()).unwrap();
        let instance = json!({"b": "x"});
        let result = validator.validate(&instance);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("required"));
    }

    #[test]
    fn fused_properties_required2_validate_missing_second() {
        let validator = crate::validator_for(&fused_schema()).unwrap();
        let instance = json!({"a": 1});
        let result = validator.validate(&instance);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("required"));
    }

    #[test]
    fn fused_properties_required2_iter_errors_missing_both() {
        let validator = crate::validator_for(&fused_schema()).unwrap();
        let instance = json!({});
        let errors: Vec<_> = validator.iter_errors(&instance).collect();
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn fused_properties_required2_iter_errors_missing_first() {
        let validator = crate::validator_for(&fused_schema()).unwrap();
        let instance = json!({"b": "x"});
        let errors: Vec<_> = validator.iter_errors(&instance).collect();
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn fused_properties_required2_iter_errors_missing_second() {
        let validator = crate::validator_for(&fused_schema()).unwrap();
        let instance = json!({"a": 1});
        let errors: Vec<_> = validator.iter_errors(&instance).collect();
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn fused_properties_required2_iter_errors_valid() {
        let validator = crate::validator_for(&fused_schema()).unwrap();
        let instance = json!({"a": 1, "b": "x"});
        let errors: Vec<_> = validator.iter_errors(&instance).collect();
        assert!(errors.is_empty());
    }

    #[test_case(&json!({"a": 1, "b": "x"}), &[])] // valid
    #[test_case(&json!({"a": 1}), &[("/required", "", "required", "\"b\" is a required property")])] // missing b
    #[test_case(&json!({"b": "x"}), &[("/required", "", "required", "\"a\" is a required property")])] // missing a
    #[test_case(&json!({}), &[
        ("/required", "", "required", "\"a\" is a required property"),
        ("/required", "", "required", "\"b\" is a required property"),
    ])] // missing both
    fn fused_properties_required2_evaluate(
        instance: &Value,
        expected: &[(&str, &str, &str, &str)],
    ) {
        let validator = crate::validator_for(&fused_schema()).unwrap();
        let eval = validator.evaluate(instance);
        assert_eq!(eval.flag().valid, expected.is_empty());
        let errors: Vec<_> = eval
            .iter_errors()
            .map(|e| {
                (
                    e.schema_location,
                    e.instance_location.as_str(),
                    e.error.keyword(),
                    e.error.message(),
                )
            })
            .collect();
        assert_eq!(errors.as_slice(), expected);
    }
}
