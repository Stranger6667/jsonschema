use crate::{
    compiler,
    error::ValidationError,
    evaluation::{Annotations, ErrorDescription},
    keywords::CompilationResult,
    node::SchemaNode,
    paths::LazyLocation,
    validator::{EvaluationResult, Validate, ValidationContext},
    Draft,
};
use serde_json::{Map, Value};

use super::helpers::map_get_u64;

pub(crate) struct ContainsValidator {
    node: SchemaNode,
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

impl Validate for ContainsValidator {
    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        if let Value::Array(items) = instance {
            items.iter().any(|i| self.node.is_valid(i, ctx))
        } else {
            true
        }
    }

    fn schema_path(&self) -> &crate::paths::Location {
        self.node.location()
    }

    fn matches_type(&self, instance: &Value) -> bool {
        matches!(instance, Value::Array(_))
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Array(items) = instance {
            if items.iter().any(|i| self.node.is_valid(i, ctx)) {
                return Ok(());
            }
            Err(ValidationError::contains(
                self.node.location().clone(),
                location.into(),
                instance,
            ))
        } else {
            Ok(())
        }
    }

    fn evaluate(
        &self,
        instance: &Value,
        location: &LazyLocation,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Value::Array(items) = instance {
            let mut results = Vec::with_capacity(items.len());
            let mut indices = Vec::with_capacity(items.len());
            for (idx, item) in items.iter().enumerate() {
                let path = location.push(idx);
                let result = self.node.evaluate_instance(item, &path, ctx);
                if result.valid {
                    indices.push(idx);
                    results.push(result);
                }
            }
            if indices.is_empty() {
                EvaluationResult::Invalid {
                    errors: vec![ErrorDescription::from_validation_error(
                        &ValidationError::contains(
                            self.node.location().clone(),
                            location.into(),
                            instance,
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

    fn trace(
        &self,
        instance: &Value,
        instance_path: &LazyLocation,
        callback: crate::tracing::TracingCallback<'_>,
        ctx: &mut ValidationContext,
    ) -> bool {
        if let Value::Array(items) = instance {
            let mut match_count = 0u64;
            for (idx, item) in items.iter().enumerate() {
                let path = instance_path.push(idx);
                if self.node.trace(item, &path, callback, ctx) {
                    match_count += 1;
                }
            }
            let is_valid = match_count >= 1;
            crate::tracing::TracingContext::new(instance_path, self.schema_path(), is_valid)
                .call(callback);
            is_valid
        } else {
            crate::tracing::TracingContext::new(instance_path, self.schema_path(), None)
                .call(callback);
            true
        }
    }
}

/// `minContains` validation. Used only if there is no `maxContains` present.
///
/// Docs: <https://json-schema.org/draft/2019-09/json-schema-validation.html#rfc.section.6.4.5>
pub(crate) struct MinContainsValidator {
    node: SchemaNode,
    min_contains: u64,
    min_contains_location: crate::paths::Location,
}

impl MinContainsValidator {
    #[inline]
    pub(crate) fn compile<'a>(
        ctx: &compiler::Context,
        schema: &'a Value,
        min_contains: u64,
    ) -> CompilationResult<'a> {
        let min_contains_location = ctx.new_at_location("minContains").location().clone();
        let ctx = ctx.new_at_location("contains");
        Ok(Box::new(MinContainsValidator {
            node: compiler::compile(&ctx, ctx.as_resource_ref(schema))?,
            min_contains,
            min_contains_location,
        }))
    }
}

impl Validate for MinContainsValidator {
    fn schema_path(&self) -> &crate::paths::Location {
        self.node.location()
    }

    fn matches_type(&self, instance: &Value) -> bool {
        matches!(instance, Value::Array(_))
    }

    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        if let Value::Array(items) = instance {
            let mut matches = 0;
            for item in items {
                if self
                    .node
                    .validators()
                    .all(|validator| validator.is_valid(item, ctx))
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
        instance: &'i Value,
        location: &LazyLocation,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Array(items) = instance {
            let mut matches = 0;
            for item in items {
                if self
                    .node
                    .validators()
                    .all(|validator| validator.is_valid(item, ctx))
                {
                    matches += 1;
                    if matches >= self.min_contains {
                        return Ok(());
                    }
                }
            }
            if self.min_contains > 0 {
                Err(ValidationError::contains(
                    self.node.location().clone(),
                    location.into(),
                    instance,
                ))
            } else {
                Ok(())
            }
        } else {
            Ok(())
        }
    }

    fn trace(
        &self,
        instance: &Value,
        instance_path: &LazyLocation,
        callback: crate::tracing::TracingCallback<'_>,
        ctx: &mut ValidationContext,
    ) -> bool {
        if let Value::Array(items) = instance {
            let mut match_count = 0u64;
            for (idx, item) in items.iter().enumerate() {
                let path = instance_path.push(idx);
                if self.node.trace(item, &path, callback, ctx) {
                    match_count += 1;
                }
            }
            // Trace contains schema result
            let contains_valid = match_count >= 1;
            crate::tracing::TracingContext::new(instance_path, self.schema_path(), contains_valid)
                .call(callback);
            // Trace minContains constraint
            let min_valid = match_count >= self.min_contains;
            crate::tracing::TracingContext::new(
                instance_path,
                &self.min_contains_location,
                min_valid,
            )
            .call(callback);
            min_valid
        } else {
            crate::tracing::TracingContext::new(instance_path, self.schema_path(), None)
                .call(callback);
            crate::tracing::TracingContext::new(instance_path, &self.min_contains_location, None)
                .call(callback);
            true
        }
    }
}

/// `maxContains` validation. Used only if there is no `minContains` present.
///
/// Docs: <https://json-schema.org/draft/2019-09/json-schema-validation.html#rfc.section.6.4.4>
pub(crate) struct MaxContainsValidator {
    node: SchemaNode,
    max_contains: u64,
    max_contains_location: crate::paths::Location,
}

impl MaxContainsValidator {
    #[inline]
    pub(crate) fn compile<'a>(
        ctx: &compiler::Context,
        schema: &'a Value,
        max_contains: u64,
    ) -> CompilationResult<'a> {
        let max_contains_location = ctx.new_at_location("maxContains").location().clone();
        let ctx = ctx.new_at_location("contains");
        Ok(Box::new(MaxContainsValidator {
            node: compiler::compile(&ctx, ctx.as_resource_ref(schema))?,
            max_contains,
            max_contains_location,
        }))
    }
}

impl Validate for MaxContainsValidator {
    fn schema_path(&self) -> &crate::paths::Location {
        self.node.location()
    }

    fn matches_type(&self, instance: &Value) -> bool {
        matches!(instance, Value::Array(_))
    }

    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        if let Value::Array(items) = instance {
            let mut matches = 0;
            for item in items {
                if self
                    .node
                    .validators()
                    .all(|validator| validator.is_valid(item, ctx))
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
        instance: &'i Value,
        location: &LazyLocation,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Array(items) = instance {
            let mut matches = 0;
            for item in items {
                if self
                    .node
                    .validators()
                    .all(|validator| validator.is_valid(item, ctx))
                {
                    matches += 1;
                    if matches > self.max_contains {
                        return Err(ValidationError::contains(
                            self.node.location().clone(),
                            location.into(),
                            instance,
                        ));
                    }
                }
            }
            if matches > 0 {
                Ok(())
            } else {
                Err(ValidationError::contains(
                    self.node.location().clone(),
                    location.into(),
                    instance,
                ))
            }
        } else {
            Ok(())
        }
    }

    fn trace(
        &self,
        instance: &Value,
        instance_path: &LazyLocation,
        callback: crate::tracing::TracingCallback<'_>,
        ctx: &mut ValidationContext,
    ) -> bool {
        if let Value::Array(items) = instance {
            let mut match_count = 0u64;
            for (idx, item) in items.iter().enumerate() {
                let path = instance_path.push(idx);
                if self.node.trace(item, &path, callback, ctx) {
                    match_count += 1;
                }
            }
            // Trace contains schema result
            let contains_valid = match_count >= 1;
            crate::tracing::TracingContext::new(instance_path, self.schema_path(), contains_valid)
                .call(callback);
            // Trace maxContains constraint
            let max_valid = match_count <= self.max_contains;
            crate::tracing::TracingContext::new(
                instance_path,
                &self.max_contains_location,
                max_valid,
            )
            .call(callback);
            contains_valid && max_valid
        } else {
            crate::tracing::TracingContext::new(instance_path, self.schema_path(), None)
                .call(callback);
            crate::tracing::TracingContext::new(instance_path, &self.max_contains_location, None)
                .call(callback);
            true
        }
    }
}

/// `maxContains` & `minContains` validation combined.
///
/// Docs:
///   `maxContains` - <https://json-schema.org/draft/2019-09/json-schema-validation.html#rfc.section.6.4.4>
///   `minContains` - <https://json-schema.org/draft/2019-09/json-schema-validation.html#rfc.section.6.4.5>
pub(crate) struct MinMaxContainsValidator {
    node: SchemaNode,
    min_contains: u64,
    max_contains: u64,
    min_contains_location: crate::paths::Location,
    max_contains_location: crate::paths::Location,
}

impl MinMaxContainsValidator {
    #[inline]
    pub(crate) fn compile<'a>(
        ctx: &compiler::Context,
        schema: &'a Value,
        min_contains: u64,
        max_contains: u64,
    ) -> CompilationResult<'a> {
        let min_contains_location = ctx.new_at_location("minContains").location().clone();
        let max_contains_location = ctx.new_at_location("maxContains").location().clone();
        let ctx = ctx.new_at_location("contains");
        Ok(Box::new(MinMaxContainsValidator {
            node: compiler::compile(&ctx, ctx.as_resource_ref(schema))?,
            min_contains,
            max_contains,
            min_contains_location,
            max_contains_location,
        }))
    }
}

impl Validate for MinMaxContainsValidator {
    fn schema_path(&self) -> &crate::paths::Location {
        self.node.location()
    }

    fn matches_type(&self, instance: &Value) -> bool {
        matches!(instance, Value::Array(_))
    }

    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        if let Value::Array(items) = instance {
            let mut matches = 0;
            for item in items {
                if self
                    .node
                    .validators()
                    .all(|validator| validator.is_valid(item, ctx))
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
        instance: &'i Value,
        location: &LazyLocation,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Array(items) = instance {
            let mut matches = 0;
            for item in items {
                if self
                    .node
                    .validators()
                    .all(|validator| validator.is_valid(item, ctx))
                {
                    matches += 1;
                    if matches > self.max_contains {
                        return Err(ValidationError::contains(
                            self.node.location().join("maxContains"),
                            location.into(),
                            instance,
                        ));
                    }
                }
            }
            if matches < self.min_contains {
                Err(ValidationError::contains(
                    self.node.location().join("minContains"),
                    location.into(),
                    instance,
                ))
            } else {
                Ok(())
            }
        } else {
            Ok(())
        }
    }

    fn trace(
        &self,
        instance: &Value,
        instance_path: &LazyLocation,
        callback: crate::tracing::TracingCallback<'_>,
        ctx: &mut ValidationContext,
    ) -> bool {
        if let Value::Array(items) = instance {
            let mut match_count = 0u64;
            for (idx, item) in items.iter().enumerate() {
                let path = instance_path.push(idx);
                if self.node.trace(item, &path, callback, ctx) {
                    match_count += 1;
                }
            }
            // Trace contains schema result
            let contains_valid = match_count >= 1;
            crate::tracing::TracingContext::new(instance_path, self.schema_path(), contains_valid)
                .call(callback);
            // Trace minContains constraint
            let min_valid = match_count >= self.min_contains;
            crate::tracing::TracingContext::new(
                instance_path,
                &self.min_contains_location,
                min_valid,
            )
            .call(callback);
            // Trace maxContains constraint
            let max_valid = match_count <= self.max_contains;
            crate::tracing::TracingContext::new(
                instance_path,
                &self.max_contains_location,
                max_valid,
            )
            .call(callback);
            min_valid && max_valid
        } else {
            crate::tracing::TracingContext::new(instance_path, self.schema_path(), None)
                .call(callback);
            crate::tracing::TracingContext::new(instance_path, &self.min_contains_location, None)
                .call(callback);
            crate::tracing::TracingContext::new(instance_path, &self.max_contains_location, None)
                .call(callback);
            true
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
