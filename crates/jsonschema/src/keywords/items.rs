use crate::{
    compiler,
    error::{no_error, ErrorIterator},
    evaluation::{Annotations, ErrorDescription},
    keywords::CompilationResult,
    node::SchemaNode,
    paths::{LazyLocation, Location, RefTracker},
    types::JsonType,
    validator::{EvaluationResult, Validate, ValidationContext},
    Draft, ValidationError,
};
use serde_json::{Map, Value};

pub(crate) struct ItemsArrayValidator {
    items: Vec<SchemaNode>,
}
impl ItemsArrayValidator {
    #[inline]
    pub(crate) fn compile<'a>(
        ctx: &compiler::Context,
        schemas: &'a [Value],
    ) -> CompilationResult<'a> {
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
impl Validate for ItemsArrayValidator {
    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        if let Value::Array(items) = instance {
            for (item, node) in items.iter().zip(self.items.iter()) {
                if !node.is_valid(item, ctx) {
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
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Array(items) = instance {
            for (idx, (item, node)) in items.iter().zip(self.items.iter()).enumerate() {
                node.validate(item, &location.push(idx), tracker, ctx)?;
            }
        }
        Ok(())
    }

    fn iter_errors<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Value::Array(items) = instance {
            let mut errors = Vec::new();
            for (idx, (item, node)) in items.iter().zip(self.items.iter()).enumerate() {
                errors.extend(node.iter_errors(item, &location.push(idx), tracker, ctx));
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
        if let Value::Array(items) = instance {
            let mut children = Vec::with_capacity(self.items.len().min(items.len()));
            for (idx, (item, node)) in items.iter().zip(self.items.iter()).enumerate() {
                children.push(node.evaluate_instance(item, &location.push(idx), tracker, ctx));
            }
            EvaluationResult::from_children(children)
        } else {
            EvaluationResult::valid_empty()
        }
    }
}

pub(crate) struct ItemsObjectValidator {
    node: SchemaNode,
}

impl ItemsObjectValidator {
    #[inline]
    pub(crate) fn compile<'a>(ctx: &compiler::Context, schema: &'a Value) -> CompilationResult<'a> {
        let ctx = ctx.new_at_location("items");
        let node = compiler::compile(&ctx, ctx.as_resource_ref(schema))?;
        Ok(Box::new(ItemsObjectValidator { node }))
    }
}
impl Validate for ItemsObjectValidator {
    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        if let Value::Array(items) = instance {
            items.iter().all(|i| self.node.is_valid(i, ctx))
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
        if let Value::Array(items) = instance {
            for (idx, item) in items.iter().enumerate() {
                self.node
                    .validate(item, &location.push(idx), tracker, ctx)?;
            }
        }
        Ok(())
    }

    fn iter_errors<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Value::Array(items) = instance {
            let mut errors = Vec::new();
            for (idx, item) in items.iter().enumerate() {
                errors.extend(
                    self.node
                        .iter_errors(item, &location.push(idx), tracker, ctx),
                );
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
        if let Value::Array(items) = instance {
            let mut children = Vec::with_capacity(items.len());
            for (idx, item) in items.iter().enumerate() {
                children.push(
                    self.node
                        .evaluate_instance(item, &location.push(idx), tracker, ctx),
                );
            }
            let schema_was_applied = !items.is_empty();
            let mut result = EvaluationResult::from_children(children);
            result.annotate(Annotations::new(serde_json::json!(schema_was_applied)));
            result
        } else {
            EvaluationResult::valid_empty()
        }
    }
}

pub(crate) struct ItemsObjectSkipPrefixValidator {
    node: SchemaNode,
    skip_prefix: usize,
}

impl ItemsObjectSkipPrefixValidator {
    #[inline]
    pub(crate) fn compile<'a>(
        schema: &'a Value,
        skip_prefix: usize,
        ctx: &compiler::Context,
    ) -> CompilationResult<'a> {
        let ctx = ctx.new_at_location("items");
        let node = compiler::compile(&ctx, ctx.as_resource_ref(schema))?;
        Ok(Box::new(ItemsObjectSkipPrefixValidator {
            node,
            skip_prefix,
        }))
    }
}

impl Validate for ItemsObjectSkipPrefixValidator {
    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        if let Value::Array(items) = instance {
            items
                .iter()
                .skip(self.skip_prefix)
                .all(|i| self.node.is_valid(i, ctx))
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
        if let Value::Array(items) = instance {
            for (idx, item) in items.iter().skip(self.skip_prefix).enumerate() {
                self.node
                    .validate(item, &location.push(idx + self.skip_prefix), tracker, ctx)?;
            }
        }
        Ok(())
    }

    fn iter_errors<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Value::Array(items) = instance {
            let mut errors = Vec::new();
            for (idx, item) in items.iter().skip(self.skip_prefix).enumerate() {
                errors.extend(self.node.iter_errors(
                    item,
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
        instance: &Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Value::Array(items) = instance {
            let mut children = Vec::with_capacity(items.len().saturating_sub(self.skip_prefix));
            for (idx, item) in items.iter().enumerate().skip(self.skip_prefix) {
                children.push(
                    self.node
                        .evaluate_instance(item, &location.push(idx), tracker, ctx),
                );
            }
            let schema_was_applied = items.len() > self.skip_prefix;
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
    pub(crate) fn compile<'a>(location: Location) -> CompilationResult<'a> {
        Ok(Box::new(ItemsNumberTypeValidator { location }))
    }
}

impl Validate for ItemsNumberTypeValidator {
    #[inline]
    fn is_valid(&self, instance: &Value, _ctx: &mut ValidationContext) -> bool {
        if let Value::Array(items) = instance {
            items.iter().all(Value::is_number)
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Array(items) = instance {
            for (idx, item) in items.iter().enumerate() {
                if !item.is_number() {
                    return Err(ValidationError::single_type_error(
                        self.location.clone(),
                        crate::paths::capture_evaluation_path(tracker, &self.location),
                        (&location.push(idx)).into(),
                        item,
                        JsonType::Number,
                    ));
                }
            }
        }
        Ok(())
    }

    fn iter_errors<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Value::Array(items) = instance {
            let errors: Vec<_> = items
                .iter()
                .enumerate()
                .filter(|(_, item)| !item.is_number())
                .map(|(idx, item)| {
                    ValidationError::single_type_error(
                        self.location.clone(),
                        crate::paths::capture_evaluation_path(tracker, &self.location),
                        (&location.push(idx)).into(),
                        item,
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
        instance: &Value,
        _location: &LazyLocation,
        _tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Value::Array(items) = instance {
            let errors: Vec<_> = items
                .iter()
                .enumerate()
                .filter(|(_, item)| !item.is_number())
                .map(|(idx, item)| {
                    ErrorDescription::new(
                        "type",
                        format!(r#"{item} at index {idx} is not of type "number""#),
                    )
                })
                .collect();
            let schema_was_applied = !items.is_empty();
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
    pub(crate) fn compile<'a>(location: Location) -> CompilationResult<'a> {
        Ok(Box::new(ItemsStringTypeValidator { location }))
    }
}

impl Validate for ItemsStringTypeValidator {
    #[inline]
    fn is_valid(&self, instance: &Value, _ctx: &mut ValidationContext) -> bool {
        if let Value::Array(items) = instance {
            items.iter().all(Value::is_string)
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Array(items) = instance {
            for (idx, item) in items.iter().enumerate() {
                if !item.is_string() {
                    return Err(ValidationError::single_type_error(
                        self.location.clone(),
                        crate::paths::capture_evaluation_path(tracker, &self.location),
                        (&location.push(idx)).into(),
                        item,
                        JsonType::String,
                    ));
                }
            }
        }
        Ok(())
    }

    fn iter_errors<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Value::Array(items) = instance {
            let errors: Vec<_> = items
                .iter()
                .enumerate()
                .filter(|(_, item)| !item.is_string())
                .map(|(idx, item)| {
                    ValidationError::single_type_error(
                        self.location.clone(),
                        crate::paths::capture_evaluation_path(tracker, &self.location),
                        (&location.push(idx)).into(),
                        item,
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
        instance: &Value,
        _location: &LazyLocation,
        _tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Value::Array(items) = instance {
            let errors: Vec<_> = items
                .iter()
                .enumerate()
                .filter(|(_, item)| !item.is_string())
                .map(|(idx, item)| {
                    ErrorDescription::new(
                        "type",
                        format!(r#"{item} at index {idx} is not of type "string""#),
                    )
                })
                .collect();
            let schema_was_applied = !items.is_empty();
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
    pub(crate) fn compile<'a>(location: Location) -> CompilationResult<'a> {
        Ok(Box::new(ItemsIntegerTypeValidator { location }))
    }
}

impl Validate for ItemsIntegerTypeValidator {
    #[inline]
    fn is_valid(&self, instance: &Value, _ctx: &mut ValidationContext) -> bool {
        if let Value::Array(items) = instance {
            items.iter().all(|item| {
                if let Value::Number(n) = item {
                    super::type_::is_integer(n)
                } else {
                    false
                }
            })
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Array(items) = instance {
            for (idx, item) in items.iter().enumerate() {
                let valid = if let Value::Number(n) = item {
                    super::type_::is_integer(n)
                } else {
                    false
                };
                if !valid {
                    return Err(ValidationError::single_type_error(
                        self.location.clone(),
                        crate::paths::capture_evaluation_path(tracker, &self.location),
                        (&location.push(idx)).into(),
                        item,
                        JsonType::Integer,
                    ));
                }
            }
        }
        Ok(())
    }

    fn iter_errors<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Value::Array(items) = instance {
            let errors: Vec<_> = items
                .iter()
                .enumerate()
                .filter(|(_, item)| {
                    if let Value::Number(n) = item {
                        !super::type_::is_integer(n)
                    } else {
                        true
                    }
                })
                .map(|(idx, item)| {
                    ValidationError::single_type_error(
                        self.location.clone(),
                        crate::paths::capture_evaluation_path(tracker, &self.location),
                        (&location.push(idx)).into(),
                        item,
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
        instance: &Value,
        _location: &LazyLocation,
        _tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Value::Array(items) = instance {
            let errors: Vec<_> = items
                .iter()
                .enumerate()
                .filter(|(_, item)| {
                    if let Value::Number(n) = item {
                        !super::type_::is_integer(n)
                    } else {
                        true
                    }
                })
                .map(|(idx, item)| {
                    ErrorDescription::new(
                        "type",
                        format!(r#"{item} at index {idx} is not of type "integer""#),
                    )
                })
                .collect();
            let schema_was_applied = !items.is_empty();
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
    pub(crate) fn compile<'a>(location: Location) -> CompilationResult<'a> {
        Ok(Box::new(ItemsIntegerTypeValidatorDraft4 { location }))
    }
}

impl Validate for ItemsIntegerTypeValidatorDraft4 {
    #[inline]
    fn is_valid(&self, instance: &Value, _ctx: &mut ValidationContext) -> bool {
        if let Value::Array(items) = instance {
            items.iter().all(|item| {
                if let Value::Number(n) = item {
                    super::legacy::type_draft_4::is_integer(n)
                } else {
                    false
                }
            })
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Array(items) = instance {
            for (idx, item) in items.iter().enumerate() {
                let valid = if let Value::Number(n) = item {
                    super::legacy::type_draft_4::is_integer(n)
                } else {
                    false
                };
                if !valid {
                    return Err(ValidationError::single_type_error(
                        self.location.clone(),
                        crate::paths::capture_evaluation_path(tracker, &self.location),
                        (&location.push(idx)).into(),
                        item,
                        JsonType::Integer,
                    ));
                }
            }
        }
        Ok(())
    }

    fn iter_errors<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Value::Array(items) = instance {
            let errors: Vec<_> = items
                .iter()
                .enumerate()
                .filter(|(_, item)| {
                    if let Value::Number(n) = item {
                        !super::legacy::type_draft_4::is_integer(n)
                    } else {
                        true
                    }
                })
                .map(|(idx, item)| {
                    ValidationError::single_type_error(
                        self.location.clone(),
                        crate::paths::capture_evaluation_path(tracker, &self.location),
                        (&location.push(idx)).into(),
                        item,
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
        instance: &Value,
        _location: &LazyLocation,
        _tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Value::Array(items) = instance {
            let errors: Vec<_> = items
                .iter()
                .enumerate()
                .filter(|(_, item)| {
                    if let Value::Number(n) = item {
                        !super::legacy::type_draft_4::is_integer(n)
                    } else {
                        true
                    }
                })
                .map(|(idx, item)| {
                    ErrorDescription::new(
                        "type",
                        format!(r#"{item} at index {idx} is not of type "integer""#),
                    )
                })
                .collect();
            let schema_was_applied = !items.is_empty();
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
    pub(crate) fn compile<'a>(location: Location) -> CompilationResult<'a> {
        Ok(Box::new(ItemsBooleanTypeValidator { location }))
    }
}

impl Validate for ItemsBooleanTypeValidator {
    #[inline]
    fn is_valid(&self, instance: &Value, _ctx: &mut ValidationContext) -> bool {
        if let Value::Array(items) = instance {
            items.iter().all(Value::is_boolean)
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Array(items) = instance {
            for (idx, item) in items.iter().enumerate() {
                if !item.is_boolean() {
                    return Err(ValidationError::single_type_error(
                        self.location.clone(),
                        crate::paths::capture_evaluation_path(tracker, &self.location),
                        (&location.push(idx)).into(),
                        item,
                        JsonType::Boolean,
                    ));
                }
            }
        }
        Ok(())
    }

    fn iter_errors<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Value::Array(items) = instance {
            let errors: Vec<_> = items
                .iter()
                .enumerate()
                .filter(|(_, item)| !item.is_boolean())
                .map(|(idx, item)| {
                    ValidationError::single_type_error(
                        self.location.clone(),
                        crate::paths::capture_evaluation_path(tracker, &self.location),
                        (&location.push(idx)).into(),
                        item,
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
        instance: &Value,
        _location: &LazyLocation,
        _tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Value::Array(items) = instance {
            let errors: Vec<_> = items
                .iter()
                .enumerate()
                .filter(|(_, item)| !item.is_boolean())
                .map(|(idx, item)| {
                    ErrorDescription::new(
                        "type",
                        format!(r#"{item} at index {idx} is not of type "boolean""#),
                    )
                })
                .collect();
            let schema_was_applied = !items.is_empty();
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

/// Check if schema is a simple `{"type": "<type>"}` pattern and return the type.
fn get_simple_type_schema(schema: &Value) -> Option<&str> {
    let obj = schema.as_object()?;
    if obj.len() != 1 {
        return None;
    }
    obj.get("type")?.as_str()
}

#[inline]
pub(crate) fn compile<'a>(
    ctx: &compiler::Context,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>> {
    match schema {
        Value::Array(items) => Some(ItemsArrayValidator::compile(ctx, items)),
        Value::Object(_) | Value::Bool(false) => {
            if let Some(Value::Array(prefix_items)) = parent.get("prefixItems") {
                return Some(ItemsObjectSkipPrefixValidator::compile(
                    schema,
                    prefix_items.len(),
                    ctx,
                ));
            }
            // Try to use specialized validators for simple type schemas
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
