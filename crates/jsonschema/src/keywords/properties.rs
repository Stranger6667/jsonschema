use std::borrow::Cow;

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
    Json, Node, Object, SerdeJson,
};
use ahash::AHashMap;
use referencing::Uri;
use serde_json::{Map, Value};
use std::sync::Arc;

pub(crate) struct SmallPropertiesValidator<F: Json = SerdeJson> {
    pub(crate) properties: Vec<(String, F::PreparedKey, SchemaNode<F>)>,
}

pub(crate) struct BigPropertiesValidator<F: Json = SerdeJson> {
    pub(crate) properties: AHashMap<String, SchemaNode<F>>,
}

/// Fused validator for `properties` + `required: [2 items]` (no `additionalProperties: false`).
/// Eliminates separate required validation pass and duplicate `BTreeMap` lookups.
pub(crate) struct SmallPropertiesWithRequired2Validator<F: Json = SerdeJson> {
    pub(crate) properties: Vec<(String, F::PreparedKey, SchemaNode<F>)>,
    first: String,
    first_key: F::PreparedKey,
    second: String,
    second_key: F::PreparedKey,
    required_location: Location,
    required_absolute_location: Option<Arc<Uri<String>>>,
}

impl SmallPropertiesValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        map: &'a Map<String, Value>,
    ) -> CompilationResult<'a, F> {
        let ctx = ctx.new_at_location("properties");
        let mut properties = Vec::with_capacity(map.len());
        for (key, subschema) in map {
            let ctx = ctx.new_at_location(key.as_str());
            properties.push((
                key.clone(),
                F::prepare_key(key),
                compiler::compile(&ctx, ctx.as_resource_ref(subschema))?,
            ));
        }
        Ok(Box::new(SmallPropertiesValidator { properties }))
    }
}

impl BigPropertiesValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        map: &'a Map<String, Value>,
    ) -> CompilationResult<'a, F> {
        let ctx = ctx.new_at_location("properties");
        let mut properties = AHashMap::with_capacity(map.len());
        for (key, subschema) in map {
            let pctx = ctx.new_at_location(key.as_str());
            properties.insert(
                key.clone(),
                compiler::compile(&pctx, pctx.as_resource_ref(subschema))?,
            );
        }
        Ok(Box::new(BigPropertiesValidator { properties }))
    }
}

impl SmallPropertiesWithRequired2Validator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        map: &'a Map<String, Value>,
        first: String,
        second: String,
    ) -> CompilationResult<'a, F> {
        let pctx = ctx.new_at_location("properties");
        let mut properties = Vec::with_capacity(map.len());
        for (key, subschema) in map {
            let kctx = pctx.new_at_location(key.as_str());
            properties.push((
                key.clone(),
                F::prepare_key(key),
                compiler::compile(&kctx, kctx.as_resource_ref(subschema))?,
            ));
        }
        let required_location = ctx.location().join("required");
        let required_absolute_location = ctx.absolute_location(&required_location);
        Ok(Box::new(SmallPropertiesWithRequired2Validator {
            properties,
            first_key: F::prepare_key(&first),
            second_key: F::prepare_key(&second),
            first,
            second,
            required_location,
            required_absolute_location,
        }))
    }
}

impl<F: Json> Validate<F> for SmallPropertiesValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        let Some(object) = instance.as_object() else {
            return true;
        };
        // Walk the smaller side: short instance in one pass, wide instance via targeted lookups.
        if object.len() <= self.properties.len() {
            for (name, value) in object.members() {
                let name = name.as_ref();
                for (prop_name, _, node) in &self.properties {
                    if prop_name == name {
                        if !node.is_valid(&value, ctx) {
                            return false;
                        }
                        break;
                    }
                }
            }
        } else {
            for (_, key, node) in &self.properties {
                if let Some(prop) = object.get(key) {
                    if !node.is_valid(&prop, ctx) {
                        return false;
                    }
                }
            }
        }
        true
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        let Some(object) = instance.as_object() else {
            return Ok(());
        };
        if object.len() <= self.properties.len() {
            for (name, value) in object.members() {
                let name = name.as_ref();
                for (prop_name, _, node) in &self.properties {
                    if prop_name == name {
                        node.validate(&value, &location.push(name), tracker, ctx)?;
                        break;
                    }
                }
            }
        } else {
            for (name, key, node) in &self.properties {
                if let Some(prop) = object.get(key) {
                    node.validate(&prop, &location.push(name), tracker, ctx)?;
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::needless_collect)]
    fn iter_errors<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        let Some(object) = instance.as_object() else {
            return no_error();
        };
        let mut errors = Vec::new();
        if object.len() <= self.properties.len() {
            for (name, value) in object.members() {
                let name = name.as_ref();
                for (prop_name, _, node) in &self.properties {
                    if prop_name == name {
                        let instance_path = location.push(name);
                        errors.extend(node.iter_errors(&value, &instance_path, tracker, ctx));
                        break;
                    }
                }
            }
        } else {
            for (name, key, node) in &self.properties {
                if let Some(prop) = object.get(key) {
                    let instance_path = location.push(name.as_str());
                    errors.extend(node.iter_errors(&prop, &instance_path, tracker, ctx));
                }
            }
        }
        ErrorIterator::from_iterator(errors.into_iter())
    }

    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        let Some(object) = instance.as_object() else {
            return EvaluationResult::valid_empty();
        };
        let mut matched_props = Vec::with_capacity(object.len());
        let mut children = Vec::new();
        if object.len() <= self.properties.len() {
            for (name, value) in object.members() {
                let name = name.as_ref();
                for (prop_name, _, node) in &self.properties {
                    if prop_name == name {
                        let path = location.push(name);
                        matched_props.push(prop_name.clone());
                        children.push(node.evaluate_instance(&value, &path, tracker, ctx));
                        break;
                    }
                }
            }
        } else {
            for (prop_name, key, node) in &self.properties {
                if let Some(prop) = object.get(key) {
                    let path = location.push(prop_name.as_str());
                    matched_props.push(prop_name.clone());
                    children.push(node.evaluate_instance(&prop, &path, tracker, ctx));
                }
            }
        }
        let mut application = EvaluationResult::from_children(children);
        application.annotate(Annotations::new(Value::from(matched_props)));
        application
    }
}

impl<F: Json> Validate<F> for SmallPropertiesWithRequired2Validator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        let Some(object) = instance.as_object() else {
            return true;
        };
        if object.len() < 2 {
            return false;
        }
        if object.len() <= self.properties.len() {
            // One pass validates matching properties and confirms both required keys.
            let mut seen_first = false;
            let mut seen_second = false;
            for (name, value) in object.members() {
                let name = name.as_ref();
                if name == self.first.as_str() {
                    seen_first = true;
                } else if name == self.second.as_str() {
                    seen_second = true;
                }
                for (prop_name, _, node) in &self.properties {
                    if prop_name.as_str() == name {
                        if !node.is_valid(&value, ctx) {
                            return false;
                        }
                        break;
                    }
                }
            }
            seen_first && seen_second
        } else {
            if object.get(&self.first_key).is_none() || object.get(&self.second_key).is_none() {
                return false;
            }
            for (_, key, node) in &self.properties {
                if let Some(prop) = object.get(key) {
                    if !node.is_valid(&prop, ctx) {
                        return false;
                    }
                }
            }
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
            // Check required first
            if object.get(&self.first_key).is_none() {
                return Err(ValidationError::required(
                    self.required_location.clone(),
                    crate::paths::capture_evaluation_path(tracker, &self.required_location),
                    location.into(),
                    instance.to_value(),
                    Value::String(self.first.clone()),
                ));
            }
            if object.get(&self.second_key).is_none() {
                return Err(ValidationError::required(
                    self.required_location.clone(),
                    crate::paths::capture_evaluation_path(tracker, &self.required_location),
                    location.into(),
                    instance.to_value(),
                    Value::String(self.second.clone()),
                ));
            }
            if object.len() <= self.properties.len() {
                for (name, value) in object.members() {
                    let name = name.as_ref();
                    for (prop_name, _, node) in &self.properties {
                        if prop_name.as_str() == name {
                            node.validate(&value, &location.push(name), tracker, ctx)?;
                            break;
                        }
                    }
                }
            } else {
                for (name, key, node) in &self.properties {
                    if let Some(prop) = object.get(key) {
                        node.validate(&prop, &location.push(name), tracker, ctx)?;
                    }
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::needless_collect)]
    fn iter_errors<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Some(object) = instance.as_object() {
            let mut errors = Vec::new();
            // Check required
            let eval_path = crate::paths::capture_evaluation_path(tracker, &self.required_location);
            if object.get(&self.first_key).is_none() {
                errors.push(ValidationError::required(
                    self.required_location.clone(),
                    eval_path.clone(),
                    location.into(),
                    instance.to_value(),
                    Value::String(self.first.clone()),
                ));
            }
            if object.get(&self.second_key).is_none() {
                errors.push(ValidationError::required(
                    self.required_location.clone(),
                    eval_path,
                    location.into(),
                    instance.to_value(),
                    Value::String(self.second.clone()),
                ));
            }
            if object.len() <= self.properties.len() {
                for (name, value) in object.members() {
                    let name = name.as_ref();
                    for (prop_name, _, node) in &self.properties {
                        if prop_name.as_str() == name {
                            let instance_path = location.push(name);
                            errors.extend(node.iter_errors(&value, &instance_path, tracker, ctx));
                            break;
                        }
                    }
                }
            } else {
                for (name, key, node) in &self.properties {
                    if let Some(prop) = object.get(key) {
                        let instance_path = location.push(name.as_str());
                        errors.extend(node.iter_errors(&prop, &instance_path, tracker, ctx));
                    }
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
        instance: &F::Node<'_>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Some(object) = instance.as_object() {
            let mut matched_props = Vec::with_capacity(object.len());
            let mut children = Vec::new();
            if object.len() <= self.properties.len() {
                for (name, value) in object.members() {
                    let name = name.as_ref();
                    for (prop_name, _, node) in &self.properties {
                        if prop_name.as_str() == name {
                            let path = location.push(name);
                            matched_props.push(prop_name.clone());
                            children.push(node.evaluate_instance(&value, &path, tracker, ctx));
                            break;
                        }
                    }
                }
            } else {
                for (prop_name, key, node) in &self.properties {
                    if let Some(prop) = object.get(key) {
                        let path = location.push(prop_name.as_str());
                        matched_props.push(prop_name.clone());
                        children.push(node.evaluate_instance(&prop, &path, tracker, ctx));
                    }
                }
            }
            // `required` is fused into this validator, so its failures are emitted as a child node at
            // the `required` keyword location to keep the correct `schemaLocation` in structured output.
            let mut required_errors = Vec::new();
            let eval_path = crate::paths::capture_evaluation_path(tracker, &self.required_location);
            if object.get(&self.first_key).is_none() {
                required_errors.push(ErrorDescription::from_validation_error(
                    &ValidationError::required(
                        self.required_location.clone(),
                        eval_path.clone(),
                        location.into(),
                        instance.to_value(),
                        Value::String(self.first.clone()),
                    ),
                ));
            }
            if object.get(&self.second_key).is_none() {
                required_errors.push(ErrorDescription::from_validation_error(
                    &ValidationError::required(
                        self.required_location.clone(),
                        eval_path,
                        location.into(),
                        instance.to_value(),
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

impl<F: Json> Validate<F> for BigPropertiesValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        if let Some(object) = instance.as_object() {
            // Iterate over instance properties and look up in schema's HashMap
            for (name, prop) in object.members() {
                if let Some(node) = self.properties.get(name.as_ref()) {
                    if !node.is_valid(&prop, ctx) {
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
            for (name, value) in object.members() {
                if let Some(node) = self.properties.get(name.as_ref()) {
                    node.validate(&value, &location.push(name.as_ref()), tracker, ctx)?;
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::needless_collect)]
    fn iter_errors<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Some(object) = instance.as_object() {
            let mut errors = Vec::new();
            for (name, prop) in object.members() {
                if let Some(node) = self.properties.get(name.as_ref()) {
                    let instance_path = location.push(name.as_ref());
                    errors.extend(node.iter_errors(&prop, &instance_path, tracker, ctx));
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
            let mut matched_props = Vec::with_capacity(object.len());
            let mut children = Vec::new();
            for (prop_name, prop) in object.members() {
                if let Some(node) = self.properties.get(prop_name.as_ref()) {
                    let path = location.push(prop_name.as_ref());
                    matched_props.push(prop_name.as_ref().to_owned());
                    children.push(node.evaluate_instance(&prop, &path, tracker, ctx));
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
pub(crate) fn compile<'a, F: Json>(
    ctx: &compiler::Context<F>,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a, F>> {
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
                    Cow::Borrowed(schema),
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

    fn two_props_schema() -> Value {
        json!({"properties": {"a": {"type": "integer"}, "b": {"type": "string"}}})
    }

    fn with_extra_keys(base: Value, count: usize) -> Value {
        let Value::Object(mut map) = base else {
            unreachable!()
        };
        for i in 0..count {
            map.insert(format!("extra{i}"), json!(i));
        }
        Value::Object(map)
    }

    // len(instance) <= declared props -> single-pass iterate branch
    #[test_case(&json!({"a": 1, "b": "x"}), true)]
    #[test_case(&json!({"a": 1}), true)] // subset of declared props
    #[test_case(&json!({"a": "not-int", "b": "x"}), false)]
    #[test_case(&json!({"b": 2}), false)] // b must be string
    fn small_properties_iterate_branch(instance: &Value, expected: bool) {
        let validator = crate::validator_for(&two_props_schema()).unwrap();
        assert_eq!(validator.is_valid(instance), expected);
    }

    // len(instance) > declared props -> targeted-lookup get branch
    #[test_case(json!({"a": 1, "b": "x"}), true)]
    #[test_case(json!({"a": "not-int", "b": "x"}), false)]
    fn small_properties_get_branch_wide_instance(base: Value, expected: bool) {
        let validator = crate::validator_for(&two_props_schema()).unwrap();
        let instance = with_extra_keys(base, 300);
        assert_eq!(validator.is_valid(&instance), expected);
    }

    // validate/iter_errors reach the same verdict on both crossover branches.
    #[test_case(json!({"a": 1, "b": "x"}), true)] // small -> iterate branch
    #[test_case(json!({"a": "not-int", "b": "x"}), false)]
    fn small_properties_validate_iter_errors_crossover(base: Value, valid: bool) {
        let validator = crate::validator_for(&two_props_schema()).unwrap();
        // small instance (iterate branch)
        assert_eq!(validator.validate(&base).is_ok(), valid);
        assert_eq!(validator.iter_errors(&base).next().is_none(), valid);
        // same instance widened past the declared-prop count (get branch)
        let wide = with_extra_keys(base, 300);
        assert_eq!(validator.validate(&wide).is_ok(), valid);
        assert_eq!(validator.iter_errors(&wide).next().is_none(), valid);
    }

    // Fused variant: a missing required key fails on both crossover branches.
    #[test]
    fn fused_required_missing_both_branches() {
        let validator = crate::validator_for(&fused_schema()).unwrap();
        // len == declared props, required "b" absent -> iterate branch (seen_second == false)
        assert!(!validator.is_valid(&json!({"a": 1, "c": 3})));
        // wide instance, required "a" absent -> get branch early fast-fail
        let wide = with_extra_keys(json!({"b": "x"}), 300);
        assert!(!validator.is_valid(&wide));
    }
}
