use crate::{
    compiler,
    error::{no_error, ErrorIterator, ValidationError},
    keywords::CompilationResult,
    node::SchemaNode,
    paths::{LazyLocation, Location, RefTracker},
    validator::{Validate, ValidationContext},
    Array, Json, Node, SerdeJson,
};
use serde_json::{Map, Value};

pub(crate) struct AdditionalItemsObjectValidator<F: Json = SerdeJson> {
    node: SchemaNode<F>,
    items_count: usize,
}
impl AdditionalItemsObjectValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        schema: &'a Value,
        items_count: usize,
    ) -> CompilationResult<'a, F> {
        let node = compiler::compile(ctx, ctx.as_resource_ref(schema))?;
        Ok(Box::new(AdditionalItemsObjectValidator {
            node,
            items_count,
        }))
    }
}
impl<F: Json> Validate<F> for AdditionalItemsObjectValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        if let Some(array) = instance.as_array() {
            array
                .elements()
                .skip(self.items_count)
                .all(|item| self.node.is_valid(&item, ctx))
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
        if let Some(array) = instance.as_array() {
            for (idx, item) in array.elements().enumerate().skip(self.items_count) {
                self.node
                    .validate(&item, &location.push(idx), tracker, ctx)?;
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
        if let Some(array) = instance.as_array() {
            let mut errors = Vec::new();
            for (idx, item) in array.elements().enumerate().skip(self.items_count) {
                errors.extend(
                    self.node
                        .iter_errors(&item, &location.push(idx), tracker, ctx),
                );
            }
            ErrorIterator::from_iterator(errors.into_iter())
        } else {
            no_error()
        }
    }
}

pub(crate) struct AdditionalItemsBooleanValidator {
    items_count: usize,
    location: Location,
}
impl AdditionalItemsBooleanValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        items_count: usize,
        location: Location,
    ) -> CompilationResult<'a, F> {
        Ok(Box::new(AdditionalItemsBooleanValidator {
            items_count,
            location,
        }))
    }
}
impl<F: Json> Validate<F> for AdditionalItemsBooleanValidator {
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        if let Some(array) = instance.as_array() {
            if array.len() > self.items_count {
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
        _ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Some(array) = instance.as_array() {
            if array.len() > self.items_count {
                return Err(ValidationError::additional_items(
                    self.location.clone(),
                    crate::paths::capture_evaluation_path(tracker, &self.location),
                    location.into(),
                    instance.to_value(),
                    self.items_count,
                ));
            }
        }
        Ok(())
    }
}

#[inline]
pub(crate) fn compile<'a, F: Json>(
    ctx: &compiler::Context<F>,
    parent: &Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a, F>> {
    if let Some(items) = parent.get("items") {
        match items {
            Value::Array(items) => {
                let kctx = ctx.new_at_location("additionalItems");
                let items_count = items.len();
                match schema {
                    Value::Object(_) => Some(AdditionalItemsObjectValidator::compile(
                        &kctx,
                        schema,
                        items_count,
                    )),
                    Value::Bool(false) => Some(AdditionalItemsBooleanValidator::compile(
                        items_count,
                        kctx.location().clone(),
                    )),
                    _ => None,
                }
            }
            // `additionalItems` describes the elements past an array-form `items` tuple. Any other
            // `items` value leaves no tail, so the keyword is ignored: a schema (object or boolean)
            // covers every element — including `items: false`, where the tail is empty rather than
            // forbidden — and an invalid `items` value is itself ignored. Non-arrays are not this
            // keyword's business at all.
            _ => None,
        }
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use referencing::Draft;
    use serde_json::{json, Value};
    use test_case::test_case;

    fn validator_with(draft: Draft, schema: &Value) -> crate::Validator {
        // Draft 4's metaschema forbids boolean schemas, so meta-validation is off to let one
        // builder cover every draft.
        crate::options()
            .with_draft(draft)
            .without_schema_validation()
            .build(schema)
            .expect("schema compiles")
    }

    // A boolean `items` leaves no tuple tail, so `additionalItems` is ignored and only `items`
    // can reject. The keyword constrains array elements, so it must never touch a non-array.
    // Dispatch is draft-agnostic: 2020-12 goes through the same `Bool` arm as older drafts.
    #[test_case(Draft::Draft4, &json!({"additionalItems": false, "items": false}), &json!(null); "draft4 null")]
    #[test_case(Draft::Draft4, &json!({"additionalItems": false, "items": false}), &json!([]); "draft4 empty array")]
    #[test_case(Draft::Draft4, &json!({"additionalItems": false, "items": false}), &json!("a"); "draft4 string")]
    #[test_case(Draft::Draft4, &json!({"additionalItems": false, "items": true}), &json!(null); "draft4 items true")]
    #[test_case(Draft::Draft6, &json!({"additionalItems": false, "items": false}), &json!(null); "draft6 null")]
    #[test_case(Draft::Draft6, &json!({"additionalItems": false, "items": false}), &json!([]); "draft6 empty array")]
    #[test_case(Draft::Draft6, &json!({"additionalItems": false, "items": false}), &json!("a"); "draft6 string")]
    #[test_case(Draft::Draft6, &json!({"additionalItems": false, "items": true}), &json!(null); "draft6 items true")]
    #[test_case(Draft::Draft7, &json!({"additionalItems": false, "items": false}), &json!(null); "draft7 null")]
    #[test_case(Draft::Draft7, &json!({"additionalItems": false, "items": false}), &json!([]); "draft7 empty array")]
    #[test_case(Draft::Draft7, &json!({"additionalItems": false, "items": false}), &json!("a"); "draft7 string")]
    #[test_case(Draft::Draft7, &json!({"additionalItems": false, "items": true}), &json!(null); "draft7 items true")]
    #[test_case(Draft::Draft201909, &json!({"additionalItems": false, "items": false}), &json!(null); "draft2019 null")]
    #[test_case(Draft::Draft201909, &json!({"additionalItems": false, "items": false}), &json!([]); "draft2019 empty array")]
    #[test_case(Draft::Draft201909, &json!({"additionalItems": false, "items": false}), &json!("a"); "draft2019 string")]
    #[test_case(Draft::Draft201909, &json!({"additionalItems": false, "items": true}), &json!(null); "draft2019 items true")]
    #[test_case(Draft::Draft202012, &json!({"additionalItems": false, "items": false}), &json!(null); "draft2020 null")]
    #[test_case(Draft::Draft202012, &json!({"additionalItems": false, "items": false}), &json!([]); "draft2020 empty array")]
    #[test_case(Draft::Draft202012, &json!({"additionalItems": false, "items": false}), &json!("a"); "draft2020 string")]
    #[test_case(Draft::Draft202012, &json!({"additionalItems": false, "items": true}), &json!(null); "draft2020 items true")]
    fn boolean_items_makes_additional_items_inert(draft: Draft, schema: &Value, instance: &Value) {
        let validator = validator_with(draft, schema);
        tests_util::is_valid_with(&validator, instance);
    }

    // A non-array `items` value leaves no tuple tail either, so `additionalItems` beside it is
    // ignored just like the invalid `items` itself. Only a registry resource can carry such a
    // pair, since top-level builds are meta-validated.
    #[test_case(&json!(42))]
    #[test_case(&json!("items"))]
    #[test_case(&json!(null))]
    fn non_array_items_makes_additional_items_inert(items: &Value) {
        let resource = json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "additionalItems": {"type": "string"},
            "items": items,
        });
        let registry = crate::Registry::new()
            .add("https://example.com/tail", &resource)
            .expect("resource accepted")
            .prepare()
            .expect("registry build failed");
        let validator = crate::options()
            .with_registry(&registry)
            .build(&json!({"$ref": "https://example.com/tail"}))
            .expect("Invalid schema");
        assert!(validator.is_valid(&json!([1, 2])));
        assert!(validator.is_valid(&json!(null)));
    }

    // `items: false` still forbids every element, with or without `additionalItems` beside it.
    #[test_case(&json!({"additionalItems": false, "items": false}))]
    #[test_case(&json!({"additionalItems": true, "items": false}))]
    #[test_case(&json!({"items": false}))]
    fn boolean_items_still_rejects_elements(schema: &Value) {
        let validator = validator_with(Draft::Draft7, schema);
        tests_util::is_not_valid_with(&validator, &json!([1]));
    }

    // Beside a boolean `items` no additionalItems validator exists, so `/items` is the only
    // possible schema path; with array-form `items` the tail error comes from `/additionalItems`.
    #[test_case(&json!({"additionalItems": false, "items": false}), &json!([1]), "/items")]
    #[test_case(&json!({"additionalItems": false, "items": [{}]}), &json!([1, 2]), "/additionalItems")]
    #[test_case(&json!({"additionalItems": {"type": "string"}, "items": [{}]}), &json!([1, 2]), "/additionalItems/type")]
    fn location(schema: &Value, instance: &Value, expected: &str) {
        let validator = crate::options()
            .with_draft(Draft::Draft7)
            .build(schema)
            .expect("Invalid schema");
        let error = validator.validate(instance).expect_err("Should fail");
        assert_eq!(error.schema_path().as_str(), expected);
    }
}
