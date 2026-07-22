use crate::{
    compiler,
    error::{no_error, ErrorIterator, ValidationError},
    keywords::CompilationResult,
    node::SchemaNode,
    paths::{LazyLocation, Location, RefTracker},
    validator::{EvaluationResult, Validate, ValidationContext},
    Json, Node, Object, SerdeJson,
};
use serde_json::{Map, Value};

pub(crate) struct PropertyNamesObjectValidator {
    // Property names are always strings, validated via a materialized `Value::String`, so the names
    // subschema is compiled against `serde_json` regardless of the instance representation.
    node: SchemaNode<SerdeJson>,
}

impl PropertyNamesObjectValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        schema: &'a Value,
    ) -> CompilationResult<'a, F> {
        let ctx = ctx.to_representation::<SerdeJson>();
        let ctx = ctx.new_at_location("propertyNames");
        Ok(Box::new(PropertyNamesObjectValidator {
            node: compiler::compile(&ctx, ctx.as_resource_ref(schema))?,
        }))
    }
}

// The names subschema takes a `&Value`, so each key is written into one reusable buffer instead of
// allocating a fresh `String` per key.
#[inline]
fn write_member_name(wrapper: &mut Value, name: &str) {
    if let Value::String(buffer) = wrapper {
        buffer.clear();
        buffer.push_str(name);
    }
}

impl<F: Json> Validate<F> for PropertyNamesObjectValidator {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        if let Some(object) = instance.as_object() {
            let mut wrapper = Value::String(String::new());
            for (name, _) in object.members() {
                write_member_name(&mut wrapper, name.as_ref());
                if !self.node.is_valid(&&wrapper, ctx) {
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
            let mut wrapper = Value::String(String::new());
            for (name, _) in object.members() {
                write_member_name(&mut wrapper, name.as_ref());
                if let Err(error) = self.node.validate(&&wrapper, location, tracker, ctx) {
                    let schema_path = error.schema_path().clone();
                    return Err(ValidationError::property_names(
                        schema_path.clone(),
                        crate::paths::capture_evaluation_path(tracker, &schema_path),
                        location.into(),
                        instance.to_value(),
                        error.to_owned(),
                    ));
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
            let mut wrapper = Value::String(String::new());
            for (name, _) in object.members() {
                write_member_name(&mut wrapper, name.as_ref());
                for error in self.node.iter_errors(&&wrapper, location, tracker, ctx) {
                    let schema_path = error.schema_path().clone();
                    errors.push(ValidationError::property_names(
                        schema_path.clone(),
                        crate::paths::capture_evaluation_path(tracker, &schema_path),
                        location.into(),
                        instance.to_value(),
                        error.to_owned(),
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
            let mut children = Vec::with_capacity(object.len());
            let mut wrapper = Value::String(String::new());
            for (name, _) in object.members() {
                write_member_name(&mut wrapper, name.as_ref());
                children.push(
                    self.node
                        .evaluate_instance(&&wrapper, location, tracker, ctx),
                );
            }
            EvaluationResult::from_children(children)
        } else {
            EvaluationResult::valid_empty()
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
                instance.to_value(),
            ))
        }
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
