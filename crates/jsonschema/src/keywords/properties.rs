use crate::{
    compiler,
    error::{no_error, ErrorIterator, ValidationError},
    evaluation::Annotations,
    keywords::CompilationResult,
    node::SchemaNode,
    paths::{LazyLocation, Location, RefTracker},
    types::JsonType,
    validator::{EvaluationResult, Validate, ValidationContext},
};
use ahash::AHashMap;
use serde_json::{Map, Value};

/// Threshold for switching from Vec to `HashMap` storage.
/// For small schemas, linear search on Vec is faster than `HashMap` lookup
/// due to cache locality and no hashing overhead.
const HASHMAP_THRESHOLD: usize = 40;

pub(crate) struct SmallPropertiesValidator {
    pub(crate) properties: Vec<(String, SchemaNode)>,
}

pub(crate) struct BigPropertiesValidator {
    pub(crate) properties: AHashMap<String, SchemaNode>,
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
        Ok(Box::new(SmallPropertiesValidator { properties }))
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
        Ok(Box::new(BigPropertiesValidator { properties }))
    }
}

impl Validate for SmallPropertiesValidator {
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

impl Validate for BigPropertiesValidator {
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
                    Some(SmallPropertiesValidator::compile(ctx, map))
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
    use serde_json::json;

    #[test]
    fn location() {
        tests_util::assert_schema_location(
            &json!({"properties": {"foo": {"properties": {"bar": {"required": ["spam"]}}}}}),
            &json!({"foo": {"bar": {}}}),
            "/properties/foo/properties/bar/required",
        );
    }
}
