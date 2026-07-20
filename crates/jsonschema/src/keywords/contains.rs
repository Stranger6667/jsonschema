use crate::{
    compiler,
    error::ValidationError,
    evaluation::{Annotations, ErrorDescription},
    keywords::CompilationResult,
    node::SchemaNode,
    paths::{LazyLocation, RefTracker},
    validator::{EvaluationResult, Validate, ValidationContext},
    Draft, Json, JsonArrayAccess, JsonNode, SerdeJson,
};
use serde_json::{Map, Value};

use super::helpers::map_get_u64;

pub(crate) struct ContainsValidator<F: Json = SerdeJson> {
    node: SchemaNode<F>,
}

impl ContainsValidator {
    #[inline]
    pub(crate) fn compile<'a>(ctx: &compiler::Context, schema: &'a Value) -> CompilationResult<'a> {
        let ctx = ctx.new_at_location("contains");
        Ok(Box::new(ContainsValidator {
            node: compiler::compile(&ctx, ctx.as_resource_ref(schema))?,
        }))
    }
}

impl<F: Json> Validate<F> for ContainsValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        if let Some(array) = instance.as_array() {
            array.elements().any(|item| self.node.is_valid(&item, ctx))
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
            if array.elements().any(|item| self.node.is_valid(&item, ctx)) {
                return Ok(());
            }
            let loc = self.node.location();
            Err(ValidationError::contains(
                loc.clone(),
                crate::paths::capture_evaluation_path(tracker, loc),
                location.into(),
                instance.to_value(),
            ))
        } else {
            Ok(())
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
            let mut results = Vec::with_capacity(array.len());
            let mut indices = Vec::with_capacity(array.len());
            for (idx, item) in array.elements().enumerate() {
                let path = location.push(idx);
                let result = self.node.evaluate_instance(&item, &path, tracker, ctx);
                if result.valid {
                    indices.push(idx);
                    results.push(result);
                }
            }
            if indices.is_empty() {
                let loc = self.node.location();
                let eval_path = crate::paths::capture_evaluation_path(tracker, loc);
                EvaluationResult::Invalid {
                    errors: vec![ErrorDescription::from_validation_error(
                        &ValidationError::contains(
                            loc.clone(),
                            eval_path,
                            location.into(),
                            instance.to_value(),
                        ),
                    )],
                    children: Vec::new(),
                    annotations: None,
                }
            } else {
                EvaluationResult::Valid {
                    annotations: Some(Annotations::new(Value::from(indices))),
                    children: results,
                }
            }
        } else {
            let mut result = EvaluationResult::valid_empty();
            result.annotate(Annotations::new(Value::Array(Vec::new())));
            result
        }
    }
}

/// `minContains` validation. Used only if there is no `maxContains` present.
///
/// Docs: <https://json-schema.org/draft/2019-09/json-schema-validation.html#rfc.section.6.4.5>
pub(crate) struct MinContainsValidator<F: Json = SerdeJson> {
    node: SchemaNode<F>,
    min_contains: u64,
}

impl MinContainsValidator {
    #[inline]
    pub(crate) fn compile<'a>(
        ctx: &compiler::Context,
        schema: &'a Value,
        min_contains: u64,
    ) -> CompilationResult<'a> {
        let ctx = ctx.new_at_location("minContains");
        Ok(Box::new(MinContainsValidator {
            node: compiler::compile(&ctx, ctx.as_resource_ref(schema))?,
            min_contains,
        }))
    }
}

impl<F: Json> Validate<F> for MinContainsValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        if let Some(array) = instance.as_array() {
            let mut matches = 0;
            for item in array.elements() {
                if self
                    .node
                    .validators()
                    .all(|validator| validator.is_valid(&item, ctx))
                {
                    matches += 1;
                    if matches >= self.min_contains {
                        return true;
                    }
                }
            }
            self.min_contains == 0
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
            let mut matches = 0;
            for item in array.elements() {
                if self
                    .node
                    .validators()
                    .all(|validator| validator.is_valid(&item, ctx))
                {
                    matches += 1;
                    if matches >= self.min_contains {
                        return Ok(());
                    }
                }
            }
            if self.min_contains > 0 {
                let loc = self.node.location();
                Err(ValidationError::contains(
                    loc.clone(),
                    crate::paths::capture_evaluation_path(tracker, loc),
                    location.into(),
                    instance.to_value(),
                ))
            } else {
                Ok(())
            }
        } else {
            Ok(())
        }
    }
}

/// `maxContains` validation. Used only if there is no `minContains` present.
///
/// Docs: <https://json-schema.org/draft/2019-09/json-schema-validation.html#rfc.section.6.4.4>
pub(crate) struct MaxContainsValidator<F: Json = SerdeJson> {
    node: SchemaNode<F>,
    max_contains: u64,
}

impl MaxContainsValidator {
    #[inline]
    pub(crate) fn compile<'a>(
        ctx: &compiler::Context,
        schema: &'a Value,
        max_contains: u64,
    ) -> CompilationResult<'a> {
        let ctx = ctx.new_at_location("maxContains");
        Ok(Box::new(MaxContainsValidator {
            node: compiler::compile(&ctx, ctx.as_resource_ref(schema))?,
            max_contains,
        }))
    }
}

impl<F: Json> Validate<F> for MaxContainsValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        if let Some(array) = instance.as_array() {
            let mut matches = 0;
            for item in array.elements() {
                if self
                    .node
                    .validators()
                    .all(|validator| validator.is_valid(&item, ctx))
                {
                    matches += 1;
                    if matches > self.max_contains {
                        return false;
                    }
                }
            }
            matches != 0
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
            let loc = self.node.location();
            let mut matches = 0;
            for item in array.elements() {
                if self
                    .node
                    .validators()
                    .all(|validator| validator.is_valid(&item, ctx))
                {
                    matches += 1;
                    if matches > self.max_contains {
                        return Err(ValidationError::contains(
                            loc.clone(),
                            crate::paths::capture_evaluation_path(tracker, loc),
                            location.into(),
                            instance.to_value(),
                        ));
                    }
                }
            }
            if matches > 0 {
                Ok(())
            } else {
                Err(ValidationError::contains(
                    loc.clone(),
                    crate::paths::capture_evaluation_path(tracker, loc),
                    location.into(),
                    instance.to_value(),
                ))
            }
        } else {
            Ok(())
        }
    }
}

/// `maxContains` & `minContains` validation combined.
///
/// Docs:
///   `maxContains` - <https://json-schema.org/draft/2019-09/json-schema-validation.html#rfc.section.6.4.4>
///   `minContains` - <https://json-schema.org/draft/2019-09/json-schema-validation.html#rfc.section.6.4.5>
pub(crate) struct MinMaxContainsValidator<F: Json = SerdeJson> {
    node: SchemaNode<F>,
    min_contains: u64,
    max_contains: u64,
}

impl MinMaxContainsValidator {
    #[inline]
    pub(crate) fn compile<'a>(
        ctx: &compiler::Context,
        schema: &'a Value,
        min_contains: u64,
        max_contains: u64,
    ) -> CompilationResult<'a> {
        Ok(Box::new(MinMaxContainsValidator {
            node: compiler::compile(ctx, ctx.as_resource_ref(schema))?,
            min_contains,
            max_contains,
        }))
    }
}

impl<F: Json> Validate<F> for MinMaxContainsValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        if let Some(array) = instance.as_array() {
            let mut matches = 0;
            for item in array.elements() {
                if self
                    .node
                    .validators()
                    .all(|validator| validator.is_valid(&item, ctx))
                {
                    matches += 1;
                    if matches > self.max_contains {
                        return false;
                    }
                }
            }
            matches <= self.max_contains && matches >= self.min_contains
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
            let mut matches = 0;
            for item in array.elements() {
                if self
                    .node
                    .validators()
                    .all(|validator| validator.is_valid(&item, ctx))
                {
                    matches += 1;
                    if matches > self.max_contains {
                        let max_location = self.node.location().join("maxContains");
                        let eval_path =
                            crate::paths::capture_evaluation_path(tracker, &max_location);
                        return Err(ValidationError::contains(
                            max_location,
                            eval_path,
                            location.into(),
                            instance.to_value(),
                        ));
                    }
                }
            }
            if matches < self.min_contains {
                let min_location = self.node.location().join("minContains");
                let eval_path = crate::paths::capture_evaluation_path(tracker, &min_location);
                Err(ValidationError::contains(
                    min_location,
                    eval_path,
                    location.into(),
                    instance.to_value(),
                ))
            } else {
                Ok(())
            }
        } else {
            Ok(())
        }
    }
}

#[inline]
pub(crate) fn compile<'a>(
    ctx: &compiler::Context,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>> {
    match ctx.draft() {
        Draft::Draft4 | Draft::Draft6 | Draft::Draft7 => {
            Some(ContainsValidator::compile(ctx, schema))
        }
        Draft::Draft201909 | Draft::Draft202012 => compile_contains(ctx, parent, schema),
        _ => None,
    }
}

#[inline]
fn compile_contains<'a>(
    ctx: &compiler::Context,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>> {
    let min_contains = match map_get_u64(parent, ctx, "minContains").transpose() {
        Ok(n) => n,
        Err(err) => return Some(Err(err)),
    };
    let max_contains = match map_get_u64(parent, ctx, "maxContains").transpose() {
        Ok(n) => n,
        Err(err) => return Some(Err(err)),
    };

    match (min_contains, max_contains) {
        (Some(min), Some(max)) => Some(MinMaxContainsValidator::compile(ctx, schema, min, max)),
        (Some(min), None) => Some(MinContainsValidator::compile(ctx, schema, min)),
        (None, Some(max)) => Some(MaxContainsValidator::compile(ctx, schema, max)),
        (None, None) => Some(ContainsValidator::compile(ctx, schema)),
    }
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use serde_json::json;

    #[test]
    fn location() {
        tests_util::assert_schema_location(
            &json!({"contains": {"const": 2}}),
            &json!([]),
            "/contains",
        );
    }
}
