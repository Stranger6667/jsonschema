use crate::{
    compiler,
    error::ValidationError,
    keywords::CompilationResult,
    node::SchemaNode,
    paths::{LazyLocation, Location, RefTracker},
    types::JsonType,
    validator::{EvaluationResult, Validate, ValidationContext},
    Json, Node, Object,
};
use serde_json::{Map, Value};

pub(crate) struct PropertyNamesObjectValidator<F: Json> {
    node: SchemaNode<F>,
}

impl<F: Json> PropertyNamesObjectValidator<F> {
    #[inline]
    pub(crate) fn compile<'a>(
        ctx: &compiler::Context<F>,
        schema: &'a Value,
    ) -> CompilationResult<'a, F> {
        let ctx = ctx.new_at_location("propertyNames");
        Ok(Box::new(PropertyNamesObjectValidator {
            node: compiler::compile(&ctx, ctx.as_resource_ref(schema))?,
        }))
    }
}

impl<F: Json> Validate<F> for PropertyNamesObjectValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        if let Some(object) = instance.as_object() {
            let mut buffer = F::StringBuffer::default();
            for (name, _) in object.members() {
                let valid = F::with_string_node(&mut buffer, name.as_ref(), |node| {
                    self.node.is_valid(&node, ctx)
                });
                if !valid {
                    return false;
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
        if let Some(object) = instance.as_object() {
            let mut buffer = F::StringBuffer::default();
            for (name, _) in object.members() {
                let result = F::with_string_node(&mut buffer, name.as_ref(), |node| {
                    self.node
                        .validate(&node, location, tracker, ctx)
                        .map_err(ValidationError::to_owned)
                });
                if let Err(error) = result {
                    let schema_path = error.schema_path().clone();
                    return Err(ValidationError::property_names(
                        schema_path.clone(),
                        crate::paths::capture_evaluation_path(tracker, &schema_path),
                        location.into(),
                        instance.lazy_value(),
                        error,
                    ));
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
        let mut buffer = F::StringBuffer::default();
        let mut name_errors = Vec::new();
        for (name, _) in object.members() {
            F::with_string_node(&mut buffer, name.as_ref(), |node| {
                let mut collected = Vec::new();
                self.node
                    .collect_errors(&node, location, tracker, ctx, &mut collected);
                name_errors.extend(collected.into_iter().map(ValidationError::to_owned));
            });
            for error in name_errors.drain(..) {
                let schema_path = error.schema_path().clone();
                errors.push(ValidationError::property_names(
                    schema_path.clone(),
                    crate::paths::capture_evaluation_path(tracker, &schema_path),
                    location.into(),
                    instance.lazy_value(),
                    error,
                ));
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
            let mut children = Vec::with_capacity(object.len());
            let mut buffer = F::StringBuffer::default();
            for (name, _) in object.members() {
                children.push(F::with_string_node(&mut buffer, name.as_ref(), |node| {
                    self.node.evaluate_instance(&node, location, tracker, ctx)
                }));
            }
            EvaluationResult::from_children(children)
        } else {
            EvaluationResult::valid_empty()
        }
    }
    fn matches_type(&self, instance: &F::Node<'_>) -> bool {
        instance.json_type() == JsonType::Object
    }
    fn schema_path(&self) -> &Location {
        self.node.location()
    }
    fn trace(
        &self,
        instance: &F::Node<'_>,
        instance_path: &LazyLocation,
        callback: crate::tracing::TracingCallback<'_>,
        ctx: &mut ValidationContext,
    ) -> bool {
        if let Some(item) = instance.as_object() {
            let mut is_valid = true;
            let mut at_least_one = false;
            let mut buffer = F::StringBuffer::default();
            for (name, _) in item.members() {
                at_least_one = true;
                // Trace the subschema validation for each property name
                let key_is_valid = F::with_string_node(&mut buffer, name.as_ref(), |node| {
                    self.node.trace(&node, instance_path, callback, ctx)
                });
                is_valid &= key_is_valid;
            }
            // Report the overall propertyNames result
            let rv = if at_least_one { Some(is_valid) } else { None };
            crate::tracing::TracingContext::new(instance_path, self.schema_path(), rv)
                .call(callback);
            is_valid
        } else {
            // Not an object - validation doesn't apply
            crate::tracing::TracingContext::new(instance_path, self.schema_path(), None)
                .call(callback);
            true
        }
    }
}

pub(crate) struct PropertyNamesBooleanValidator {
    location: Location,
}

impl PropertyNamesBooleanValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(ctx: &compiler::Context<F>) -> CompilationResult<'a, F> {
        let location = ctx.location().join("propertyNames");
        Ok(Box::new(PropertyNamesBooleanValidator { location }))
    }
}

impl<F: Json> Validate<F> for PropertyNamesBooleanValidator {
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        if let Some(object) = instance.as_object() {
            if !object.is_empty() {
                return false;
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
        if <Self as Validate<F>>::is_valid(self, instance, ctx) {
            Ok(())
        } else {
            Err(ValidationError::false_schema(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance.lazy_value(),
            ))
        }
    }
    fn matches_type(&self, instance: &F::Node<'_>) -> bool {
        instance.json_type() == JsonType::Object
    }
    fn schema_path(&self) -> &Location {
        &self.location
    }
}

#[inline]
pub(crate) fn compile<'a, F: Json>(
    ctx: &compiler::Context<F>,
    _: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a, F>> {
    match schema {
        Value::Object(_) => Some(PropertyNamesObjectValidator::compile(ctx, schema)),
        Value::Bool(false) => Some(PropertyNamesBooleanValidator::compile(ctx)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use serde_json::{json, Value};
    use test_case::test_case;

    // Each key is validated on its own, not concatenated with the ones before it
    #[test_case(&json!({"propertyNames": {"maxLength": 3}}), &json!({"abc": 1, "de": 1}))]
    #[test_case(&json!({"propertyNames": {"maxLength": 3}}), &json!({"a": 1, "bc": 1, "def": 1}))]
    fn keys_validate_independently(schema: &Value, instance: &Value) {
        tests_util::is_valid(schema, instance);
    }

    #[test_case(&json!({"propertyNames": {"maxLength": 2}}), &json!({"ab": 1, "cde": 1}))]
    #[test_case(&json!({"propertyNames": {"minLength": 2}}), &json!({"ab": 1, "c": 1}))]
    fn invalid_key_is_reported(schema: &Value, instance: &Value) {
        tests_util::is_not_valid(schema, instance);
    }

    #[test_case(&json!({"propertyNames": false}), &json!({"foo": 1}), "/propertyNames")]
    #[test_case(&json!({"propertyNames": {"minLength": 2}}), &json!({"f": 1}), "/propertyNames/minLength")]
    fn location(schema: &Value, instance: &Value, expected: &str) {
        tests_util::assert_schema_location(schema, instance, expected);
    }
}
