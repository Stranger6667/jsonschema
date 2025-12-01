use crate::{
    compiler,
    error::{no_error, ErrorIterator, ValidationError},
    evaluation::Annotations,
    keywords::CompilationResult,
    node::SchemaNode,
    paths::{LazyLocation, Location},
    types::JsonType,
    validator::{EvaluationResult, Validate, ValidationContext},
};
use serde_json::{Map, Value};

pub(crate) struct PropertiesValidator {
    pub(crate) properties: Vec<(String, SchemaNode)>,
    location: Location,
}

impl PropertiesValidator {
    #[inline]
    pub(crate) fn compile<'a>(ctx: &compiler::Context, schema: &'a Value) -> CompilationResult<'a> {
        match schema {
            Value::Object(map) => {
                let ctx = ctx.new_at_location("properties");
                let mut properties = Vec::with_capacity(map.len());
                for (key, subschema) in map {
                    let ctx = ctx.new_at_location(key.as_str());
                    properties.push((
                        key.clone(),
                        compiler::compile(&ctx, ctx.as_resource_ref(subschema))?,
                    ));
                }
                Ok(Box::new(PropertiesValidator {
                    properties,
                    location: ctx.location().clone(),
                }))
            }
            _ => Err(ValidationError::single_type_error(
                Location::new(),
                ctx.location().clone(),
                schema,
                JsonType::Object,
            )),
        }
    }
}

impl Validate for PropertiesValidator {
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
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Object(item) = instance {
            for (name, node) in &self.properties {
                if let Some(item) = item.get(name) {
                    node.validate(item, &location.push(name), ctx)?;
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
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Value::Object(item) = instance {
            let mut errors = Vec::new();
            for (name, node) in &self.properties {
                if let Some(prop) = item.get(name) {
                    let instance_path = location.push(name.as_str());
                    errors.extend(node.iter_errors(prop, &instance_path, ctx));
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
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Value::Object(props) = instance {
            let mut matched_props = Vec::with_capacity(props.len());
            let mut children = Vec::new();
            for (prop_name, node) in &self.properties {
                if let Some(prop) = props.get(prop_name) {
                    let path = location.push(prop_name.as_str());
                    matched_props.push(prop_name.clone());
                    children.push(node.evaluate_instance(prop, &path, ctx));
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
        _ => Some(PropertiesValidator::compile(ctx, schema)),
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
