use std::borrow::Cow;

use crate::{
    compiler,
    error::{no_error, ErrorIterator, ValidationError},
    keywords::CompilationResult,
    node::SchemaNode,
    paths::{LazyLocation, Location, RefTracker},
    validator::{EvaluationResult, Validate, ValidationContext},
    Json, JsonNode, JsonObjectAccess, SerdeJson,
};
use serde_json::{Map, Value};

pub(crate) struct PropertyNamesObjectValidator {
    node: SchemaNode,
}

impl PropertyNamesObjectValidator {
    #[inline]
    pub(crate) fn compile<'a>(ctx: &compiler::Context, schema: &'a Value) -> CompilationResult<'a> {
        let ctx = ctx.new_at_location("propertyNames");
        Ok(Box::new(PropertyNamesObjectValidator {
            node: compiler::compile(&ctx, ctx.as_resource_ref(schema))?,
        }))
    }
}

// String-keyed subschema; stays serde_json-only until the node trait can wrap host strings.
impl Validate for PropertyNamesObjectValidator {
    fn is_valid(
        &self,
        instance: &<SerdeJson as Json>::Node<'_>,
        ctx: &mut ValidationContext,
    ) -> bool {
        if let Value::Object(item) = &**instance {
            item.keys().all(move |key| {
                let wrapper = Value::String(key.clone());
                self.node.is_valid(&&wrapper, ctx)
            })
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &<SerdeJson as Json>::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Object(item) = &**instance {
            for key in item.keys() {
                let wrapper = Value::String(key.clone());
                match self.node.validate(&&wrapper, location, tracker, ctx) {
                    Ok(()) => {}
                    Err(error) => {
                        let schema_path = error.schema_path().clone();
                        return Err(ValidationError::property_names(
                            schema_path.clone(),
                            crate::paths::capture_evaluation_path(tracker, &schema_path),
                            location.into(),
                            Cow::Borrowed(*instance),
                            error.to_owned(),
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    fn iter_errors<'i>(
        &self,
        instance: &<SerdeJson as Json>::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Value::Object(item) = &**instance {
            let mut errors = Vec::new();
            for key in item.keys() {
                let wrapper = Value::String(key.clone());
                for error in self.node.iter_errors(&&wrapper, location, tracker, ctx) {
                    let schema_path = error.schema_path().clone();
                    errors.push(ValidationError::property_names(
                        schema_path.clone(),
                        crate::paths::capture_evaluation_path(tracker, &schema_path),
                        location.into(),
                        Cow::Borrowed(*instance),
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
        instance: &<SerdeJson as Json>::Node<'_>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Value::Object(item) = &**instance {
            let mut children = Vec::with_capacity(item.len());
            for key in item.keys() {
                let wrapper = Value::String(key.clone());
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
    pub(crate) fn compile<'a>(ctx: &compiler::Context) -> CompilationResult<'a> {
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
pub(crate) fn compile<'a>(
    ctx: &compiler::Context,
    _: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>> {
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

    #[test_case(&json!({"propertyNames": false}), &json!({"foo": 1}), "/propertyNames")]
    #[test_case(&json!({"propertyNames": {"minLength": 2}}), &json!({"f": 1}), "/propertyNames/minLength")]
    fn location(schema: &Value, instance: &Value, expected: &str) {
        tests_util::assert_schema_location(schema, instance, expected);
    }
}
