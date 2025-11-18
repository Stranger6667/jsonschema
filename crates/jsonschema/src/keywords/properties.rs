use crate::{
    compiler,
    error::{no_error, ErrorIterator, ValidationError},
    evaluation::Annotations,
    keywords::CompilationResult,
    node::SchemaNode,
    paths::{LazyLocation, Location},
    types::JsonType,
    validator::{EvaluationResult, Validate},
};
use serde_json::{Map, Value};

pub(crate) struct PropertiesValidator {
    pub(crate) properties: Vec<(String, SchemaNode)>,
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
                Ok(Box::new(PropertiesValidator { properties }))
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
    #[allow(clippy::needless_collect)]
    fn iter_errors<'i>(&self, instance: &'i Value, location: &LazyLocation) -> ErrorIterator<'i> {
        if let Value::Object(item) = instance {
            let errors: Vec<_> = self
                .properties
                .iter()
                .flat_map(move |(name, node)| {
                    let option = item.get(name);
                    option.into_iter().flat_map(move |item| {
                        let instance_path = location.push(name.as_str());
                        node.iter_errors(item, &instance_path)
                    })
                })
                .collect();
            ErrorIterator::from_iterator(errors.into_iter())
        } else {
            no_error()
        }
    }

    fn is_valid(&self, instance: &Value) -> bool {
        if let Value::Object(item) = instance {
            self.properties.iter().all(move |(name, node)| {
                let option = item.get(name);
                option.into_iter().all(move |item| node.is_valid(item))
            })
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Object(item) = instance {
            for (name, node) in &self.properties {
                if let Some(item) = item.get(name) {
                    node.validate(item, &location.push(name))?;
                }
            }
        }
        Ok(())
    }

    fn evaluate(&self, instance: &Value, location: &LazyLocation) -> EvaluationResult {
        if let Value::Object(props) = instance {
            let mut matched_props = Vec::with_capacity(props.len());
            let mut children = Vec::new();
            for (prop_name, node) in &self.properties {
                if let Some(prop) = props.get(prop_name) {
                    let path = location.push(prop_name.as_str());
                    matched_props.push(prop_name.clone());
                    children.push(node.evaluate_instance(prop, &path));
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
