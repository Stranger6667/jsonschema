use crate::{
    compiler,
    error::ValidationError,
    evaluation::{ChildList, ErrorDescription},
    keywords::{
        helpers::{unit_over, SchemaSite},
        CompilationResult,
    },
    node::SchemaNode,
    paths::{LazyLocation, RefTracker},
    validator::{EvaluationResult, Validate, ValidationContext},
    Json, Node, Object,
};
use serde_json::{Map, Value};
use std::{convert::Infallible, ops::ControlFlow};

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
            return object
                .try_for_each_name_node(&mut buffer, |node| {
                    if self.node.is_valid_child(&node, ctx) {
                        ControlFlow::Continue(())
                    } else {
                        ControlFlow::Break(())
                    }
                })
                .is_continue();
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
            let flow = object.try_for_each_name_node(&mut buffer, |node| {
                match self.node.validate(&node, location, tracker, ctx) {
                    Ok(()) => ControlFlow::Continue(()),
                    Err(error) => ControlFlow::Break(error.to_owned()),
                }
            });
            if let ControlFlow::Break(error) = flow {
                return Err(ValidationError::property_names(
                    location.into(),
                    instance.lazy_value(),
                    error,
                ));
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
        let ControlFlow::Continue(()) =
            object.try_for_each_name_node::<Infallible>(&mut buffer, |node| {
                let mut collected = Vec::new();
                self.node
                    .collect_errors(&node, location, tracker, ctx, &mut collected);
                for error in collected {
                    errors.push(ValidationError::property_names(
                        location.into(),
                        instance.lazy_value(),
                        error.to_owned(),
                    ));
                }
                ControlFlow::Continue(())
            });
    }

    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Some(object) = instance.as_object() {
            let mut children = ChildList::default();
            let mut buffer = F::StringBuffer::default();
            let ControlFlow::Continue(()) =
                object.try_for_each_name_node::<Infallible>(&mut buffer, |node| {
                    let child = self.node.evaluate_instance(&node, location, tracker, ctx);
                    children.push(ctx.arena(), child);
                    ControlFlow::Continue(())
                });
            EvaluationResult::from_children(children)
        } else {
            EvaluationResult::valid_empty()
        }
    }
}

pub(crate) struct PropertyNamesBooleanValidator {
    site: SchemaSite,
}

impl PropertyNamesBooleanValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(ctx: &compiler::Context<F>) -> CompilationResult<'a, F> {
        let location = ctx.location().join("propertyNames");
        Ok(Box::new(PropertyNamesBooleanValidator {
            site: SchemaSite::new(ctx, location),
        }))
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
                self.site.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.site.location),
                location.into(),
                instance.lazy_value(),
            ))
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
        // The false schema fails each name, at the object's location.
        let position = ctx.instance_position(location);
        let mut children = ChildList::default();
        let mut buffer = F::StringBuffer::default();
        let ControlFlow::Continue(()) =
            object.try_for_each_name_node::<Infallible>(&mut buffer, |name| {
                let error =
                    ErrorDescription::from_validation_error(&ValidationError::false_schema(
                        self.site.location.clone(),
                        crate::paths::capture_evaluation_path(tracker, &self.site.location),
                        location.into(),
                        name.lazy_value(),
                    ));
                let node = unit_over(
                    &self.site,
                    vec![error],
                    None,
                    ChildList::default(),
                    position,
                    tracker,
                    ctx,
                );
                children.push(ctx.arena(), node);
                ControlFlow::Continue(())
            });
        EvaluationResult::from_children(children)
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
    use crate::{error::ValidationErrorKind, tests_util, ValidationError};
    use serde_json::{json, Value};
    use test_case::test_case;

    // The `propertyNames` error and the error it wraps, outermost first.
    fn absolute_locations(error: &ValidationError<'_>) -> Vec<Option<String>> {
        let mut locations = vec![error.absolute_keyword_location().map(ToString::to_string)];
        if let ValidationErrorKind::PropertyNames { error } = error.kind() {
            locations.extend(absolute_locations(error));
        }
        locations
    }

    // A name's error is located at the keyword that rejected it, in that keyword's resource.
    #[test_case(
        &json!({"$id": "http://example.com/root", "propertyNames": {"maxLength": 0}}),
        "http://example.com/root#/propertyNames/maxLength" ; "in place"
    )]
    #[test_case(
        &json!({"$id": "http://example.com/root", "$defs": {"s": {"maxLength": 0}}, "propertyNames": {"$ref": "#/$defs/s"}}),
        "http://example.com/root#/$defs/s/maxLength" ; "ref in the same resource"
    )]
    #[test_case(
        &json!({"$id": "http://example.com/root", "$defs": {"other": {"$id": "http://example.com/other", "maxLength": 0}}, "propertyNames": {"$ref": "http://example.com/other"}}),
        "http://example.com/other#/maxLength" ; "ref into another resource"
    )]
    fn absolute_keyword_location(schema: &Value, expected: &str) {
        let validator = crate::validator_for(schema).expect("valid schema");
        let instance = json!({"a": 1});
        let expected = vec![Some(expected.to_string()), Some(expected.to_string())];
        let error = validator.validate(&instance).expect_err("invalid");
        assert_eq!(absolute_locations(&error), expected);
        let locations: Vec<_> = validator
            .iter_errors(&instance)
            .map(|error| absolute_locations(&error))
            .collect();
        assert_eq!(locations, [expected]);
    }

    // A name's error is reported along the path that reached it, `$ref`s included.
    #[test_case(
        &json!({"$defs": {"s": {"pattern": "^a"}}, "propertyNames": {"$ref": "#/$defs/s"}}),
        "/propertyNames/$ref/pattern" ; "ref below propertyNames"
    )]
    #[test_case(
        &json!({"$defs": {"o": {"propertyNames": {"pattern": "^a"}}}, "$ref": "#/$defs/o"}),
        "/$ref/propertyNames/pattern" ; "propertyNames below ref"
    )]
    #[test_case(
        &json!({"propertyNames": {"pattern": "^a"}}),
        "/propertyNames/pattern" ; "no ref"
    )]
    fn evaluation_path(schema: &Value, expected: &str) {
        let validator = crate::validator_for(schema).expect("valid schema");
        let instance = json!({"bx": 1});
        let error = validator.validate(&instance).expect_err("invalid");
        assert_eq!(error.evaluation_path().as_str(), expected);
        let paths: Vec<_> = validator
            .iter_errors(&instance)
            .map(|error| error.evaluation_path().as_str().to_string())
            .collect();
        assert_eq!(paths, [expected]);
    }

    // Each name gets a unit of its own at the object's location; `false` fails each.
    #[test_case(
        &json!({"propertyNames": {"maxLength": 1}}),
        &json!({"a": 1, "bb": 2}),
        &[
            " @ FAIL",
            "  /propertyNames @ FAIL",
            "    /propertyNames @ ok",
            "      /propertyNames/maxLength @ ok",
            "    /propertyNames @ FAIL",
            "      /propertyNames/maxLength @ FAIL err=maxLength",
        ] ; "schema"
    )]
    #[test_case(
        &json!({"propertyNames": false}),
        &json!({"a": 1, "b": 2}),
        &[
            " @ FAIL",
            "  /propertyNames @ FAIL",
            "    /propertyNames @ FAIL err=falseSchema",
            "    /propertyNames @ FAIL err=falseSchema",
        ] ; "false"
    )]
    #[test_case(
        &json!({"properties": {"a": {"propertyNames": false}}}),
        &json!({"a": {"b": 1}}),
        &[
            " @ FAIL",
            "  /properties @ FAIL dropped=[\"a\"]",
            "    /properties/a @/a FAIL",
            "      /properties/a/propertyNames @/a FAIL",
            "        /properties/a/propertyNames @/a FAIL err=falseSchema",
        ] ; "false below the root"
    )]
    #[test_case(&json!({"propertyNames": false}), &json!({}), &[" @ ok", "  /propertyNames @ ok"] ; "false on an empty object")]
    fn evaluate_reports_each_name(schema: &Value, instance: &Value, expected: &[&str]) {
        assert_eq!(tests_util::evaluation_outline(schema, instance), expected);
    }

    // Each key is validated on its own, not concatenated with the ones before it
    #[test_case(&json!({"propertyNames": {"maxLength": 3}}), &json!({"abc": 1, "de": 1}))]
    #[test_case(&json!({"propertyNames": {"maxLength": 3}}), &json!({"a": 1, "bc": 1, "def": 1}))]
    #[test_case(&json!({"propertyNames": {"maxLength": 2}, "additionalProperties": {"propertyNames": {"maxLength": 6}}}), &json!({"a": {"abcdef": 1}, "b": {"cd": 1}}); "across nested objects")]
    fn keys_validate_independently(schema: &Value, instance: &Value) {
        tests_util::is_valid(schema, instance);
    }

    // Names from one validation do not carry over into the next
    #[test]
    fn keys_validate_independently_across_validations() {
        tests_util::is_valid(
            &json!({"propertyNames": {"maxLength": 6}}),
            &json!({"abcdef": 1}),
        );
        tests_util::is_valid(
            &json!({"propertyNames": {"maxLength": 2}}),
            &json!({"ab": 1}),
        );
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
