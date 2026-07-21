use crate::{
    compiler,
    error::{no_error, ErrorIterator},
    evaluation::{format_schema_location, Annotations, ErrorDescription, EvaluationNode},
    keywords::{BoxedValidator, CompilationResult},
    node::SchemaNode,
    paths::{LazyLocation, Location, RefTracker},
    types::JsonType,
    validator::{EvaluationResult, Validate, ValidationContext},
    Draft, Json, JsonArrayAccess, JsonNode, SerdeJson, ValidationError,
};
use referencing::{Uri, Vocabulary};
use serde_json::{Map, Value};
use std::sync::Arc;

pub(crate) struct ItemsArrayValidator<F: Json = SerdeJson> {
    items: Vec<SchemaNode<F>>,
}
impl ItemsArrayValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        schemas: &'a [Value],
    ) -> CompilationResult<'a, F> {
        let kctx = ctx.new_at_location("items");
        let mut items = Vec::with_capacity(schemas.len());
        for (idx, item) in schemas.iter().enumerate() {
            let ictx = kctx.new_at_location(idx);
            let validators = compiler::compile(&ictx, ictx.as_resource_ref(item))?;
            items.push(validators);
        }
        Ok(Box::new(ItemsArrayValidator { items }))
    }
}
impl<F: Json> Validate<F> for ItemsArrayValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        if let Some(array) = instance.as_array() {
            for (item, node) in array.elements().zip(self.items.iter()) {
                if !node.is_valid(&item, ctx) {
                    return false;
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
        if let Some(array) = instance.as_array() {
            for (idx, (item, node)) in array.elements().zip(self.items.iter()).enumerate() {
                node.validate(&item, &location.push(idx), tracker, ctx)?;
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
            for (idx, (item, node)) in array.elements().zip(self.items.iter()).enumerate() {
                errors.extend(node.iter_errors(&item, &location.push(idx), tracker, ctx));
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
        if let Some(array) = instance.as_array() {
            let mut children = Vec::with_capacity(self.items.len().min(array.len()));
            for (idx, (item, node)) in array.elements().zip(self.items.iter()).enumerate() {
                children.push(node.evaluate_instance(&item, &location.push(idx), tracker, ctx));
            }
            EvaluationResult::from_children(children)
        } else {
            EvaluationResult::valid_empty()
        }
    }
}

pub(crate) struct ItemsObjectValidator<F: Json = SerdeJson> {
    node: SchemaNode<F>,
}

impl ItemsObjectValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        schema: &'a Value,
    ) -> CompilationResult<'a, F> {
        let ctx = ctx.new_at_location("items");
        let node = compiler::compile(&ctx, ctx.as_resource_ref(schema))?;
        Ok(Box::new(ItemsObjectValidator { node }))
    }
}
impl<F: Json> Validate<F> for ItemsObjectValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        if let Some(array) = instance.as_array() {
            array.elements().all(|item| self.node.is_valid(&item, ctx))
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
            for (idx, item) in array.elements().enumerate() {
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
            for (idx, item) in array.elements().enumerate() {
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

    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Some(array) = instance.as_array() {
            let mut children = Vec::with_capacity(array.len());
            for (idx, item) in array.elements().enumerate() {
                children.push(self.node.evaluate_instance(
                    &item,
                    &location.push(idx),
                    tracker,
                    ctx,
                ));
            }
            let schema_was_applied = array.len() != 0;
            let mut result = EvaluationResult::from_children(children);
            result.annotate(Annotations::new(serde_json::json!(schema_was_applied)));
            result
        } else {
            EvaluationResult::valid_empty()
        }
    }
}

pub(crate) struct ItemsObjectSkipPrefixValidator<F: Json = SerdeJson> {
    node: SchemaNode<F>,
    skip_prefix: usize,
}

impl ItemsObjectSkipPrefixValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        schema: &'a Value,
        skip_prefix: usize,
        ctx: &compiler::Context<F>,
    ) -> CompilationResult<'a, F> {
        let ctx = ctx.new_at_location("items");
        let node = compiler::compile(&ctx, ctx.as_resource_ref(schema))?;
        Ok(Box::new(ItemsObjectSkipPrefixValidator {
            node,
            skip_prefix,
        }))
    }
}

impl<F: Json> Validate<F> for ItemsObjectSkipPrefixValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        if let Some(array) = instance.as_array() {
            array
                .elements()
                .skip(self.skip_prefix)
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
            for (idx, item) in array.elements().skip(self.skip_prefix).enumerate() {
                self.node
                    .validate(&item, &location.push(idx + self.skip_prefix), tracker, ctx)?;
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
            for (idx, item) in array.elements().skip(self.skip_prefix).enumerate() {
                errors.extend(self.node.iter_errors(
                    &item,
                    &location.push(idx + self.skip_prefix),
                    tracker,
                    ctx,
                ));
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
        if let Some(array) = instance.as_array() {
            let mut children = Vec::with_capacity(array.len().saturating_sub(self.skip_prefix));
            for (idx, item) in array.elements().enumerate().skip(self.skip_prefix) {
                children.push(self.node.evaluate_instance(
                    &item,
                    &location.push(idx),
                    tracker,
                    ctx,
                ));
            }
            let schema_was_applied = array.len() > self.skip_prefix;
            let mut result = EvaluationResult::from_children(children);
            result.annotate(Annotations::new(serde_json::json!(schema_was_applied)));
            result
        } else {
            EvaluationResult::valid_empty()
        }
    }
}

// Specialized validators for common simple item schemas.
// These avoid dynamic dispatch overhead by inlining the type check.

pub(crate) struct ItemsNumberTypeValidator {
    location: Location,
}

impl ItemsNumberTypeValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(location: Location) -> CompilationResult<'a, F> {
        Ok(Box::new(ItemsNumberTypeValidator { location }))
    }
}

impl<F: Json> Validate<F> for ItemsNumberTypeValidator {
    #[inline]
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        if let Some(array) = instance.as_array() {
            array.elements().all(|item| item.is_number())
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Some(array) = instance.as_array() {
            for (idx, item) in array.elements().enumerate() {
                if !item.is_number() {
                    return Err(ValidationError::single_type_error(
                        self.location.clone(),
                        crate::paths::capture_evaluation_path(tracker, &self.location),
                        (&location.push(idx)).into(),
                        item.to_value(),
                        JsonType::Number,
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
        _ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Some(array) = instance.as_array() {
            let errors: Vec<_> = array
                .elements()
                .enumerate()
                .filter(|(_, item)| !item.is_number())
                .map(|(idx, item)| {
                    ValidationError::single_type_error(
                        self.location.clone(),
                        crate::paths::capture_evaluation_path(tracker, &self.location),
                        (&location.push(idx)).into(),
                        item.to_value(),
                        JsonType::Number,
                    )
                })
                .collect();
            ErrorIterator::from_iterator(errors.into_iter())
        } else {
            no_error()
        }
    }

    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        _location: &LazyLocation,
        _tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Some(array) = instance.as_array() {
            let errors: Vec<_> = array
                .elements()
                .enumerate()
                .filter(|(_, item)| !item.is_number())
                .map(|(idx, item)| {
                    let item = item.to_value();
                    ErrorDescription::new(
                        "type",
                        format!(r#"{item} at index {idx} is not of type "number""#),
                    )
                })
                .collect();
            let schema_was_applied = array.len() != 0;
            if errors.is_empty() {
                let mut result = EvaluationResult::valid_empty();
                result.annotate(Annotations::new(serde_json::json!(schema_was_applied)));
                result
            } else {
                let mut result = EvaluationResult::invalid_empty(errors);
                result.annotate(Annotations::new(serde_json::json!(schema_was_applied)));
                result
            }
        } else {
            EvaluationResult::valid_empty()
        }
    }
}

pub(crate) struct ItemsStringTypeValidator {
    location: Location,
}

impl ItemsStringTypeValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(location: Location) -> CompilationResult<'a, F> {
        Ok(Box::new(ItemsStringTypeValidator { location }))
    }
}

impl<F: Json> Validate<F> for ItemsStringTypeValidator {
    #[inline]
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        if let Some(array) = instance.as_array() {
            array.elements().all(|item| item.is_string())
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Some(array) = instance.as_array() {
            for (idx, item) in array.elements().enumerate() {
                if !item.is_string() {
                    return Err(ValidationError::single_type_error(
                        self.location.clone(),
                        crate::paths::capture_evaluation_path(tracker, &self.location),
                        (&location.push(idx)).into(),
                        item.to_value(),
                        JsonType::String,
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
        _ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Some(array) = instance.as_array() {
            let errors: Vec<_> = array
                .elements()
                .enumerate()
                .filter(|(_, item)| !item.is_string())
                .map(|(idx, item)| {
                    ValidationError::single_type_error(
                        self.location.clone(),
                        crate::paths::capture_evaluation_path(tracker, &self.location),
                        (&location.push(idx)).into(),
                        item.to_value(),
                        JsonType::String,
                    )
                })
                .collect();
            ErrorIterator::from_iterator(errors.into_iter())
        } else {
            no_error()
        }
    }

    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        _location: &LazyLocation,
        _tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Some(array) = instance.as_array() {
            let errors: Vec<_> = array
                .elements()
                .enumerate()
                .filter(|(_, item)| !item.is_string())
                .map(|(idx, item)| {
                    let item = item.to_value();
                    ErrorDescription::new(
                        "type",
                        format!(r#"{item} at index {idx} is not of type "string""#),
                    )
                })
                .collect();
            let schema_was_applied = array.len() != 0;
            if errors.is_empty() {
                let mut result = EvaluationResult::valid_empty();
                result.annotate(Annotations::new(serde_json::json!(schema_was_applied)));
                result
            } else {
                let mut result = EvaluationResult::invalid_empty(errors);
                result.annotate(Annotations::new(serde_json::json!(schema_was_applied)));
                result
            }
        } else {
            EvaluationResult::valid_empty()
        }
    }
}

pub(crate) struct ItemsIntegerTypeValidator {
    location: Location,
}

impl ItemsIntegerTypeValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(location: Location) -> CompilationResult<'a, F> {
        Ok(Box::new(ItemsIntegerTypeValidator { location }))
    }
}

impl<F: Json> Validate<F> for ItemsIntegerTypeValidator {
    #[inline]
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        if let Some(array) = instance.as_array() {
            array.elements().all(|item| {
                item.as_number()
                    .is_some_and(|n| super::type_::is_integer(&n))
            })
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Some(array) = instance.as_array() {
            for (idx, item) in array.elements().enumerate() {
                let valid = item
                    .as_number()
                    .is_some_and(|n| super::type_::is_integer(&n));
                if !valid {
                    return Err(ValidationError::single_type_error(
                        self.location.clone(),
                        crate::paths::capture_evaluation_path(tracker, &self.location),
                        (&location.push(idx)).into(),
                        item.to_value(),
                        JsonType::Integer,
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
        _ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Some(array) = instance.as_array() {
            let errors: Vec<_> = array
                .elements()
                .enumerate()
                .filter(|(_, item)| {
                    !item
                        .as_number()
                        .is_some_and(|n| super::type_::is_integer(&n))
                })
                .map(|(idx, item)| {
                    ValidationError::single_type_error(
                        self.location.clone(),
                        crate::paths::capture_evaluation_path(tracker, &self.location),
                        (&location.push(idx)).into(),
                        item.to_value(),
                        JsonType::Integer,
                    )
                })
                .collect();
            ErrorIterator::from_iterator(errors.into_iter())
        } else {
            no_error()
        }
    }

    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        _location: &LazyLocation,
        _tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Some(array) = instance.as_array() {
            let errors: Vec<_> = array
                .elements()
                .enumerate()
                .filter(|(_, item)| {
                    !item
                        .as_number()
                        .is_some_and(|n| super::type_::is_integer(&n))
                })
                .map(|(idx, item)| {
                    let item = item.to_value();
                    ErrorDescription::new(
                        "type",
                        format!(r#"{item} at index {idx} is not of type "integer""#),
                    )
                })
                .collect();
            let schema_was_applied = array.len() != 0;
            if errors.is_empty() {
                let mut result = EvaluationResult::valid_empty();
                result.annotate(Annotations::new(serde_json::json!(schema_was_applied)));
                result
            } else {
                let mut result = EvaluationResult::invalid_empty(errors);
                result.annotate(Annotations::new(serde_json::json!(schema_was_applied)));
                result
            }
        } else {
            EvaluationResult::valid_empty()
        }
    }
}

// Draft 4 has stricter integer semantics: numbers with decimal points are NOT integers
pub(crate) struct ItemsIntegerTypeValidatorDraft4 {
    location: Location,
}

impl ItemsIntegerTypeValidatorDraft4 {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(location: Location) -> CompilationResult<'a, F> {
        Ok(Box::new(ItemsIntegerTypeValidatorDraft4 { location }))
    }
}

impl<F: Json> Validate<F> for ItemsIntegerTypeValidatorDraft4 {
    #[inline]
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        if let Some(array) = instance.as_array() {
            array.elements().all(|item| {
                item.as_number()
                    .is_some_and(|n| super::legacy::type_draft_4::is_integer(&n))
            })
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Some(array) = instance.as_array() {
            for (idx, item) in array.elements().enumerate() {
                let valid = item
                    .as_number()
                    .is_some_and(|n| super::legacy::type_draft_4::is_integer(&n));
                if !valid {
                    return Err(ValidationError::single_type_error(
                        self.location.clone(),
                        crate::paths::capture_evaluation_path(tracker, &self.location),
                        (&location.push(idx)).into(),
                        item.to_value(),
                        JsonType::Integer,
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
        _ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Some(array) = instance.as_array() {
            let errors: Vec<_> = array
                .elements()
                .enumerate()
                .filter(|(_, item)| {
                    !item
                        .as_number()
                        .is_some_and(|n| super::legacy::type_draft_4::is_integer(&n))
                })
                .map(|(idx, item)| {
                    ValidationError::single_type_error(
                        self.location.clone(),
                        crate::paths::capture_evaluation_path(tracker, &self.location),
                        (&location.push(idx)).into(),
                        item.to_value(),
                        JsonType::Integer,
                    )
                })
                .collect();
            ErrorIterator::from_iterator(errors.into_iter())
        } else {
            no_error()
        }
    }

    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        _location: &LazyLocation,
        _tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Some(array) = instance.as_array() {
            let errors: Vec<_> = array
                .elements()
                .enumerate()
                .filter(|(_, item)| {
                    !item
                        .as_number()
                        .is_some_and(|n| super::legacy::type_draft_4::is_integer(&n))
                })
                .map(|(idx, item)| {
                    let item = item.to_value();
                    ErrorDescription::new(
                        "type",
                        format!(r#"{item} at index {idx} is not of type "integer""#),
                    )
                })
                .collect();
            let schema_was_applied = array.len() != 0;
            if errors.is_empty() {
                let mut result = EvaluationResult::valid_empty();
                result.annotate(Annotations::new(serde_json::json!(schema_was_applied)));
                result
            } else {
                let mut result = EvaluationResult::invalid_empty(errors);
                result.annotate(Annotations::new(serde_json::json!(schema_was_applied)));
                result
            }
        } else {
            EvaluationResult::valid_empty()
        }
    }
}

pub(crate) struct ItemsBooleanTypeValidator {
    location: Location,
}

impl ItemsBooleanTypeValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(location: Location) -> CompilationResult<'a, F> {
        Ok(Box::new(ItemsBooleanTypeValidator { location }))
    }
}

impl<F: Json> Validate<F> for ItemsBooleanTypeValidator {
    #[inline]
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        if let Some(array) = instance.as_array() {
            array.elements().all(|item| item.as_boolean().is_some())
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Some(array) = instance.as_array() {
            for (idx, item) in array.elements().enumerate() {
                if item.as_boolean().is_none() {
                    return Err(ValidationError::single_type_error(
                        self.location.clone(),
                        crate::paths::capture_evaluation_path(tracker, &self.location),
                        (&location.push(idx)).into(),
                        item.to_value(),
                        JsonType::Boolean,
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
        _ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Some(array) = instance.as_array() {
            let errors: Vec<_> = array
                .elements()
                .enumerate()
                .filter(|(_, item)| item.as_boolean().is_none())
                .map(|(idx, item)| {
                    ValidationError::single_type_error(
                        self.location.clone(),
                        crate::paths::capture_evaluation_path(tracker, &self.location),
                        (&location.push(idx)).into(),
                        item.to_value(),
                        JsonType::Boolean,
                    )
                })
                .collect();
            ErrorIterator::from_iterator(errors.into_iter())
        } else {
            no_error()
        }
    }

    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        _location: &LazyLocation,
        _tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Some(array) = instance.as_array() {
            let errors: Vec<_> = array
                .elements()
                .enumerate()
                .filter(|(_, item)| item.as_boolean().is_none())
                .map(|(idx, item)| {
                    let item = item.to_value();
                    ErrorDescription::new(
                        "type",
                        format!(r#"{item} at index {idx} is not of type "boolean""#),
                    )
                })
                .collect();
            let schema_was_applied = array.len() != 0;
            if errors.is_empty() {
                let mut result = EvaluationResult::valid_empty();
                result.annotate(Annotations::new(serde_json::json!(schema_was_applied)));
                result
            } else {
                let mut result = EvaluationResult::invalid_empty(errors);
                result.annotate(Annotations::new(serde_json::json!(schema_was_applied)));
                result
            }
        } else {
            EvaluationResult::valid_empty()
        }
    }
}

struct CountConstraint {
    limit: u64,
    location: Location,
    absolute_location: Option<Arc<Uri<String>>>,
}

/// Element validation for the fused validator. Single-type variants keep the specialized `items`
/// validator so `is_valid` avoids `SchemaNode` dispatch on the per-element hot path.
enum FusedItems<F: Json> {
    Number(BoxedValidator<F>),
    String(BoxedValidator<F>),
    Boolean(BoxedValidator<F>),
    IntegerDraft4(BoxedValidator<F>),
    IntegerDraft7(BoxedValidator<F>),
    Generic(SchemaNode<F>),
}

impl<F: Json> FusedItems<F> {
    fn compile<'a>(
        ctx: &compiler::Context<F>,
        items: &'a Value,
    ) -> Result<Self, ValidationError<'a>> {
        if let Some(type_name) = get_simple_type_schema(items) {
            let location = ctx.location().join("items").join("type");
            match type_name {
                "number" => {
                    return Ok(FusedItems::Number(ItemsNumberTypeValidator::compile(
                        location,
                    )?))
                }
                "string" => {
                    return Ok(FusedItems::String(ItemsStringTypeValidator::compile(
                        location,
                    )?))
                }
                "boolean" => {
                    return Ok(FusedItems::Boolean(ItemsBooleanTypeValidator::compile(
                        location,
                    )?))
                }
                "integer" => {
                    return Ok(if ctx.draft() == Draft::Draft4 {
                        FusedItems::IntegerDraft4(ItemsIntegerTypeValidatorDraft4::compile(
                            location,
                        )?)
                    } else {
                        FusedItems::IntegerDraft7(ItemsIntegerTypeValidator::compile(location)?)
                    });
                }
                _ => {}
            }
        }
        let ctx = ctx.new_at_location("items");
        Ok(FusedItems::Generic(compiler::compile(
            &ctx,
            ctx.as_resource_ref(items),
        )?))
    }
}

/// Fused `type: "array"` + optional `minItems`/`maxItems` + schema-form `items`: one `as_array`,
/// one length check, and one element pass instead of three validators re-reading the same node.
pub(crate) struct ArrayShapeValidator<F: Json = SerdeJson> {
    items: FusedItems<F>,
    min_items: Option<CountConstraint>,
    max_items: Option<CountConstraint>,
    type_location: Location,
    type_absolute_location: Option<Arc<Uri<String>>>,
}

impl ArrayShapeValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        parent: &'a Map<String, Value>,
        items: &'a Value,
    ) -> CompilationResult<'a, F> {
        let items = FusedItems::compile(ctx, items)?;
        let type_location = ctx.location().join("type");
        let type_absolute_location = ctx.absolute_location(&type_location);
        let constraint = |key: &str| -> Option<CountConstraint> {
            let limit = accepts_item_count(ctx, parent.get(key)?)?;
            let location = ctx.location().join(key);
            let absolute_location = ctx.absolute_location(&location);
            Some(CountConstraint {
                limit,
                location,
                absolute_location,
            })
        };
        Ok(Box::new(ArrayShapeValidator {
            items,
            min_items: constraint("minItems"),
            max_items: constraint("maxItems"),
            type_location,
            type_absolute_location,
        }))
    }
}

impl<F: Json> ArrayShapeValidator<F> {
    fn type_error<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
    ) -> ValidationError<'i> {
        ValidationError::single_type_error(
            self.type_location.clone(),
            crate::paths::capture_evaluation_path(tracker, &self.type_location),
            location.into(),
            instance.to_value(),
            JsonType::Array,
        )
    }
}

fn min_items_error<'i, F: Json>(
    constraint: &CountConstraint,
    instance: &F::Node<'i>,
    location: &LazyLocation,
    tracker: Option<&RefTracker>,
) -> ValidationError<'i> {
    ValidationError::min_items(
        constraint.location.clone(),
        crate::paths::capture_evaluation_path(tracker, &constraint.location),
        location.into(),
        instance.to_value(),
        constraint.limit,
    )
}

fn max_items_error<'i, F: Json>(
    constraint: &CountConstraint,
    instance: &F::Node<'i>,
    location: &LazyLocation,
    tracker: Option<&RefTracker>,
) -> ValidationError<'i> {
    ValidationError::max_items(
        constraint.location.clone(),
        crate::paths::capture_evaluation_path(tracker, &constraint.location),
        location.into(),
        instance.to_value(),
        constraint.limit,
    )
}

/// Wraps an absorbed keyword's failure as a child node at that keyword's own schema location,
/// so structured output keeps the correct `schemaLocation`.
fn absorbed_error_node(
    location: &LazyLocation,
    tracker: Option<&RefTracker>,
    keyword_location: &Location,
    absolute_location: Option<&Arc<Uri<String>>>,
    error: ErrorDescription,
) -> EvaluationNode {
    EvaluationNode::invalid(
        crate::paths::evaluation_path(tracker, keyword_location),
        absolute_location.cloned(),
        format_schema_location(keyword_location, absolute_location),
        location.into(),
        None,
        vec![error],
        Vec::new(),
    )
}

impl<F: Json> Validate<F> for ArrayShapeValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        let Some(array) = instance.as_array() else {
            return false;
        };
        let count = array.len() as u64;
        if let Some(constraint) = &self.min_items {
            if count < constraint.limit {
                return false;
            }
        }
        if let Some(constraint) = &self.max_items {
            if count > constraint.limit {
                return false;
            }
        }
        match &self.items {
            FusedItems::Number(_) => array.elements().all(|item| item.is_number()),
            FusedItems::String(_) => array.elements().all(|item| item.is_string()),
            FusedItems::Boolean(_) => array.elements().all(|item| item.as_boolean().is_some()),
            FusedItems::IntegerDraft7(_) => array.elements().all(|item| {
                item.as_number()
                    .is_some_and(|n| super::type_::is_integer(&n))
            }),
            FusedItems::IntegerDraft4(_) => array.elements().all(|item| {
                item.as_number()
                    .is_some_and(|n| super::legacy::type_draft_4::is_integer(&n))
            }),
            FusedItems::Generic(node) => array.elements().all(|item| node.is_valid(&item, ctx)),
        }
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        let Some(array) = instance.as_array() else {
            return Err(self.type_error(instance, location, tracker));
        };
        let count = array.len() as u64;
        if let Some(constraint) = &self.min_items {
            if count < constraint.limit {
                return Err(min_items_error::<F>(
                    constraint, instance, location, tracker,
                ));
            }
        }
        if let Some(constraint) = &self.max_items {
            if count > constraint.limit {
                return Err(max_items_error::<F>(
                    constraint, instance, location, tracker,
                ));
            }
        }
        match &self.items {
            FusedItems::Generic(node) => {
                for (idx, item) in array.elements().enumerate() {
                    node.validate(&item, &location.push(idx), tracker, ctx)?;
                }
            }
            FusedItems::Number(cold)
            | FusedItems::String(cold)
            | FusedItems::Boolean(cold)
            | FusedItems::IntegerDraft4(cold)
            | FusedItems::IntegerDraft7(cold) => {
                cold.validate(instance, location, tracker, ctx)?;
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
        let Some(array) = instance.as_array() else {
            return ErrorIterator::from_iterator(std::iter::once(
                self.type_error(instance, location, tracker),
            ));
        };
        let mut errors = Vec::new();
        let count = array.len() as u64;
        if let Some(constraint) = &self.min_items {
            if count < constraint.limit {
                errors.push(min_items_error::<F>(
                    constraint, instance, location, tracker,
                ));
            }
        }
        if let Some(constraint) = &self.max_items {
            if count > constraint.limit {
                errors.push(max_items_error::<F>(
                    constraint, instance, location, tracker,
                ));
            }
        }
        match &self.items {
            FusedItems::Generic(node) => {
                for (idx, item) in array.elements().enumerate() {
                    errors.extend(node.iter_errors(&item, &location.push(idx), tracker, ctx));
                }
            }
            FusedItems::Number(cold)
            | FusedItems::String(cold)
            | FusedItems::Boolean(cold)
            | FusedItems::IntegerDraft4(cold)
            | FusedItems::IntegerDraft7(cold) => {
                errors.extend(cold.iter_errors(instance, location, tracker, ctx));
            }
        }
        if errors.is_empty() {
            no_error()
        } else {
            ErrorIterator::from_iterator(errors.into_iter())
        }
    }

    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        let Some(array) = instance.as_array() else {
            let error = ErrorDescription::new(
                "type",
                format!(r#"{} is not of type "array""#, instance.to_value()),
            );
            return EvaluationResult::from_children(vec![absorbed_error_node(
                location,
                tracker,
                &self.type_location,
                self.type_absolute_location.as_ref(),
                error,
            )]);
        };
        let count = array.len() as u64;
        let mut children = Vec::with_capacity(array.len());
        if let Some(constraint) = &self.min_items {
            if count < constraint.limit {
                let error = ErrorDescription::from_validation_error(&min_items_error::<F>(
                    constraint, instance, location, tracker,
                ));
                children.push(absorbed_error_node(
                    location,
                    tracker,
                    &constraint.location,
                    constraint.absolute_location.as_ref(),
                    error,
                ));
            }
        }
        if let Some(constraint) = &self.max_items {
            if count > constraint.limit {
                let error = ErrorDescription::from_validation_error(&max_items_error::<F>(
                    constraint, instance, location, tracker,
                ));
                children.push(absorbed_error_node(
                    location,
                    tracker,
                    &constraint.location,
                    constraint.absolute_location.as_ref(),
                    error,
                ));
            }
        }
        let element_result = match &self.items {
            FusedItems::Generic(node) => {
                let mut element_children = Vec::with_capacity(array.len());
                for (idx, item) in array.elements().enumerate() {
                    element_children.push(node.evaluate_instance(
                        &item,
                        &location.push(idx),
                        tracker,
                        ctx,
                    ));
                }
                let mut result = EvaluationResult::from_children(element_children);
                result.annotate(Annotations::new(serde_json::json!(array.len() != 0)));
                result
            }
            FusedItems::Number(cold)
            | FusedItems::String(cold)
            | FusedItems::Boolean(cold)
            | FusedItems::IntegerDraft4(cold)
            | FusedItems::IntegerDraft7(cold) => cold.evaluate(instance, location, tracker, ctx),
        };
        // Fold the element evaluation into this `items` node, keeping the absorbed length nodes
        // ahead of the element children.
        let (errors, mut element_children, annotations) = match element_result {
            EvaluationResult::Valid {
                annotations,
                children,
            } => (Vec::new(), children, annotations),
            EvaluationResult::Invalid {
                errors,
                children,
                annotations,
            } => (errors, children, annotations),
        };
        children.append(&mut element_children);
        if errors.is_empty() && children.iter().all(|node| node.valid) {
            EvaluationResult::Valid {
                annotations,
                children,
            }
        } else {
            EvaluationResult::Invalid {
                errors,
                children,
                annotations,
            }
        }
    }
}

/// Parses a length keyword exactly as `MinItemsValidator`/`MaxItemsValidator` would accept it.
#[allow(clippy::float_cmp)]
fn accepts_item_count<F: Json>(ctx: &compiler::Context<F>, schema: &Value) -> Option<u64> {
    if let Some(limit) = schema.as_u64() {
        return Some(limit);
    }
    if ctx.supports_integer_valued_numbers() {
        if let Some(limit) = schema.as_f64() {
            if limit.trunc() == limit {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                return Some(limit as u64);
            }
        }
    }
    None
}

/// Whether `{type: "array", (minItems|maxItems)?, items: {schema}}` can be fused into a single
/// `ArrayShapeValidator`. Any positional or extra element keyword blocks the fusion.
pub(crate) fn array_shape_fusion<F: Json>(
    ctx: &compiler::Context<F>,
    parent: &Map<String, Value>,
) -> bool {
    if !ctx.has_vocabulary(&Vocabulary::Validation) {
        return false;
    }
    match parent.get("type") {
        Some(Value::String(ty)) if ty.as_str() == "array" => {}
        _ => return false,
    }
    if !matches!(
        parent.get("items"),
        Some(Value::Object(_) | Value::Bool(false))
    ) {
        return false;
    }
    for key in [
        "prefixItems",
        "additionalItems",
        "contains",
        "unevaluatedItems",
        "uniqueItems",
    ] {
        if parent.contains_key(key) {
            return false;
        }
    }
    for key in ["minItems", "maxItems"] {
        if let Some(value) = parent.get(key) {
            if accepts_item_count(ctx, value).is_none() {
                return false;
            }
        }
    }
    true
}

/// Check if schema is a simple `{"type": "<type>"}` pattern and return the type.
fn get_simple_type_schema(schema: &Value) -> Option<&str> {
    let obj = schema.as_object()?;
    if obj.len() != 1 {
        return None;
    }
    obj.get("type")?.as_str()
}

#[inline]
pub(crate) fn compile<'a, F: Json>(
    ctx: &compiler::Context<F>,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a, F>> {
    match schema {
        Value::Array(items) => Some(ItemsArrayValidator::compile(ctx, items)),
        Value::Object(_) | Value::Bool(false) => {
            if array_shape_fusion(ctx, parent) {
                return Some(ArrayShapeValidator::compile(ctx, parent, schema));
            }
            if let Some(Value::Array(prefix_items)) = parent.get("prefixItems") {
                return Some(ItemsObjectSkipPrefixValidator::compile(
                    schema,
                    prefix_items.len(),
                    ctx,
                ));
            }
            // Specialized `{"type": ...}` validators assert `type`, so they apply only when
            // the validation vocabulary that defines `type` is in effect.
            if ctx.has_vocabulary(&Vocabulary::Validation) {
                if let Some(type_name) = get_simple_type_schema(schema) {
                    let location = ctx.location().join("items").join("type");
                    match type_name {
                        "number" => return Some(ItemsNumberTypeValidator::compile(location)),
                        "string" => return Some(ItemsStringTypeValidator::compile(location)),
                        "integer" => {
                            // Draft 4 has stricter integer semantics
                            return if ctx.draft() == Draft::Draft4 {
                                Some(ItemsIntegerTypeValidatorDraft4::compile(location))
                            } else {
                                Some(ItemsIntegerTypeValidator::compile(location))
                            };
                        }
                        "boolean" => return Some(ItemsBooleanTypeValidator::compile(location)),
                        _ => {}
                    }
                }
            }
            Some(ItemsObjectValidator::compile(ctx, schema))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use serde_json::{json, Value};
    use test_case::test_case;

    #[test_case(&json!({"items": false}), &json!([1]), "/items")]
    #[test_case(&json!({"items": {"type": "string"}}), &json!([1]), "/items/type")]
    #[test_case(&json!({"prefixItems": [{"type": "string"}]}), &json!([1]), "/prefixItems/0/type")]
    fn schema_location(schema: &Value, instance: &Value, expected: &str) {
        tests_util::assert_schema_location(schema, instance, expected);
    }

    #[test_case(&json!({"items": {"type": "string"}}), &json!([1]), "/0"; "string first")]
    #[test_case(&json!({"items": {"type": "string"}}), &json!(["a", 1]), "/1"; "string second")]
    #[test_case(&json!({"items": {"type": "number"}}), &json!(["x"]), "/0"; "number first")]
    #[test_case(&json!({"items": {"type": "integer"}}), &json!([1.5]), "/0"; "integer first")]
    #[test_case(&json!({"items": {"type": "boolean"}}), &json!([1]), "/0"; "boolean first")]
    fn instance_location(schema: &Value, instance: &Value, expected: &str) {
        let validator = crate::validator_for(schema).unwrap();
        let error = validator.iter_errors(instance).next().unwrap();
        assert_eq!(error.instance_path().as_str(), expected);
    }

    // Fused `type:array` + optional min/maxItems + schema `items` (ArrayShapeValidator)
    #[test_case(&json!({"type": "array", "items": {"type": "number"}}), &json!([1, 2, 3]), true; "all valid")]
    #[test_case(&json!({"type": "array", "items": {"type": "number"}}), &json!([1, "x"]), false; "bad element")]
    #[test_case(&json!({"type": "array", "items": {"type": "number"}}), &json!("nope"), false; "non-array")]
    #[test_case(&json!({"type": "array", "items": {"type": "number"}}), &json!([]), true; "empty array")]
    #[test_case(&json!({"type": "array", "minItems": 2, "items": {"type": "number"}}), &json!([1]), false; "too short")]
    #[test_case(&json!({"type": "array", "minItems": 2, "items": {"type": "number"}}), &json!([1, 2]), true; "min satisfied")]
    #[test_case(&json!({"type": "array", "maxItems": 2, "items": {"type": "number"}}), &json!([1, 2, 3]), false; "too long")]
    #[test_case(&json!({"type": "array", "minItems": 1, "maxItems": 3, "items": {"type": "number"}}), &json!([1, 2]), true; "within bounds")]
    #[test_case(&json!({"type": "array", "items": {"type": "array", "items": {"type": "number"}}}), &json!([[1], [2, 3]]), true; "nested valid")]
    #[test_case(&json!({"type": "array", "items": {"type": "array", "items": {"type": "number"}}}), &json!([[1], ["x"]]), false; "nested bad element")]
    #[test_case(&json!({"type": "array", "items": {"type": "string"}}), &json!(["a", "b"]), true; "string valid")]
    #[test_case(&json!({"type": "array", "items": {"type": "string"}}), &json!(["a", 1]), false; "string bad element")]
    #[test_case(&json!({"type": "array", "items": {"type": "boolean"}}), &json!([true, false]), true; "boolean valid")]
    #[test_case(&json!({"type": "array", "items": {"type": "boolean"}}), &json!([true, 1]), false; "boolean bad element")]
    #[test_case(&json!({"type": "array", "items": {"type": "integer"}}), &json!([1, 2]), true; "integer valid")]
    #[test_case(&json!({"type": "array", "items": {"type": "integer"}}), &json!([1, 1.5]), false; "integer bad element")]
    #[test_case(&json!({"type": "array", "minItems": 2.0, "items": {"type": "number"}}), &json!([1]), false; "float minItems too short")]
    #[test_case(&json!({"type": "array", "minItems": 2.0, "items": {"type": "number"}}), &json!([1, 2]), true; "float minItems satisfied")]
    #[test_case(&json!({"type": "array", "items": {"type": "null"}}), &json!([null, null]), true; "null items valid")]
    #[test_case(&json!({"type": "array", "items": {"type": "null"}}), &json!([null, 1]), false; "null items bad element")]
    fn array_shape_is_valid(schema: &Value, instance: &Value, expected: bool) {
        let validator = crate::validator_for(schema).unwrap();
        assert_eq!(validator.is_valid(instance), expected);
        // `is_valid`, `validate`, `iter_errors`, and `evaluate` must agree on the verdict.
        assert_eq!(validator.validate(instance).is_ok(), expected);
        assert_eq!(validator.iter_errors(instance).next().is_none(), expected);
        assert_eq!(validator.evaluate(instance).flag().valid, expected);
    }

    // Draft 4 integer element semantics (1.0 is not an integer) route through the fused validator.
    #[test_case(&json!([1, 2]), true; "d4 integers valid")]
    #[test_case(&json!([1, 1.0]), false; "d4 float not integer")]
    #[test_case(&json!([1, "x"]), false; "d4 non-integer element")]
    fn array_shape_integer_draft4(instance: &Value, expected: bool) {
        let schema = json!({"type": "array", "items": {"type": "integer"}});
        if expected {
            tests_util::is_valid_with_draft4(&schema, instance);
        } else {
            tests_util::is_not_valid_with_draft4(&schema, instance);
        }
    }

    // Absorbed keywords keep their own schema location in errors.
    #[test_case(&json!({"type": "array", "items": {"type": "number"}}), &json!("x"), "/type"; "type location")]
    #[test_case(&json!({"type": "array", "minItems": 2, "items": {"type": "number"}}), &json!([1]), "/minItems"; "min location")]
    #[test_case(&json!({"type": "array", "maxItems": 1, "items": {"type": "number"}}), &json!([1, 2]), "/maxItems"; "max location")]
    #[test_case(&json!({"type": "array", "items": {"type": "number"}}), &json!([1, "x"]), "/items/type"; "element location")]
    fn array_shape_schema_location(schema: &Value, instance: &Value, expected: &str) {
        tests_util::assert_schema_location(schema, instance, expected);
    }

    // A non-integer length keyword blocks fusion so the standalone validator still reports it.
    #[test]
    fn array_shape_invalid_length_keeps_error() {
        let schema = json!({"type": "array", "minItems": 1.5, "items": {"type": "number"}});
        assert!(crate::validator_for(&schema).is_err());
    }

    #[test]
    fn simple_type_items_respects_disabled_validation_vocabulary() {
        let meta = json!({
            "$id": "json-schema:///meta/no-validation",
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$vocabulary": {
                "https://json-schema.org/draft/2020-12/vocab/core": true,
                "https://json-schema.org/draft/2020-12/vocab/applicator": true,
                "https://json-schema.org/draft/2020-12/vocab/validation": false
            }
        });
        let registry = crate::Registry::new()
            .add("json-schema:///meta/no-validation", &meta)
            .unwrap()
            .prepare()
            .unwrap();
        let schema = json!({
            "$schema": "json-schema:///meta/no-validation",
            "type": "array",
            "items": {"type": "integer"}
        });
        let validator = crate::options()
            .with_registry(&registry)
            .build(&schema)
            .unwrap();
        assert!(validator.is_valid(&json!([1, "x"])));
    }

    fn parse_json(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    // Specialized string type validator tests
    #[test_case(r#"{"items": {"type": "string"}}"#, r#"["a", "b", "c"]"#, true; "all strings valid")]
    #[test_case(r#"{"items": {"type": "string"}}"#, r#"["a", 1, "c"]"#, false; "mixed with number invalid")]
    #[test_case(r#"{"items": {"type": "string"}}"#, r"[]", true; "empty array valid")]
    #[test_case(r#"{"items": {"type": "string"}}"#, r#"[""]"#, true; "empty string valid")]
    #[test_case(r#"{"items": {"type": "string"}}"#, r"[null]", false; "null invalid")]
    #[test_case(r#"{"items": {"type": "string"}}"#, r"[true]", false; "boolean invalid")]
    fn items_string_type(schema_json: &str, instance_json: &str, expected: bool) {
        let schema = parse_json(schema_json);
        let instance = parse_json(instance_json);
        if expected {
            tests_util::is_valid(&schema, &instance);
        } else {
            tests_util::is_not_valid(&schema, &instance);
        }
    }

    // Specialized number type validator tests
    #[test_case(r#"{"items": {"type": "number"}}"#, r"[1, 2.5, -3]", true; "all numbers valid")]
    #[test_case(r#"{"items": {"type": "number"}}"#, r#"[1, "2", 3]"#, false; "mixed with string invalid")]
    #[test_case(r#"{"items": {"type": "number"}}"#, r"[]", true; "empty array valid")]
    #[test_case(r#"{"items": {"type": "number"}}"#, r"[0]", true; "zero valid")]
    #[test_case(r#"{"items": {"type": "number"}}"#, r"[1.0]", true; "float valid")]
    #[test_case(r#"{"items": {"type": "number"}}"#, r"[null]", false; "null invalid")]
    #[test_case(r#"{"items": {"type": "number"}}"#, r"[9223372036854775807]", true; "i64 max valid")]
    #[test_case(r#"{"items": {"type": "number"}}"#, r"[-9223372036854775808]", true; "i64 min valid")]
    #[test_case(r#"{"items": {"type": "number"}}"#, r"[18446744073709551615]", true; "u64 max valid")]
    fn items_number_type(schema_json: &str, instance_json: &str, expected: bool) {
        let schema = parse_json(schema_json);
        let instance = parse_json(instance_json);
        if expected {
            tests_util::is_valid(&schema, &instance);
        } else {
            tests_util::is_not_valid(&schema, &instance);
        }
    }

    // Specialized boolean type validator tests
    #[test_case(r#"{"items": {"type": "boolean"}}"#, r"[true, false]", true; "all booleans valid")]
    #[test_case(r#"{"items": {"type": "boolean"}}"#, r"[true, 1]", false; "mixed with number invalid")]
    #[test_case(r#"{"items": {"type": "boolean"}}"#, r"[]", true; "empty array valid")]
    #[test_case(r#"{"items": {"type": "boolean"}}"#, r"[null]", false; "null invalid")]
    #[test_case(r#"{"items": {"type": "boolean"}}"#, r#"["true"]"#, false; "string true invalid")]
    fn items_boolean_type(schema_json: &str, instance_json: &str, expected: bool) {
        let schema = parse_json(schema_json);
        let instance = parse_json(instance_json);
        if expected {
            tests_util::is_valid(&schema, &instance);
        } else {
            tests_util::is_not_valid(&schema, &instance);
        }
    }

    // Specialized integer type validator tests (Draft 7+ semantics: 1.0 is integer)
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[1, 2, 3]", true; "d7 all integers valid")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[1, 2.5, 3]", false; "d7 float invalid")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[]", true; "d7 empty array valid")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[0]", true; "d7 zero valid")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[-42]", true; "d7 negative valid")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[1.0]", true; "d7 1.0 is integer")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[42.0]", true; "d7 42.0 is integer")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[-42.0]", true; "d7 neg 42.0 is integer")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[null]", false; "d7 null invalid")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r#"["1"]"#, false; "d7 string invalid")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[9223372036854775807]", true; "d7 i64 max valid")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[-9223372036854775808]", true; "d7 i64 min valid")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[18446744073709551615]", true; "d7 u64 max valid")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[1e10]", true; "d7 scientific notation integer")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[1e-10]", false; "d7 scientific small not integer")]
    fn items_integer_type_draft7(schema_json: &str, instance_json: &str, expected: bool) {
        let schema = parse_json(schema_json);
        let instance = parse_json(instance_json);
        if expected {
            tests_util::is_valid(&schema, &instance);
        } else {
            tests_util::is_not_valid(&schema, &instance);
        }
    }

    // Draft 4 integer semantics: 1.0 is NOT an integer
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[1, 2, 3]", true; "d4 all integers valid")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[1, 2.5, 3]", false; "d4 float invalid")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[]", true; "d4 empty array valid")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[1.0]", false; "d4 1.0 is NOT integer")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[42.0]", false; "d4 42.0 is NOT integer")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[-42.0]", false; "d4 neg 42.0 is NOT integer")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[9223372036854775807]", true; "d4 i64 max valid")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[-9223372036854775808]", true; "d4 i64 min valid")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[18446744073709551615]", true; "d4 u64 max valid")]
    fn items_integer_type_draft4(schema_json: &str, instance_json: &str, expected: bool) {
        let schema = parse_json(schema_json);
        let instance = parse_json(instance_json);
        if expected {
            tests_util::is_valid_with_draft4(&schema, &instance);
        } else {
            tests_util::is_not_valid_with_draft4(&schema, &instance);
        }
    }

    #[cfg(feature = "arbitrary-precision")]
    mod arbitrary_precision {
        use crate::tests_util;
        use serde_json::Value;
        use test_case::test_case;

        fn parse_json(s: &str) -> Value {
            serde_json::from_str(s).unwrap()
        }

        // Draft 7+ with huge integers
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[18446744073709551616]", true; "u64 max plus 1")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[18446744073709551616.0]", true; "u64 max plus 1 with .0")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[99999999999999999999]", true; "huge plain integer")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[99999999999999999999.0]", true; "huge integer with .0")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[-18446744073709551616]", true; "negative huge")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[-18446744073709551616.0]", true; "negative huge with .0")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[18446744073709551616.5]", false; "huge decimal")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[99999999999999999999.5]", false; "huge float")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[1e1000]", true; "huge scientific notation")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[1e1000001]", false; "infinity positive")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[-1e1000001]", false; "infinity negative")]
        fn items_integer_huge_draft7(schema_json: &str, instance_json: &str, expected: bool) {
            let schema = parse_json(schema_json);
            let instance = parse_json(instance_json);
            if expected {
                tests_util::is_valid(&schema, &instance);
            } else {
                tests_util::is_not_valid(&schema, &instance);
            }
        }

        // Draft 4 with huge integers (stricter: .0 is NOT integer)
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[18446744073709551616]", true; "u64 max plus 1")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[18446744073709551616.0]", false; "u64 max plus 1 with .0 NOT integer")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[99999999999999999999]", true; "huge plain integer")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[99999999999999999999.0]", false; "huge integer with .0 NOT integer")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[-18446744073709551616]", true; "negative huge")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[-18446744073709551616.0]", false; "negative huge with .0 NOT integer")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[18446744073709551616.5]", false; "huge decimal")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[1e1000]", true; "huge scientific notation")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[1e1000001]", false; "infinity positive")]
        fn items_integer_huge_draft4(schema_json: &str, instance_json: &str, expected: bool) {
            let schema = parse_json(schema_json);
            let instance = parse_json(instance_json);
            if expected {
                tests_util::is_valid_with_draft4(&schema, &instance);
            } else {
                tests_util::is_not_valid_with_draft4(&schema, &instance);
            }
        }

        // Huge numbers for number type (all should be valid)
        #[test_case(r#"{"items": {"type": "number"}}"#, r"[18446744073709551616]", true; "huge int valid as number")]
        #[test_case(r#"{"items": {"type": "number"}}"#, r"[18446744073709551616.0]", true; "huge .0 valid as number")]
        #[test_case(r#"{"items": {"type": "number"}}"#, r"[18446744073709551616.5]", true; "huge float valid as number")]
        #[test_case(r#"{"items": {"type": "number"}}"#, r"[1e10000]", true; "infinity valid as number")]
        fn items_number_huge(schema_json: &str, instance_json: &str, expected: bool) {
            let schema = parse_json(schema_json);
            let instance = parse_json(instance_json);
            if expected {
                tests_util::is_valid(&schema, &instance);
            } else {
                tests_util::is_not_valid(&schema, &instance);
            }
        }
    }
}
