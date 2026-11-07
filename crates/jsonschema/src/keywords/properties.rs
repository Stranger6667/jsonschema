use crate::LazyInstance;
use std::borrow::Cow;

use crate::{
    compiler,
    error::ValidationError,
    evaluation::{
        format_keyword_location, Annotations, ChildList, ErrorDescription, EvaluationNode,
    },
    keywords::CompilationResult,
    node::SchemaNode,
    paths::{LazyLocation, Location, RefTracker},
    properties::{
        compile_big_map, scan_members, BigValidatorsMap, KeyHead, NameLengths,
        PropertiesValidatorsMap as _, PropertyName, HASHMAP_THRESHOLD,
    },
    types::JsonType,
    validator::{EvaluationResult, Validate, ValidationContext},
    Json, Node, Object, SerdeJson,
};
use referencing::{Uri, Vocabulary};
use serde_json::{Map, Value};
use std::sync::Arc;

pub(crate) struct SmallPropertiesValidator<F: Json = SerdeJson> {
    pub(crate) properties: Vec<(PropertyName, F::PreparedKey, SchemaNode<F>)>,
    lengths: NameLengths,
}

pub(crate) struct BigPropertiesValidator<F: Json = SerdeJson> {
    pub(crate) properties: BigValidatorsMap<F>,
}

/// Fused validator for `properties` + `required: [2 items]` (no `additionalProperties: false`).
/// Eliminates separate required validation pass and duplicate `BTreeMap` lookups.
pub(crate) struct SmallPropertiesWithRequired2Validator<F: Json = SerdeJson> {
    pub(crate) properties: Vec<(PropertyName, F::PreparedKey, SchemaNode<F>)>,
    first: PropertyName,
    first_key: F::PreparedKey,
    second: PropertyName,
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
                PropertyName::new(key.clone()),
                F::prepare_key(key),
                compiler::compile(&ctx, ctx.as_resource_ref(subschema))?,
            ));
        }
        Ok(Box::new(SmallPropertiesValidator {
            properties,
            lengths: NameLengths::of(map.keys().map(String::as_str)),
        }))
    }
}

impl BigPropertiesValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        map: &'a Map<String, Value>,
    ) -> CompilationResult<'a, F> {
        Ok(Box::new(BigPropertiesValidator {
            properties: compile_big_map(ctx, map)?,
        }))
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
                PropertyName::new(key.clone()),
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
            first: PropertyName::new(first),
            second: PropertyName::new(second),
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
        // Short instance in one pass, wide instance via targeted lookups.
        if scan_members(object.len(), self.properties.len()) {
            for (name, value) in object.members() {
                let name = name.as_ref();
                if !self.lengths.may_contain(name.len()) {
                    continue;
                }
                let head = KeyHead::of(name);
                for (prop_name, _, node) in &self.properties {
                    if prop_name.matches(head, name) {
                        if !node.is_valid_child(&value, ctx) {
                            return false;
                        }
                        break;
                    }
                }
            }
        } else {
            for (_, key, node) in &self.properties {
                if let Some(prop) = object.get(key) {
                    if !node.is_valid_child(&prop, ctx) {
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
        if scan_members(object.len(), self.properties.len()) {
            for (name, value) in object.members() {
                let name = name.as_ref();
                let head = KeyHead::of(name);
                for (prop_name, _, node) in &self.properties {
                    if prop_name.matches(head, name) {
                        node.validate(&value, &location.push(name), tracker, ctx)?;
                        break;
                    }
                }
            }
        } else {
            for (name, key, node) in &self.properties {
                if let Some(prop) = object.get(key) {
                    node.validate(&prop, &location.push(name.as_str()), tracker, ctx)?;
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
        if scan_members(object.len(), self.properties.len()) {
            for (name, value) in object.members() {
                let name = name.as_ref();
                let head = KeyHead::of(name);
                for (prop_name, _, node) in &self.properties {
                    if prop_name.matches(head, name) {
                        let instance_path = location.push(name);
                        node.collect_errors(&value, &instance_path, tracker, ctx, errors);
                        break;
                    }
                }
            }
        } else {
            for (name, key, node) in &self.properties {
                if let Some(prop) = object.get(key) {
                    let instance_path = location.push(name.as_str());
                    node.collect_errors(&prop, &instance_path, tracker, ctx, errors);
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
        let Some(object) = instance.as_object() else {
            return EvaluationResult::valid_empty();
        };
        let mut matched_props = Vec::with_capacity(object.len());
        let mut children = ChildList::default();
        if scan_members(object.len(), self.properties.len()) {
            for (name, value) in object.members() {
                let name = name.as_ref();
                let head = KeyHead::of(name);
                for (prop_name, _, node) in &self.properties {
                    if prop_name.matches(head, name) {
                        let path = location.push(name);
                        matched_props.push(prop_name.as_str().to_owned());
                        let child = node.evaluate_instance_below(&value, &path, tracker, ctx);
                        children.push(ctx.arena(), child);
                        break;
                    }
                }
            }
        } else {
            for (prop_name, key, node) in &self.properties {
                if let Some(prop) = object.get(key) {
                    let path = location.push(prop_name.as_str());
                    matched_props.push(prop_name.as_str().to_owned());
                    let child = node.evaluate_instance_below(&prop, &path, tracker, ctx);
                    children.push(ctx.arena(), child);
                }
            }
        }
        let mut application = EvaluationResult::from_children(children);
        application.annotate(Annotations::new(Value::from(matched_props)));
        application
    }
}

impl<F: Json> SmallPropertiesWithRequired2Validator<F> {
    #[cold]
    #[inline(never)]
    fn missing<'i>(
        &self,
        key: &PropertyName,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
    ) -> ValidationError<'i> {
        ValidationError::required(
            self.required_location.clone(),
            crate::paths::capture_evaluation_path(tracker, &self.required_location),
            location.into(),
            instance.lazy_value(),
            Value::String(key.as_str().to_owned()),
        )
        .with_absolute_keyword_location(self.required_absolute_location.clone())
    }

    fn check_required<'i, O: Object<'i, F>>(
        &self,
        instance: &F::Node<'i>,
        object: &O,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
    ) -> Result<(), ValidationError<'i>> {
        if object.get(&self.first_key).is_none() {
            return Err(self.missing(&self.first, instance, location, tracker));
        }
        if object.get(&self.second_key).is_none() {
            return Err(self.missing(&self.second, instance, location, tracker));
        }
        Ok(())
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
        if scan_members(object.len(), self.properties.len()) {
            // One pass validates matching properties and confirms both required keys.
            let mut seen_first = false;
            let mut seen_second = false;
            for (name, value) in object.members() {
                let name = name.as_ref();
                let head = KeyHead::of(name);
                if self.first.matches(head, name) {
                    seen_first = true;
                } else if self.second.matches(head, name) {
                    seen_second = true;
                }
                for (prop_name, _, node) in &self.properties {
                    if prop_name.matches(head, name) {
                        if !node.is_valid_child(&value, ctx) {
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
                    if !node.is_valid_child(&prop, ctx) {
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
        let Some(object) = instance.as_object() else {
            return Ok(());
        };
        if scan_members(object.len(), self.properties.len()) {
            // One pass validates matching properties and confirms both required keys.
            let mut seen_first = false;
            let mut seen_second = false;
            for (name, value) in object.members() {
                let name = name.as_ref();
                let head = KeyHead::of(name);
                if self.first.matches(head, name) {
                    seen_first = true;
                } else if self.second.matches(head, name) {
                    seen_second = true;
                }
                for (prop_name, _, node) in &self.properties {
                    if prop_name.matches(head, name) {
                        if let Err(error) =
                            node.validate(&value, &location.push(name), tracker, ctx)
                        {
                            // A missing required name is reported before a property error.
                            self.check_required(instance, &object, location, tracker)?;
                            return Err(error);
                        }
                        break;
                    }
                }
            }
            if !seen_first {
                return Err(self.missing(&self.first, instance, location, tracker));
            }
            if !seen_second {
                return Err(self.missing(&self.second, instance, location, tracker));
            }
        } else {
            self.check_required(instance, &object, location, tracker)?;
            for (name, key, node) in &self.properties {
                if let Some(prop) = object.get(key) {
                    node.validate(&prop, &location.push(name.as_str()), tracker, ctx)?;
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
        {
            // Check required
            if object.get(&self.first_key).is_none() {
                errors.push(self.missing(&self.first, instance, location, tracker));
            }
            if object.get(&self.second_key).is_none() {
                errors.push(self.missing(&self.second, instance, location, tracker));
            }
            if scan_members(object.len(), self.properties.len()) {
                for (name, value) in object.members() {
                    let name = name.as_ref();
                    let head = KeyHead::of(name);
                    for (prop_name, _, node) in &self.properties {
                        if prop_name.matches(head, name) {
                            let instance_path = location.push(name);
                            node.collect_errors(&value, &instance_path, tracker, ctx, errors);
                            break;
                        }
                    }
                }
            } else {
                for (name, key, node) in &self.properties {
                    if let Some(prop) = object.get(key) {
                        let instance_path = location.push(name.as_str());
                        node.collect_errors(&prop, &instance_path, tracker, ctx, errors);
                    }
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
            let mut matched_props = Vec::with_capacity(object.len());
            let mut children = ChildList::default();
            if scan_members(object.len(), self.properties.len()) {
                for (name, value) in object.members() {
                    let name = name.as_ref();
                    let head = KeyHead::of(name);
                    for (prop_name, _, node) in &self.properties {
                        if prop_name.matches(head, name) {
                            let path = location.push(name);
                            matched_props.push(prop_name.as_str().to_owned());
                            let child = node.evaluate_instance_below(&value, &path, tracker, ctx);
                            children.push(ctx.arena(), child);
                            break;
                        }
                    }
                }
            } else {
                for (prop_name, key, node) in &self.properties {
                    if let Some(prop) = object.get(key) {
                        let path = location.push(prop_name.as_str());
                        matched_props.push(prop_name.as_str().to_owned());
                        let child = node.evaluate_instance_below(&prop, &path, tracker, ctx);
                        children.push(ctx.arena(), child);
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
                        instance.lazy_value(),
                        Value::String(self.first.as_str().to_owned()),
                    ),
                ));
            }
            if object.get(&self.second_key).is_none() {
                required_errors.push(ErrorDescription::from_validation_error(
                    &ValidationError::required(
                        self.required_location.clone(),
                        eval_path,
                        location.into(),
                        instance.lazy_value(),
                        Value::String(self.second.as_str().to_owned()),
                    ),
                ));
            }
            if !required_errors.is_empty() {
                let child = EvaluationNode::invalid(
                    crate::paths::evaluation_path(tracker, &self.required_location, ctx),
                    self.required_absolute_location.clone(),
                    format_keyword_location(
                        &self.required_location,
                        self.required_absolute_location.as_ref(),
                    ),
                    ctx.instance_position(location),
                    None,
                    required_errors,
                    ChildList::default(),
                );
                children.push(ctx.arena(), child);
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
                if let Some(node) = self.properties.get_validator(name.as_ref()) {
                    if !node.is_valid_child(&prop, ctx) {
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
                if let Some(node) = self.properties.get_validator(name.as_ref()) {
                    node.validate(&value, &location.push(name.as_ref()), tracker, ctx)?;
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
        for (name, prop) in object.members() {
            if let Some(node) = self.properties.get_validator(name.as_ref()) {
                let instance_path = location.push(name.as_ref());
                node.collect_errors(&prop, &instance_path, tracker, ctx, errors);
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
            let mut matched_props = Vec::with_capacity(object.len());
            let mut children = ChildList::default();
            for (prop_name, prop) in object.members() {
                if let Some(node) = self.properties.get_validator(prop_name.as_ref()) {
                    let path = location.push(prop_name.as_ref());
                    matched_props.push(prop_name.as_ref().to_owned());
                    let child = node.evaluate_instance_below(&prop, &path, tracker, ctx);
                    children.push(ctx.arena(), child);
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
fn extract_required2<F: Json>(
    ctx: &compiler::Context<F>,
    parent: &Map<String, Value>,
) -> Option<(String, String)> {
    // `required` compiles under the validation vocabulary only
    if !ctx.has_vocabulary(&Vocabulary::Validation) {
        return None;
    }
    // No patternProperties (uses separate validator paths)
    if parent.contains_key("patternProperties") {
        return None;
    }
    if ctx.is_keyword_overridden("required") {
        return None;
    }
    // `required::compile` keeps its own validator once `additionalProperties` takes this shape
    if matches!(
        parent.get("additionalProperties"),
        Some(Value::Bool(false) | Value::Object(_))
    ) {
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
        Some(Value::Bool(false) | Value::Object(_))
            if !ctx.is_keyword_overridden("additionalProperties") =>
        {
            None
        }
        _ => {
            if let Value::Object(map) = schema {
                if map.len() < HASHMAP_THRESHOLD {
                    // Try fused validator for properties + required: [2 items]
                    if let Some((first, second)) = extract_required2(ctx, parent) {
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
                    LazyInstance::Ready(Cow::Borrowed(schema)),
                    JsonType::Object,
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use serde_json::{json, Map, Value};
    use test_case::test_case;

    // Member subschemas that only check `type`, against every kind of value
    #[test_case(&json!({"type": "string"}), &json!(null), false; "string child, null")]
    #[test_case(&json!({"type": "string"}), &json!(true), false; "string child, true")]
    #[test_case(&json!({"type": "string"}), &json!(1), false; "string child, 1")]
    #[test_case(&json!({"type": "string"}), &json!(1.0), false; "string child, 1.0")]
    #[test_case(&json!({"type": "string"}), &json!(1.5), false; "string child, 1.5")]
    #[test_case(&json!({"type": "string"}), &json!("s"), true; "string child, string")]
    #[test_case(&json!({"type": "string"}), &json!([]), false; "string child, array")]
    #[test_case(&json!({"type": "string"}), &json!({}), false; "string child, object")]
    #[test_case(&json!({"type": ["null", "object"]}), &json!(null), true; "null or object child, null")]
    #[test_case(&json!({"type": ["null", "object"]}), &json!(true), false; "null or object child, true")]
    #[test_case(&json!({"type": ["null", "object"]}), &json!(1), false; "null or object child, 1")]
    #[test_case(&json!({"type": ["null", "object"]}), &json!(1.0), false; "null or object child, 1.0")]
    #[test_case(&json!({"type": ["null", "object"]}), &json!(1.5), false; "null or object child, 1.5")]
    #[test_case(&json!({"type": ["null", "object"]}), &json!("s"), false; "null or object child, string")]
    #[test_case(&json!({"type": ["null", "object"]}), &json!([]), false; "null or object child, array")]
    #[test_case(&json!({"type": ["null", "object"]}), &json!({}), true; "null or object child, object")]
    #[test_case(&json!({"type": "number"}), &json!(null), false; "number child, null")]
    #[test_case(&json!({"type": "number"}), &json!(true), false; "number child, true")]
    #[test_case(&json!({"type": "number"}), &json!(1), true; "number child, 1")]
    #[test_case(&json!({"type": "number"}), &json!(1.0), true; "number child, 1.0")]
    #[test_case(&json!({"type": "number"}), &json!(1.5), true; "number child, 1.5")]
    #[test_case(&json!({"type": "number"}), &json!("s"), false; "number child, string")]
    #[test_case(&json!({"type": "number"}), &json!([]), false; "number child, array")]
    #[test_case(&json!({"type": "number"}), &json!({}), false; "number child, object")]
    #[test_case(&json!({"type": "integer"}), &json!(null), false; "integer child, null")]
    #[test_case(&json!({"type": "integer"}), &json!(true), false; "integer child, true")]
    #[test_case(&json!({"type": "integer"}), &json!(1), true; "integer child, 1")]
    #[test_case(&json!({"type": "integer"}), &json!(1.0), true; "integer child, 1.0")]
    #[test_case(&json!({"type": "integer"}), &json!(1.5), false; "integer child, 1.5")]
    #[test_case(&json!({"type": "integer"}), &json!("s"), false; "integer child, string")]
    #[test_case(&json!({"type": "integer"}), &json!([]), false; "integer child, array")]
    #[test_case(&json!({"type": "integer"}), &json!({}), false; "integer child, object")]
    #[test_case(&json!(true), &json!(null), true; "true child, null")]
    #[test_case(&json!(true), &json!(true), true; "true child, true")]
    #[test_case(&json!(true), &json!(1), true; "true child, 1")]
    #[test_case(&json!(true), &json!(1.0), true; "true child, 1.0")]
    #[test_case(&json!(true), &json!(1.5), true; "true child, 1.5")]
    #[test_case(&json!(true), &json!("s"), true; "true child, string")]
    #[test_case(&json!(true), &json!([]), true; "true child, array")]
    #[test_case(&json!(true), &json!({}), true; "true child, object")]
    #[test_case(&json!(false), &json!(null), false; "false child, null")]
    #[test_case(&json!(false), &json!(true), false; "false child, true")]
    #[test_case(&json!(false), &json!(1), false; "false child, 1")]
    #[test_case(&json!(false), &json!(1.0), false; "false child, 1.0")]
    #[test_case(&json!(false), &json!(1.5), false; "false child, 1.5")]
    #[test_case(&json!(false), &json!("s"), false; "false child, string")]
    #[test_case(&json!(false), &json!([]), false; "false child, array")]
    #[test_case(&json!(false), &json!({}), false; "false child, object")]
    #[test_case(&json!({}), &json!(null), true; "empty child, null")]
    #[test_case(&json!({}), &json!(true), true; "empty child, true")]
    #[test_case(&json!({}), &json!(1), true; "empty child, 1")]
    #[test_case(&json!({}), &json!(1.0), true; "empty child, 1.0")]
    #[test_case(&json!({}), &json!(1.5), true; "empty child, 1.5")]
    #[test_case(&json!({}), &json!("s"), true; "empty child, string")]
    #[test_case(&json!({}), &json!([]), true; "empty child, array")]
    #[test_case(&json!({}), &json!({}), true; "empty child, object")]
    fn child_matches_is_valid(child: &Value, value: &Value, expected: bool) {
        let schema = json!({"properties": {"a": child}});
        let instance = json!({"a": value});
        let validator = crate::validator_for(&schema).expect("schema compiles");
        assert_eq!(validator.is_valid(&instance), expected);
        assert_eq!(validator.validate(&instance).is_ok(), expected);
        assert_eq!(validator.evaluate(&instance).flag().valid, expected);
    }

    #[test_case(&json!({"$schema": "http://json-schema.org/draft-04/schema#", "properties": {"a": {"type": "integer"}}}), &json!({"a": 1.0}), false; "draft 4 integer rejects 1.0")]
    #[test_case(&json!({"$schema": "http://json-schema.org/draft-07/schema#", "definitions": {"s": {"type": "string"}}, "properties": {"a": {"$ref": "#/definitions/s", "type": "number"}}}), &json!({"a": "x"}), true; "draft 7 ignores type beside ref")]
    #[test_case(&json!({"properties": {"a": {"type": "string"}}, "additionalProperties": {"type": "integer"}}), &json!({"a": "x", "b": 2}), true; "additional properties typed")]
    #[test_case(&json!({"properties": {"a": {"type": "string"}}, "additionalProperties": {"type": "integer"}}), &json!({"a": "x", "b": "y"}), false; "additional properties wrong type")]
    #[test_case(&json!({"patternProperties": {"^x-": {"type": "boolean"}}}), &json!({"x-a": true}), true; "pattern properties typed")]
    #[test_case(&json!({"patternProperties": {"^x-": {"type": "boolean"}}}), &json!({"x-a": 1}), false; "pattern properties wrong type")]
    fn typed_members(schema: &Value, instance: &Value, expected: bool) {
        let validator = crate::validator_for(schema).expect("schema compiles");
        assert_eq!(validator.is_valid(instance), expected);
        assert_eq!(validator.validate(instance).is_ok(), expected);
        assert_eq!(validator.evaluate(instance).flag().valid, expected);
    }

    #[test]
    fn location() {
        tests_util::assert_schema_location(
            &json!({"properties": {"foo": {"properties": {"bar": {"required": ["spam"]}}}}}),
            &json!({"foo": {"bar": {}}}),
            "/properties/foo/properties/bar/required",
        );
    }

    // `same-head*` names share a slot: they differ only in the middle.
    fn many_properties(additional: Option<bool>) -> Value {
        let mut properties = Map::new();
        for index in 0..20 {
            properties.insert(format!("p{index}"), json!({"const": index}));
        }
        for middle in ["A", "B", "C"] {
            let name = format!("same-head{middle}same-tail");
            properties.insert(name.clone(), json!({"const": name}));
        }
        let mut schema = json!({ "properties": properties });
        if let Some(additional) = additional {
            schema["additionalProperties"] = json!(additional);
        }
        schema
    }

    #[test_case(None, &json!({"p0": 0, "p19": 19}), true; "declared names")]
    #[test_case(None, &json!({"p7": 8}), false; "declared name with a wrong value")]
    #[test_case(None, &json!({"same-headBsame-tail": "same-headBsame-tail"}), true; "same head")]
    #[test_case(None, &json!({"same-headBsame-tail": "same-headCsame-tail"}), false; "same head, other value")]
    #[test_case(None, &json!({"same-headDsame-tail": 1, "p20": 1, "": 1}), true; "undeclared names")]
    #[test_case(Some(false), &json!({"p3": 3, "same-headAsame-tail": "same-headAsame-tail"}), true; "closed, declared names")]
    #[test_case(Some(false), &json!({"same-headBsame-tail": "same-headBsame-tail", "same-headCsame-tail": "same-headCsame-tail"}), true; "closed, every name with the same head")]
    #[test_case(Some(false), &json!({"same-headDsame-tail": 1}), false; "closed, undeclared name with a declared head")]
    #[test_case(Some(false), &json!({"p20": 1}), false; "closed, undeclared name")]
    #[test_case(Some(false), &json!({"p2": 3}), false; "closed, declared name with a wrong value")]
    fn many_properties_lookup(additional: Option<bool>, instance: &Value, expected: bool) {
        let validator = crate::validator_for(&many_properties(additional)).unwrap();
        assert_eq!(validator.is_valid(instance), expected);
        assert_eq!(validator.validate(instance).is_ok(), expected);
        assert_eq!(validator.iter_errors(instance).next().is_none(), expected);
    }

    // 70 bytes, and 6 bytes: lengths 64 apart share a bit in the length filter.
    const LONG: &str = "property-name-that-is-seventy-bytes-long-so-that-it-wraps-past-sixty-4";

    #[test_case(&json!({LONG: 1}), true; "long name")]
    #[test_case(&json!({LONG: "x"}), false; "long name with a wrong value")]
    #[test_case(&json!({"abcdef": "x"}), false; "short name with a wrong value")]
    #[test_case(&json!({"ab": "x"}), true; "undeclared name")]
    #[test_case(&json!({"abcdeg": "x", LONG: 1}), true; "undeclared name sharing a length")]
    #[test_case(&json!({"property-name-that-is-seventy-bytes-long-so-that-it-wraps-past-sixty-5": "x"}), true; "undeclared long name")]
    fn names_by_length(instance: &Value, expected: bool) {
        let schema = json!({
            "properties": {
                LONG: {"type": "integer"},
                "abcdef": {"type": "integer"},
                "a": {"type": "integer"}
            }
        });
        let validator = crate::validator_for(&schema).unwrap();
        assert_eq!(validator.is_valid(instance), expected);
        assert_eq!(validator.validate(instance).is_ok(), expected);
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

    // Declared names and instance keys that agree on length and on long prefixes
    #[test_case("abcdefghijklX", "abcdefghijklX", true; "thirteen bytes equal")]
    #[test_case("abcdefghijklX", "abcdefghijklY", false; "thirteen bytes differing after the head")]
    #[test_case("abcdefghijkl", "abcdefghijkl", true; "twelve bytes equal")]
    #[test_case("abcdefghijkl", "abcdefghijkL", false; "twelve bytes differing in the last byte")]
    #[test_case("abcdEFGHijkl", "abcdefghijkl", false; "twelve bytes differing in the middle")]
    #[test_case("abcd", "abcd", true; "four bytes equal")]
    #[test_case("abcd", "abce", false; "four bytes differing")]
    #[test_case("abcd", "abcde", false; "prefix of a longer key")]
    #[test_case("", "", true; "empty equal")]
    #[test_case("", "a", false; "empty against one byte")]
    #[test_case("café-résumé-naïve", "café-résumé-naïve", true; "multibyte equal")]
    #[test_case("café-résumé-naïve", "café-rÉsumé-naïve", false; "multibyte differing after the head")]
    fn property_name_matching(declared: &str, key: &str, matches: bool) {
        let instance = json!({key: 1, "p": 1});
        let scanned = json!({"properties": {declared: false, "p": true}});
        assert_eq!(crate::is_valid(&scanned, &instance), !matches);
        let fused =
            json!({"properties": {declared: false, "p": true}, "required": [declared, "p"]});
        assert!(!crate::is_valid(&fused, &instance));
        let fused_present =
            json!({"properties": {declared: true, "p": true}, "required": [declared, "p"]});
        assert_eq!(crate::is_valid(&fused_present, &instance), matches);
        let additional = json!({
            "properties": {declared: {"type": "integer"}},
            "additionalProperties": {"type": "string"},
        });
        assert_eq!(crate::is_valid(&additional, &json!({key: 1})), matches);
    }

    // Names agreeing on length, first and last eight bytes share a head
    fn colliding_name(middle: usize) -> String {
        format!("prefix00{middle:02}suffix00")
    }

    #[test_case("plain", 3, 3, true)]
    #[test_case("plain", 3, 4, false)]
    #[test_case("plain", 13, 13, true)]
    #[test_case("plain", 0, 13, false)]
    #[test_case("plain", 99, 0, true)]
    #[test_case("fused", 3, 3, true)]
    #[test_case("fused", 3, 4, false)]
    #[test_case("fused", 13, 13, true)]
    #[test_case("additional", 3, 3, true)]
    #[test_case("additional", 3, 4, false)]
    #[test_case("additional", 99, 0, false)]
    fn colliding_heads_select_the_named_property(
        kind: &str,
        key: usize,
        value: usize,
        expected: bool,
    ) {
        let properties: Map<String, Value> = (0..14)
            .map(|index| (colliding_name(index), json!({"const": index})))
            .collect();
        let mut schema = json!({"properties": properties});
        let mut instance = json!({colliding_name(key): value});
        match kind {
            "fused" => {
                schema["required"] = json!([colliding_name(0), colliding_name(1)]);
                instance[colliding_name(0)] = json!(0);
                instance[colliding_name(1)] = json!(1);
            }
            "additional" => schema["additionalProperties"] = json!(false),
            _ => {}
        }
        assert_eq!(crate::is_valid(&schema, &instance), expected);
    }

    // Required names outside `properties`
    #[test_case(&json!({"a": 1, "z": 2}), true)]
    #[test_case(&json!({"a": 1}), false)]
    #[test_case(&json!({"z": 2, "y": 3}), false)]
    #[test_case(&json!({"a": "x", "z": 2}), false)]
    fn fused_required_outside_properties(instance: &Value, expected: bool) {
        let schema = json!({
            "properties": {"a": {"type": "integer"}, "b": {}},
            "required": ["a", "z"],
        });
        assert_eq!(crate::is_valid(&schema, instance), expected);
        assert_eq!(
            crate::validator_for(&schema)
                .unwrap()
                .validate(instance)
                .is_ok(),
            expected
        );
    }

    // A missing required name is reported before an invalid property
    #[test_case(&json!({"a": "x"}), r#""b" is a required property"#)]
    #[test_case(&json!({"b": 1}), r#""a" is a required property"#)]
    #[test_case(&json!({"a": "x", "b": "y"}), r#""x" is not of type "integer""#)]
    fn fused_error_precedence(instance: &Value, expected: &str) {
        let validator = crate::validator_for(&fused_schema()).unwrap();
        assert_eq!(
            validator.validate(instance).unwrap_err().to_string(),
            expected
        );
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

    #[test]
    fn fused_required_absolute_keyword_locations() {
        tests_util::assert_absolute_keyword_locations(
            &json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "$id": "https://example.com/s.json",
                "properties": {"a": {"type": "string"}},
                "required": ["a", "b"]
            }),
            &json!({}),
            &[
                ("required", "https://example.com/s.json#/required"),
                ("required", "https://example.com/s.json#/required"),
            ],
        );
    }

    // `required` is inert, while `properties` still applies.
    #[test_case(&json!({"properties": {"a": false}, "required": ["b", "c"]}); "two names")]
    #[test_case(&json!({"properties": {"a": false}, "required": ["b", "c", "d"]}); "three names")]
    fn properties_without_validation_vocabulary(schema: &Value) {
        let instances = [json!({}), json!({"a": 1})];
        assert_eq!(
            tests_util::outcomes_with_only_vocabulary("applicator", schema, &instances),
            [
                (true, Vec::new()),
                (
                    false,
                    vec![(
                        "/properties/a".to_string(),
                        "False schema does not allow 1".to_string()
                    )]
                ),
            ]
        );
    }

    #[test_case(
        &json!({}),
        &[
            ("\"a\" is a required property", "", "/required"),
            ("\"b\" is a required property", "", "/required"),
        ];
        "required alone"
    )]
    #[test_case(
        &json!({"a": 1}),
        &[
            ("\"b\" is a required property", "", "/required"),
            ("1 is not of type \"string\"", "/a", "/properties/a/type"),
        ];
        "required with properties"
    )]
    #[test_case(
        &json!({"a": 1, "b": 1}),
        &[("1 is not of type \"string\"", "/a", "/properties/a/type")];
        "properties alone"
    )]
    fn fused_required_error_locations(instance: &Value, expected: &[(&str, &str, &str)]) {
        tests_util::assert_error_locations(
            &json!({"properties": {"a": {"type": "string"}}, "required": ["a", "b"]}),
            instance,
            expected,
        );
    }

    #[test]
    fn malformed() {
        tests_util::assert_compile_error(
            &json!({"properties": 5}),
            "5 is not of type \"object\"",
            "/properties",
        );
    }

    // Two declared properties; four keys or fewer per property take the members pass, more take lookups.
    #[test_case(json!({"a": 1, "b": "x"}), true; "members pass valid")]
    #[test_case(json!({"a": "x", "b": "x", "c": 0, "d": 0, "e": 0}), false; "members pass invalid")]
    #[test_case(json!({"a": 1, "b": "x", "c": 0, "d": 0, "e": 0, "f": 0, "g": 0, "h": 0}), true; "members pass at the bound")]
    #[test_case(json!({"a": 1, "b": "x", "c": 0, "d": 0, "e": 0, "f": 0, "g": 0, "h": 0, "i": 0}), true; "lookups valid")]
    #[test_case(json!({"a": 1, "b": 2, "c": 0, "d": 0, "e": 0, "f": 0, "g": 0, "h": 0, "i": 0}), false; "lookups invalid")]
    #[test_case(json!({"c": 0, "d": 0, "e": 0, "f": 0, "g": 0, "h": 0, "i": 0, "j": 0, "k": 0}), true; "lookups none declared")]
    #[allow(clippy::needless_pass_by_value)]
    fn verdicts_on_both_sides_of_the_scan_bound(instance: Value, expected: bool) {
        for required in [json!([]), json!(["a", "b"])] {
            let schema = json!({
                "properties": {"a": {"type": "integer"}, "b": {"type": "string"}},
                "required": required
            });
            let validator = crate::validator_for(&schema).expect("schema builds");
            let expected = expected
                && (required.as_array().unwrap().is_empty() || instance.get("a").is_some());
            assert_eq!(validator.is_valid(&instance), expected);
            assert_eq!(validator.validate(&instance).is_ok(), expected);
        }
    }
}
