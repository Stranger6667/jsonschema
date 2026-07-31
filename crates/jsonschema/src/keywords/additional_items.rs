use crate::{
    compiler,
    error::{no_error, ErrorIterator, ValidationError},
    keywords::CompilationResult,
    node::SchemaNode,
    paths::{LazyLocation, Location, RefTracker},
    types::{JsonType, JsonTypeSet},
    validator::{Validate, ValidationContext},
    Array, Json, Node, SerdeJson,
};
use serde_json::{Map, Value};
use std::borrow::Cow;

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
            // `additionalItems` describes the elements past an array-form `items` tuple. Any other
            // `items` is a schema covering every element, so there is no tail and the spec says to
            // ignore the keyword — including `items: false`, where the tail is empty rather than
            // forbidden, and non-arrays are not this keyword's business at all.
            Value::Object(_) | Value::Bool(_) => None,
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
            _ => {
                let location = ctx.location().join("additionalItems");
                Some(Err(ValidationError::multiple_type_error(
                    location.clone(),
                    location,
                    Location::new(),
                    Cow::Borrowed(schema),
                    JsonTypeSet::from(JsonType::Object)
                        .insert(JsonType::Array)
                        .insert(JsonType::Boolean),
                )))
            }
        }
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use referencing::Draft;
    use serde_json::{json, Value};
    use test_case::test_case;

    // A boolean `items` leaves no tuple tail, so `additionalItems` is ignored and only `items`
    // can reject. The keyword constrains array elements, so it must never touch a non-array.
    #[test_case(&json!({"additionalItems": false, "items": false}), &json!(null))]
    #[test_case(&json!({"additionalItems": false, "items": false}), &json!([]))]
    #[test_case(&json!({"additionalItems": false, "items": false}), &json!("a"))]
    #[test_case(&json!({"additionalItems": true, "items": false}), &json!(null))]
    #[test_case(&json!({"additionalItems": {"type": "string"}, "items": false}), &json!(null))]
    #[test_case(&json!({"additionalItems": false, "items": true}), &json!(null))]
    fn boolean_items_makes_additional_items_inert(schema: &Value, instance: &Value) {
        // 2020-12 dropped the keyword entirely, so it is inert there for a second reason.
        for draft in [
            Draft::Draft6,
            Draft::Draft7,
            Draft::Draft201909,
            Draft::Draft202012,
        ] {
            let validator = crate::options()
                .with_draft(draft)
                .build(schema)
                .expect("Invalid schema");
            assert!(
                validator.is_valid(instance),
                "{draft:?} rejected {instance}"
            );
        }
    }

    // `items: false` still forbids every element, with or without `additionalItems` beside it.
    #[test_case(&json!({"additionalItems": false, "items": false}))]
    #[test_case(&json!({"additionalItems": true, "items": false}))]
    #[test_case(&json!({"items": false}))]
    fn boolean_items_still_rejects_elements(schema: &Value) {
        let validator = crate::options()
            .with_draft(Draft::Draft7)
            .build(schema)
            .expect("Invalid schema");
        assert!(!validator.is_valid(&json!([1])));
    }

    // When items: false and additionalItems: false, items runs first (lower priority)
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
