//! # Description
//! This module contains various validators for the `additionalProperties` keyword.
//!
//! The goal here is to compute intersections with another keywords affecting properties validation:
//!   - `properties`
//!   - `patternProperties`
//!
//! Each valid combination of these keywords has a validator here.
use crate::{
    compiler,
    error::{no_error, ErrorIterator, ValidationError},
    evaluation::{format_schema_location, Annotations, ErrorDescription, EvaluationNode},
    keywords::CompilationResult,
    node::SchemaNode,
    options::PatternEngineOptions,
    paths::{LazyLocation, Location},
    properties::{
        are_properties_valid, compile_big_map, compile_dynamic_prop_map_validator,
        compile_fancy_regex_patterns, compile_regex_patterns, compile_small_map, BigValidatorsMap,
        PropertiesValidatorsMap, SmallValidatorsMap,
    },
    regex::RegexEngine,
    types::JsonType,
    validator::{EvaluationResult, Validate, ValidationContext},
};
use referencing::Uri;
use serde_json::{Map, Value};
use std::sync::Arc;

/// # Schema example
///
/// ```json
/// {
///     "additionalProperties": {"type": "integer"},
/// }
/// ```
///
/// # Valid value
///
/// ```json
/// {
///     "bar": 6
/// }
/// ```
pub(crate) struct AdditionalPropertiesValidator {
    node: SchemaNode,
}
impl AdditionalPropertiesValidator {
    #[inline]
    pub(crate) fn compile<'a>(schema: &'a Value, ctx: &compiler::Context) -> CompilationResult<'a> {
        let ctx = ctx.new_at_location("additionalProperties");
        Ok(Box::new(AdditionalPropertiesValidator {
            node: compiler::compile(&ctx, ctx.as_resource_ref(schema))?,
        }))
    }
}
impl Validate for AdditionalPropertiesValidator {
    fn schema_path(&self) -> &Location {
        self.node.location()
    }

    fn matches_type(&self, instance: &Value) -> bool {
        matches!(instance, Value::Object(_))
    }

    fn trace(
        &self,
        instance: &Value,
        location: &LazyLocation,
        callback: crate::tracing::TracingCallback<'_>,
        ctx: &mut ValidationContext,
    ) -> bool {
        if let Value::Object(item) = instance {
            let mut is_valid = true;
            for (name, value) in item {
                is_valid &= self.node.trace(value, &location.push(name), callback, ctx);
            }
            let rv = if item.is_empty() {
                None
            } else {
                Some(is_valid)
            };
            crate::tracing::TracingContext::new(location, self.schema_path(), rv).call(callback);
            is_valid
        } else {
            crate::tracing::TracingContext::new(location, self.schema_path(), None).call(callback);
            true
        }
    }

    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        if let Value::Object(item) = instance {
            item.values().all(|i| self.node.is_valid(i, ctx))
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
        if let Value::Object(item) = instance {
            for (name, value) in item {
                self.node.validate(value, &location.push(name), ctx)?;
            }
        }
        Ok(())
    }

    fn iter_errors<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Value::Object(item) = instance {
            let mut errors = Vec::new();
            for (name, value) in item {
                errors.extend(
                    self.node
                        .iter_errors(value, &location.push(name.as_str()), ctx),
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
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Value::Object(item) = instance {
            let mut children = Vec::with_capacity(item.len());
            for (name, value) in item {
                children.push(self.node.evaluate_instance(
                    value,
                    &location.push(name.as_str()),
                    ctx,
                ));
            }
            let mut result = EvaluationResult::from_children(children);
            let annotated_props = item
                .keys()
                .cloned()
                .map(serde_json::Value::String)
                .collect();
            result.annotate(Annotations::new(serde_json::Value::Array(annotated_props)));
            result
        } else {
            EvaluationResult::valid_empty()
        }
    }
}

/// # Schema example
///
/// ```json
/// {
///     "additionalProperties": false
/// }
/// ```
///
/// # Valid value
///
/// ```json
/// {}
/// ```
pub(crate) struct AdditionalPropertiesFalseValidator {
    location: Location,
}
impl AdditionalPropertiesFalseValidator {
    #[inline]
    pub(crate) fn compile<'a>(location: Location) -> CompilationResult<'a> {
        Ok(Box::new(AdditionalPropertiesFalseValidator { location }))
    }
}
impl Validate for AdditionalPropertiesFalseValidator {
    fn schema_path(&self) -> &Location {
        &self.location
    }

    fn matches_type(&self, instance: &Value) -> bool {
        matches!(instance, Value::Object(_))
    }

    fn is_valid(&self, instance: &Value, _ctx: &mut ValidationContext) -> bool {
        if let Value::Object(item) = instance {
            item.iter().next().is_none()
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        _ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Object(item) = instance {
            if let Some((_, value)) = item.iter().next() {
                return Err(ValidationError::false_schema(
                    self.location.clone(),
                    location.into(),
                    value,
                ));
            }
        }
        Ok(())
    }
}

/// # Schema example
///
/// ```json
/// {
///     "additionalProperties": false,
///     "properties": {
///         "foo": {"type": "string"}
///     },
/// }
/// ```
///
/// # Valid value
///
/// ```json
/// {
///     "foo": "bar",
/// }
/// ```
pub(crate) struct AdditionalPropertiesNotEmptyFalseValidator<M: PropertiesValidatorsMap> {
    properties: M,
    properties_location: Location,
    additional_properties_location: Location,
}
impl AdditionalPropertiesNotEmptyFalseValidator<SmallValidatorsMap> {
    #[inline]
    pub(crate) fn compile<'a>(
        map: &'a Map<String, Value>,
        ctx: &compiler::Context,
    ) -> CompilationResult<'a> {
        Ok(Box::new(AdditionalPropertiesNotEmptyFalseValidator {
            properties: compile_small_map(ctx, map)?,
            properties_location: ctx.location().join("properties"),
            additional_properties_location: ctx.location().join("additionalProperties"),
        }))
    }
}
impl AdditionalPropertiesNotEmptyFalseValidator<BigValidatorsMap> {
    #[inline]
    pub(crate) fn compile<'a>(
        map: &'a Map<String, Value>,
        ctx: &compiler::Context,
    ) -> CompilationResult<'a> {
        Ok(Box::new(AdditionalPropertiesNotEmptyFalseValidator {
            properties: compile_big_map(ctx, map)?,
            properties_location: ctx.location().join("properties"),
            additional_properties_location: ctx.location().join("additionalProperties"),
        }))
    }
}
impl<M: PropertiesValidatorsMap> Validate for AdditionalPropertiesNotEmptyFalseValidator<M> {
    fn schema_path(&self) -> &Location {
        &self.additional_properties_location
    }

    fn matches_type(&self, instance: &Value) -> bool {
        matches!(instance, Value::Object(_))
    }

    fn trace(
        &self,
        instance: &Value,
        location: &LazyLocation,
        callback: crate::tracing::TracingCallback<'_>,
        ctx: &mut ValidationContext,
    ) -> bool {
        if let Value::Object(item) = instance {
            let mut properties_result: Option<bool> = None;
            let mut has_unexpected_properties = false;

            for (property, value) in item {
                let property_path = location.push(property);
                if let Some(node) = self.properties.get_validator(property) {
                    let schema_is_valid = node.trace(value, &property_path, callback, ctx);
                    crate::tracing::TracingContext::new(
                        &property_path,
                        node.schema_path(),
                        schema_is_valid,
                    )
                    .call(callback);
                    properties_result =
                        Some(properties_result.map_or(schema_is_valid, |c| c && schema_is_valid));
                } else {
                    has_unexpected_properties = true;
                }
            }

            crate::tracing::TracingContext::new(
                location,
                &self.properties_location,
                properties_result,
            )
            .call(callback);

            let additional_props_valid = !has_unexpected_properties;
            crate::tracing::TracingContext::new(
                location,
                &self.additional_properties_location,
                additional_props_valid,
            )
            .call(callback);
            properties_result.unwrap_or(true) && additional_props_valid
        } else {
            crate::tracing::TracingContext::new(location, self.schema_path(), None).call(callback);
            true
        }
    }

    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        if let Value::Object(props) = instance {
            are_properties_valid(&self.properties, props, ctx, |_, _| false)
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
        if let Value::Object(item) = instance {
            for (property, value) in item {
                if let Some((name, node)) = self.properties.get_key_validator(property) {
                    node.validate(value, &location.push(name), ctx)?;
                } else {
                    return Err(ValidationError::additional_properties(
                        self.additional_properties_location.clone(),
                        location.into(),
                        instance,
                        vec![property.clone()],
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
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Value::Object(item) = instance {
            let mut errors = vec![];
            let mut unexpected = vec![];
            for (property, value) in item {
                if let Some((name, node)) = self.properties.get_key_validator(property) {
                    errors.extend(node.iter_errors(value, &location.push(name.as_str()), ctx));
                } else {
                    unexpected.push(property.clone());
                }
            }
            if !unexpected.is_empty() {
                errors.push(ValidationError::additional_properties(
                    self.additional_properties_location.clone(),
                    location.into(),
                    instance,
                    unexpected,
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
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Value::Object(item) = instance {
            let mut unexpected = Vec::with_capacity(item.len());
            let mut children = Vec::with_capacity(item.len());
            for (property, value) in item {
                if let Some((_name, node)) = self.properties.get_key_validator(property) {
                    children.push(node.evaluate_instance(
                        value,
                        &location.push(property.as_str()),
                        ctx,
                    ));
                } else {
                    unexpected.push(property.clone());
                }
            }
            let mut result = EvaluationResult::from_children(children);
            if !unexpected.is_empty() {
                result.mark_errored(ErrorDescription::from_validation_error(
                    &ValidationError::additional_properties(
                        self.additional_properties_location.clone(),
                        location.into(),
                        instance,
                        unexpected,
                    ),
                ));
            }
            result
        } else {
            EvaluationResult::valid_empty()
        }
    }
}

/// # Schema example
///
/// ```json
/// {
///     "additionalProperties": {"type": "integer"},
///     "properties": {
///         "foo": {"type": "string"}
///     }
/// }
/// ```
///
/// # Valid value
///
/// ```json
/// {
///     "foo": "bar",
///     "bar": 6
/// }
/// ```
pub(crate) struct AdditionalPropertiesNotEmptyValidator<M: PropertiesValidatorsMap> {
    node: SchemaNode,
    properties: M,
    properties_location: Location,
}
impl AdditionalPropertiesNotEmptyValidator<SmallValidatorsMap> {
    #[inline]
    pub(crate) fn compile<'a>(
        map: &'a Map<String, Value>,
        ctx: &compiler::Context,
        schema: &'a Value,
    ) -> CompilationResult<'a> {
        let kctx = ctx.new_at_location("additionalProperties");
        Ok(Box::new(AdditionalPropertiesNotEmptyValidator {
            properties: compile_small_map(ctx, map)?,
            properties_location: ctx.location().join("properties"),
            node: compiler::compile(&kctx, kctx.as_resource_ref(schema))?,
        }))
    }
}
impl AdditionalPropertiesNotEmptyValidator<BigValidatorsMap> {
    #[inline]
    pub(crate) fn compile<'a>(
        map: &'a Map<String, Value>,
        ctx: &compiler::Context,
        schema: &'a Value,
    ) -> CompilationResult<'a> {
        let kctx = ctx.new_at_location("additionalProperties");
        Ok(Box::new(AdditionalPropertiesNotEmptyValidator {
            properties: compile_big_map(ctx, map)?,
            properties_location: ctx.location().join("properties"),
            node: compiler::compile(&kctx, kctx.as_resource_ref(schema))?,
        }))
    }
}
impl<M: PropertiesValidatorsMap> Validate for AdditionalPropertiesNotEmptyValidator<M> {
    fn schema_path(&self) -> &Location {
        self.node.location()
    }

    fn matches_type(&self, instance: &Value) -> bool {
        matches!(instance, Value::Object(_))
    }

    fn trace(
        &self,
        instance: &Value,
        location: &LazyLocation,
        callback: crate::tracing::TracingCallback<'_>,
        ctx: &mut ValidationContext,
    ) -> bool {
        if let Value::Object(item) = instance {
            let mut properties_result: Option<bool> = None;
            let mut additional_props_result: Option<bool> = None;

            for (property, value) in item {
                let property_path = location.push(property);
                if let Some(node) = self.properties.get_validator(property) {
                    let schema_is_valid = node.trace(value, &property_path, callback, ctx);
                    crate::tracing::TracingContext::new(
                        &property_path,
                        node.schema_path(),
                        schema_is_valid,
                    )
                    .call(callback);
                    properties_result =
                        Some(properties_result.map_or(schema_is_valid, |c| c && schema_is_valid));
                } else {
                    let schema_is_valid = self.node.trace(value, &property_path, callback, ctx);
                    additional_props_result = Some(
                        additional_props_result.map_or(schema_is_valid, |c| c && schema_is_valid),
                    );
                }
            }

            crate::tracing::TracingContext::new(
                location,
                &self.properties_location,
                properties_result,
            )
            .call(callback);
            crate::tracing::TracingContext::new(
                location,
                self.node.schema_path(),
                additional_props_result,
            )
            .call(callback);

            properties_result.unwrap_or(true) && additional_props_result.unwrap_or(true)
        } else {
            crate::tracing::TracingContext::new(location, self.schema_path(), None).call(callback);
            true
        }
    }

    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        if let Value::Object(props) = instance {
            are_properties_valid(&self.properties, props, ctx, |instance, ctx| {
                self.node.is_valid(instance, ctx)
            })
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
        if let Value::Object(props) = instance {
            for (property, value) in props {
                let property_location = location.push(property);
                if let Some(validator) = self.properties.get_validator(property) {
                    validator.validate(value, &property_location, ctx)?;
                } else {
                    self.node.validate(value, &property_location, ctx)?;
                }
            }
        }
        Ok(())
    }

    fn iter_errors<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Value::Object(map) = instance {
            let mut errors = vec![];
            for (property, value) in map {
                if let Some((name, property_validators)) =
                    self.properties.get_key_validator(property)
                {
                    errors.extend(property_validators.iter_errors(
                        value,
                        &location.push(name.as_str()),
                        ctx,
                    ));
                } else {
                    errors.extend(self.node.iter_errors(
                        value,
                        &location.push(property.as_str()),
                        ctx,
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
        instance: &Value,
        location: &LazyLocation,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Value::Object(map) = instance {
            let mut matched_propnames = Vec::with_capacity(map.len());
            let mut children = Vec::with_capacity(map.len());
            for (property, value) in map {
                let path = location.push(property.as_str());
                if let Some((_name, property_validators)) =
                    self.properties.get_key_validator(property)
                {
                    children.push(property_validators.evaluate_instance(value, &path, ctx));
                } else {
                    children.push(self.node.evaluate_instance(value, &path, ctx));
                    matched_propnames.push(property.clone());
                }
            }
            let mut result = EvaluationResult::from_children(children);
            if !matched_propnames.is_empty() {
                result.annotate(Annotations::new(Value::from(matched_propnames)));
            }
            result
        } else {
            EvaluationResult::valid_empty()
        }
    }
}

/// # Schema example
///
/// ```json
/// {
///     "additionalProperties": {"type": "integer"},
///     "patternProperties": {
///         "^x-": {"type": "integer", "minimum": 5},
///         "-x$": {"type": "integer", "maximum": 10}
///     }
/// }
/// ```
///
/// # Valid value
///
/// ```json
/// {
///     "x-foo": 6,
///     "foo-x": 7,
///     "bar": 8
/// }
/// ```
pub(crate) struct AdditionalPropertiesWithPatternsValidator<R> {
    node: SchemaNode,
    patterns: Vec<(R, SchemaNode)>,
    /// We need this because `compiler::compile` uses the additionalProperties keyword to compile
    /// this validator. That means that the schema node which contains this validator has
    /// "additionalProperties" as it's path. However, we need to produce annotations which have the
    /// patternProperties keyword as their path so we store the paths here.
    pattern_keyword_path: Location,
    pattern_keyword_absolute_location: Option<Arc<Uri<String>>>,
}

impl<R: RegexEngine> Validate for AdditionalPropertiesWithPatternsValidator<R> {
    fn schema_path(&self) -> &Location {
        self.node.location()
    }

    fn matches_type(&self, instance: &Value) -> bool {
        matches!(instance, Value::Object(_))
    }

    fn trace(
        &self,
        instance: &Value,
        location: &LazyLocation,
        callback: crate::tracing::TracingCallback<'_>,
        ctx: &mut ValidationContext,
    ) -> bool {
        if let Value::Object(item) = instance {
            let mut pattern_props_results: Option<bool> = None;
            let mut additional_props_results: Option<bool> = None;

            for (property, value) in item {
                let property_path = location.push(property);
                let mut has_pattern_match = false;

                for (re, node) in &self.patterns {
                    if re.is_match(property).unwrap_or(false) {
                        has_pattern_match = true;
                        let schema_is_valid = node.trace(value, &property_path, callback, ctx);
                        crate::tracing::TracingContext::new(
                            &property_path,
                            node.schema_path(),
                            schema_is_valid,
                        )
                        .call(callback);
                        pattern_props_results = Some(
                            pattern_props_results
                                .map_or(schema_is_valid, |prev| prev && schema_is_valid),
                        );
                    }
                }

                if !has_pattern_match {
                    let schema_is_valid = self.node.trace(value, &property_path, callback, ctx);
                    additional_props_results = Some(
                        additional_props_results
                            .map_or(schema_is_valid, |prev| prev && schema_is_valid),
                    );
                }
            }

            crate::tracing::TracingContext::new(
                location,
                &self.pattern_keyword_path,
                pattern_props_results,
            )
            .call(callback);
            crate::tracing::TracingContext::new(
                location,
                self.node.schema_path(),
                additional_props_results,
            )
            .call(callback);

            pattern_props_results.unwrap_or(true) && additional_props_results.unwrap_or(true)
        } else {
            crate::tracing::TracingContext::new(location, self.schema_path(), None).call(callback);
            true
        }
    }

    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        if let Value::Object(item) = instance {
            for (property, value) in item {
                let mut has_match = false;
                for (re, node) in &self.patterns {
                    if re.is_match(property).unwrap_or(false) {
                        has_match = true;
                        if !node.is_valid(value, ctx) {
                            return false;
                        }
                    }
                }
                if !has_match && !self.node.is_valid(value, ctx) {
                    return false;
                }
            }
        }
        true
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Object(item) = instance {
            for (property, value) in item {
                let property_location = location.push(property);
                let mut has_match = false;
                for (re, node) in &self.patterns {
                    if re.is_match(property).unwrap_or(false) {
                        has_match = true;
                        node.validate(value, &property_location, ctx)?;
                    }
                }
                if !has_match {
                    self.node.validate(value, &property_location, ctx)?;
                }
            }
        }
        Ok(())
    }

    fn iter_errors<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Value::Object(item) = instance {
            let mut errors = vec![];
            for (property, value) in item {
                let mut has_match = false;
                for (re, node) in &self.patterns {
                    if re.is_match(property).unwrap_or(false) {
                        has_match = true;
                        errors.extend(node.iter_errors(
                            value,
                            &location.push(property.as_str()),
                            ctx,
                        ));
                    }
                }
                if !has_match {
                    errors.extend(self.node.iter_errors(
                        value,
                        &location.push(property.as_str()),
                        ctx,
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
        instance: &Value,
        location: &LazyLocation,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Value::Object(item) = instance {
            let mut pattern_matched_propnames = Vec::with_capacity(item.len());
            let mut additional_matched_propnames = Vec::with_capacity(item.len());
            let mut children = Vec::with_capacity(item.len());
            for (property, value) in item {
                let path = location.push(property.as_str());
                let mut has_match = false;
                for (pattern, node) in &self.patterns {
                    if pattern.is_match(property).unwrap_or(false) {
                        has_match = true;
                        pattern_matched_propnames.push(property.clone());
                        children.push(node.evaluate_instance(value, &path, ctx));
                    }
                }
                if !has_match {
                    additional_matched_propnames.push(property.clone());
                    children.push(self.node.evaluate_instance(value, &path, ctx));
                }
            }
            if !pattern_matched_propnames.is_empty() {
                let annotation = Annotations::new(Value::from(pattern_matched_propnames));
                let schema_location = format_schema_location(
                    &self.pattern_keyword_path,
                    self.pattern_keyword_absolute_location.as_ref(),
                );
                children.push(EvaluationNode::valid(
                    self.pattern_keyword_path.clone(),
                    self.pattern_keyword_absolute_location.clone(),
                    schema_location,
                    location.into(),
                    Some(annotation),
                    Vec::new(),
                ));
            }
            let mut result = EvaluationResult::from_children(children);
            if !additional_matched_propnames.is_empty() {
                result.annotate(Annotations::new(Value::from(additional_matched_propnames)));
            }
            result
        } else {
            EvaluationResult::valid_empty()
        }
    }
}

/// # Schema example
///
/// ```json
/// {
///     "additionalProperties": false,
///     "patternProperties": {
///         "^x-": {"type": "integer", "minimum": 5},
///         "-x$": {"type": "integer", "maximum": 10}
///     }
/// }
/// ```
///
/// # Valid value
///
/// ```json
/// {
///     "x-bar": 6,
///     "spam-x": 7,
///     "x-baz-x": 8,
/// }
/// ```
pub(crate) struct AdditionalPropertiesWithPatternsFalseValidator<R> {
    patterns: Vec<(R, SchemaNode)>,
    location: Location,
    pattern_keyword_path: Location,
    pattern_keyword_absolute_location: Option<Arc<Uri<String>>>,
}

impl<R: RegexEngine> Validate for AdditionalPropertiesWithPatternsFalseValidator<R> {
    fn schema_path(&self) -> &Location {
        &self.location
    }

    fn matches_type(&self, instance: &Value) -> bool {
        matches!(instance, Value::Object(_))
    }

    fn trace(
        &self,
        instance: &Value,
        location: &LazyLocation,
        callback: crate::tracing::TracingCallback<'_>,
        ctx: &mut ValidationContext,
    ) -> bool {
        if let Value::Object(item) = instance {
            let mut pattern_props_results: Option<bool> = None;
            let mut has_unexpected_properties = false;

            for (property, value) in item {
                let property_path = location.push(property);
                let mut has_pattern_match = false;

                for (re, node) in &self.patterns {
                    if re.is_match(property).unwrap_or(false) {
                        has_pattern_match = true;
                        let schema_is_valid = node.trace(value, &property_path, callback, ctx);
                        crate::tracing::TracingContext::new(
                            &property_path,
                            node.schema_path(),
                            schema_is_valid,
                        )
                        .call(callback);
                        pattern_props_results = Some(
                            pattern_props_results
                                .map_or(schema_is_valid, |prev| prev && schema_is_valid),
                        );
                    }
                }

                if !has_pattern_match {
                    has_unexpected_properties = true;
                }
            }

            crate::tracing::TracingContext::new(
                location,
                &self.pattern_keyword_path,
                pattern_props_results,
            )
            .call(callback);

            let additional_props_valid = !has_unexpected_properties;
            crate::tracing::TracingContext::new(
                location,
                &self.location,
                Some(additional_props_valid),
            )
            .call(callback);

            pattern_props_results.unwrap_or(true) && additional_props_valid
        } else {
            crate::tracing::TracingContext::new(location, self.schema_path(), None).call(callback);
            true
        }
    }

    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        if let Value::Object(item) = instance {
            for (property, value) in item {
                let mut has_match = false;
                for (re, node) in &self.patterns {
                    if re.is_match(property).unwrap_or(false) {
                        has_match = true;
                        if !node.is_valid(value, ctx) {
                            return false;
                        }
                    }
                }
                if !has_match {
                    return false;
                }
            }
        }
        true
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Object(item) = instance {
            for (property, value) in item {
                let property_location = location.push(property);
                let mut has_match = false;
                for (re, node) in &self.patterns {
                    if re.is_match(property).unwrap_or(false) {
                        has_match = true;
                        node.validate(value, &property_location, ctx)?;
                    }
                }
                if !has_match {
                    return Err(ValidationError::additional_properties(
                        self.location.clone(),
                        location.into(),
                        instance,
                        vec![property.clone()],
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
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Value::Object(item) = instance {
            let mut errors = vec![];
            let mut unexpected = vec![];
            for (property, value) in item {
                let mut has_match = false;
                for (re, node) in &self.patterns {
                    if re.is_match(property).unwrap_or(false) {
                        has_match = true;
                        errors.extend(node.iter_errors(
                            value,
                            &location.push(property.as_str()),
                            ctx,
                        ));
                    }
                }
                if !has_match {
                    unexpected.push(property.clone());
                }
            }
            if !unexpected.is_empty() {
                errors.push(ValidationError::additional_properties(
                    self.location.clone(),
                    location.into(),
                    instance,
                    unexpected,
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
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Value::Object(item) = instance {
            let mut unexpected = Vec::with_capacity(item.len());
            let mut pattern_matched_props = Vec::with_capacity(item.len());
            let mut children = Vec::with_capacity(item.len());
            for (property, value) in item {
                let path = location.push(property.as_str());
                let mut has_match = false;
                for (pattern, node) in &self.patterns {
                    if pattern.is_match(property).unwrap_or(false) {
                        has_match = true;
                        pattern_matched_props.push(property.clone());
                        children.push(node.evaluate_instance(value, &path, ctx));
                    }
                }
                if !has_match {
                    unexpected.push(property.clone());
                }
            }
            if !pattern_matched_props.is_empty() {
                let annotation = Annotations::new(Value::from(pattern_matched_props));
                let schema_location = format_schema_location(
                    &self.pattern_keyword_path,
                    self.pattern_keyword_absolute_location.as_ref(),
                );
                children.push(EvaluationNode::valid(
                    self.pattern_keyword_path.clone(),
                    self.pattern_keyword_absolute_location.clone(),
                    schema_location,
                    location.into(),
                    Some(annotation),
                    Vec::new(),
                ));
            }
            let mut result = EvaluationResult::from_children(children);
            if !unexpected.is_empty() {
                result.mark_errored(ErrorDescription::from_validation_error(
                    &ValidationError::additional_properties(
                        self.location.clone(),
                        location.into(),
                        instance,
                        unexpected,
                    ),
                ));
            }
            result
        } else {
            EvaluationResult::valid_empty()
        }
    }
}

/// # Schema example
///
/// ```json
/// {
///     "additionalProperties": {"type": "integer"},
///     "properties": {
///         "foo": {"type": "string"}
///     },
///     "patternProperties": {
///         "^x-": {"type": "integer", "minimum": 5},
///         "-x$": {"type": "integer", "maximum": 10}
///     }
/// }
/// ```
///
/// # Valid value
///
/// ```json
/// {
///     "foo": "a",
///     "x-spam": 6,
///     "spam-x": 7,
///     "x-spam-x": 8,
///     "bar": 42
/// }
/// ```
pub(crate) struct AdditionalPropertiesWithPatternsNotEmptyValidator<M: PropertiesValidatorsMap, R> {
    node: SchemaNode,
    properties: M,
    patterns: Vec<(R, SchemaNode)>,
    pattern_properties_location: Location,
    properties_location: Location,
}

impl<M: PropertiesValidatorsMap, R: RegexEngine> Validate
    for AdditionalPropertiesWithPatternsNotEmptyValidator<M, R>
{
    fn schema_path(&self) -> &Location {
        self.node.location()
    }

    fn matches_type(&self, instance: &Value) -> bool {
        matches!(instance, Value::Object(_))
    }

    fn trace(
        &self,
        instance: &Value,
        location: &LazyLocation,
        callback: crate::tracing::TracingCallback<'_>,
        ctx: &mut ValidationContext,
    ) -> bool {
        if let Value::Object(item) = instance {
            let mut properties_result: Option<bool> = None;
            let mut pattern_props_result: Option<bool> = None;
            let mut additional_props_result: Option<bool> = None;

            for (property, value) in item {
                let property_path = location.push(property);

                if let Some(node) = self.properties.get_validator(property) {
                    let prop_valid = node.trace(value, &property_path, callback, ctx);
                    crate::tracing::TracingContext::new(
                        &property_path,
                        node.schema_path(),
                        prop_valid,
                    )
                    .call(callback);
                    properties_result =
                        Some(properties_result.map_or(prop_valid, |prev| prev && prop_valid));

                    for (re, pattern_node) in &self.patterns {
                        if re.is_match(property).unwrap_or(false) {
                            let pattern_valid =
                                pattern_node.trace(value, &property_path, callback, ctx);
                            crate::tracing::TracingContext::new(
                                &property_path,
                                pattern_node.schema_path(),
                                pattern_valid,
                            )
                            .call(callback);
                            pattern_props_result = Some(
                                pattern_props_result
                                    .map_or(pattern_valid, |prev| prev && pattern_valid),
                            );
                        }
                    }
                } else {
                    let mut pattern_matched = false;
                    for (re, node) in &self.patterns {
                        if re.is_match(property).unwrap_or(false) {
                            pattern_matched = true;
                            let pattern_valid = node.trace(value, &property_path, callback, ctx);
                            crate::tracing::TracingContext::new(
                                &property_path,
                                node.schema_path(),
                                pattern_valid,
                            )
                            .call(callback);
                            pattern_props_result = Some(
                                pattern_props_result
                                    .map_or(pattern_valid, |prev| prev && pattern_valid),
                            );
                        }
                    }

                    if !pattern_matched {
                        let additional_valid =
                            self.node.trace(value, &property_path, callback, ctx);
                        additional_props_result = Some(
                            additional_props_result
                                .map_or(additional_valid, |prev| prev && additional_valid),
                        );
                    }
                }
            }

            crate::tracing::TracingContext::new(
                location,
                &self.properties_location,
                properties_result,
            )
            .call(callback);
            crate::tracing::TracingContext::new(
                location,
                &self.pattern_properties_location,
                pattern_props_result,
            )
            .call(callback);
            crate::tracing::TracingContext::new(
                location,
                self.node.schema_path(),
                additional_props_result,
            )
            .call(callback);

            properties_result.unwrap_or(true)
                && pattern_props_result.unwrap_or(true)
                && additional_props_result.unwrap_or(true)
        } else {
            crate::tracing::TracingContext::new(location, self.schema_path(), None).call(callback);
            true
        }
    }

    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        if let Value::Object(item) = instance {
            for (property, value) in item {
                if let Some(node) = self.properties.get_validator(property) {
                    if node.is_valid(value, ctx) {
                        for (re, pattern_node) in &self.patterns {
                            if re.is_match(property).unwrap_or(false)
                                && !pattern_node.is_valid(value, ctx)
                            {
                                return false;
                            }
                        }
                    } else {
                        return false;
                    }
                } else {
                    let mut has_match = false;
                    for (re, node) in &self.patterns {
                        if re.is_match(property).unwrap_or(false) {
                            has_match = true;
                            if !node.is_valid(value, ctx) {
                                return false;
                            }
                        }
                    }
                    if !has_match && !self.node.is_valid(value, ctx) {
                        return false;
                    }
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
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Object(item) = instance {
            for (property, value) in item {
                if let Some((name, node)) = self.properties.get_key_validator(property) {
                    let name_location = location.push(name);
                    node.validate(value, &name_location, ctx)?;
                    for (re, pattern_node) in &self.patterns {
                        if re.is_match(property).unwrap_or(false) {
                            pattern_node.validate(value, &name_location, ctx)?;
                        }
                    }
                } else {
                    let property_location = location.push(property);
                    let mut has_match = false;
                    for (re, node) in &self.patterns {
                        if re.is_match(property).unwrap_or(false) {
                            has_match = true;
                            node.validate(value, &property_location, ctx)?;
                        }
                    }
                    if !has_match {
                        self.node.validate(value, &property_location, ctx)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn iter_errors<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Value::Object(item) = instance {
            let mut errors = vec![];
            for (property, value) in item {
                if let Some((name, node)) = self.properties.get_key_validator(property) {
                    errors.extend(node.iter_errors(value, &location.push(name.as_str()), ctx));
                    for (re, pattern_node) in &self.patterns {
                        if re.is_match(property).unwrap_or(false) {
                            errors.extend(pattern_node.iter_errors(
                                value,
                                &location.push(name.as_str()),
                                ctx,
                            ));
                        }
                    }
                } else {
                    let mut has_match = false;
                    for (re, node) in &self.patterns {
                        if re.is_match(property).unwrap_or(false) {
                            has_match = true;
                            errors.extend(node.iter_errors(
                                value,
                                &location.push(property.as_str()),
                                ctx,
                            ));
                        }
                    }
                    if !has_match {
                        errors.extend(self.node.iter_errors(
                            value,
                            &location.push(property.as_str()),
                            ctx,
                        ));
                    }
                }
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
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Value::Object(item) = instance {
            let mut additional_matches = Vec::with_capacity(item.len());
            let mut children = Vec::with_capacity(item.len());
            for (property, value) in item {
                let path = location.push(property.as_str());
                if let Some((_name, node)) = self.properties.get_key_validator(property) {
                    children.push(node.evaluate_instance(value, &path, ctx));
                    for (pattern, pattern_node) in &self.patterns {
                        if pattern.is_match(property).unwrap_or(false) {
                            children.push(pattern_node.evaluate_instance(value, &path, ctx));
                        }
                    }
                } else {
                    let mut has_match = false;
                    for (pattern, node) in &self.patterns {
                        if pattern.is_match(property).unwrap_or(false) {
                            has_match = true;
                            children.push(node.evaluate_instance(value, &path, ctx));
                        }
                    }
                    if !has_match {
                        additional_matches.push(property.clone());
                        children.push(self.node.evaluate_instance(value, &path, ctx));
                    }
                }
            }
            let mut result = EvaluationResult::from_children(children);
            result.annotate(Annotations::new(Value::from(additional_matches)));
            result
        } else {
            EvaluationResult::valid_empty()
        }
    }
}

/// # Schema example
///
/// ```json
/// {
///     "additionalProperties": false,
///     "properties": {
///         "foo": {"type": "string"}
///     },
///     "patternProperties": {
///         "^x-": {"type": "integer", "minimum": 5},
///         "-x$": {"type": "integer", "maximum": 10}
///     }
/// }
/// ```
///
/// # Valid value
///
/// ```json
/// {
///     "foo": "bar",
///     "x-bar": 6,
///     "spam-x": 7,
///     "x-baz-x": 8,
/// }
/// ```
pub(crate) struct AdditionalPropertiesWithPatternsNotEmptyFalseValidator<
    M: PropertiesValidatorsMap,
    R,
> {
    properties: M,
    patterns: Vec<(R, SchemaNode)>,
    location: Location,
    properties_location: Location,
    pattern_properties_location: Location,
}

impl<M: PropertiesValidatorsMap, R: RegexEngine> Validate
    for AdditionalPropertiesWithPatternsNotEmptyFalseValidator<M, R>
{
    fn schema_path(&self) -> &Location {
        &self.location
    }

    fn matches_type(&self, instance: &Value) -> bool {
        matches!(instance, Value::Object(_))
    }

    fn trace(
        &self,
        instance: &Value,
        location: &LazyLocation,
        callback: crate::tracing::TracingCallback<'_>,
        ctx: &mut ValidationContext,
    ) -> bool {
        if let Value::Object(item) = instance {
            let mut properties_result: Option<bool> = None;
            let mut pattern_props_result: Option<bool> = None;
            let mut has_unexpected_properties = false;

            for (property, value) in item {
                let property_path = location.push(property);
                let mut is_known_or_pattern = false;

                if let Some(node) = self.properties.get_validator(property) {
                    is_known_or_pattern = true;
                    let prop_valid = node.trace(value, &property_path, callback, ctx);
                    crate::tracing::TracingContext::new(
                        &property_path,
                        node.schema_path(),
                        prop_valid,
                    )
                    .call(callback);
                    properties_result =
                        Some(properties_result.map_or(prop_valid, |prev| prev && prop_valid));

                    for (re, pattern_node) in &self.patterns {
                        if re.is_match(property).unwrap_or(false) {
                            let pattern_valid =
                                pattern_node.trace(value, &property_path, callback, ctx);
                            crate::tracing::TracingContext::new(
                                &property_path,
                                pattern_node.schema_path(),
                                pattern_valid,
                            )
                            .call(callback);
                            pattern_props_result = Some(
                                pattern_props_result
                                    .map_or(pattern_valid, |prev| prev && pattern_valid),
                            );
                        }
                    }
                } else {
                    for (re, node) in &self.patterns {
                        if re.is_match(property).unwrap_or(false) {
                            is_known_or_pattern = true;
                            let pattern_valid = node.trace(value, &property_path, callback, ctx);
                            crate::tracing::TracingContext::new(
                                &property_path,
                                node.schema_path(),
                                pattern_valid,
                            )
                            .call(callback);
                            pattern_props_result = Some(
                                pattern_props_result
                                    .map_or(pattern_valid, |prev| prev && pattern_valid),
                            );
                        }
                    }
                }

                if !is_known_or_pattern {
                    has_unexpected_properties = true;
                }
            }

            crate::tracing::TracingContext::new(
                location,
                &self.properties_location,
                properties_result,
            )
            .call(callback);
            crate::tracing::TracingContext::new(
                location,
                &self.pattern_properties_location,
                pattern_props_result,
            )
            .call(callback);

            let additional_props_valid = !has_unexpected_properties;
            crate::tracing::TracingContext::new(
                location,
                &self.location,
                Some(additional_props_valid),
            )
            .call(callback);

            properties_result.unwrap_or(true)
                && pattern_props_result.unwrap_or(true)
                && additional_props_valid
        } else {
            crate::tracing::TracingContext::new(location, self.schema_path(), None).call(callback);
            true
        }
    }

    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        if let Value::Object(item) = instance {
            for (property, value) in item {
                if let Some(node) = self.properties.get_validator(property) {
                    if node.is_valid(value, ctx) {
                        for (re, pattern_node) in &self.patterns {
                            if re.is_match(property).unwrap_or(false)
                                && !pattern_node.is_valid(value, ctx)
                            {
                                return false;
                            }
                        }
                    } else {
                        return false;
                    }
                } else {
                    let mut has_match = false;
                    for (re, node) in &self.patterns {
                        if re.is_match(property).unwrap_or(false) {
                            has_match = true;
                            if !node.is_valid(value, ctx) {
                                return false;
                            }
                        }
                    }
                    if !has_match {
                        return false;
                    }
                }
            }
        }
        true
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Object(item) = instance {
            for (property, value) in item {
                if let Some((name, node)) = self.properties.get_key_validator(property) {
                    let name_location = location.push(name);
                    node.validate(value, &name_location, ctx)?;
                    for (re, pattern_node) in &self.patterns {
                        if re.is_match(property).unwrap_or(false) {
                            pattern_node.validate(value, &name_location, ctx)?;
                        }
                    }
                } else {
                    let property_location = location.push(property);
                    let mut has_match = false;
                    for (re, node) in &self.patterns {
                        if re.is_match(property).unwrap_or(false) {
                            has_match = true;
                            node.validate(value, &property_location, ctx)?;
                        }
                    }
                    if !has_match {
                        return Err(ValidationError::additional_properties(
                            self.location.clone(),
                            location.into(),
                            instance,
                            vec![property.clone()],
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    fn iter_errors<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if let Value::Object(item) = instance {
            let mut errors = vec![];
            let mut unexpected = vec![];
            for (property, value) in item {
                if let Some((name, node)) = self.properties.get_key_validator(property) {
                    errors.extend(node.iter_errors(value, &location.push(name.as_str()), ctx));
                    for (re, pattern_node) in &self.patterns {
                        if re.is_match(property).unwrap_or(false) {
                            errors.extend(pattern_node.iter_errors(
                                value,
                                &location.push(name.as_str()),
                                ctx,
                            ));
                        }
                    }
                } else {
                    let mut has_match = false;
                    for (re, node) in &self.patterns {
                        if re.is_match(property).unwrap_or(false) {
                            has_match = true;
                            errors.extend(node.iter_errors(
                                value,
                                &location.push(property.as_str()),
                                ctx,
                            ));
                        }
                    }
                    if !has_match {
                        unexpected.push(property.clone());
                    }
                }
            }
            if !unexpected.is_empty() {
                errors.push(ValidationError::additional_properties(
                    self.location.clone(),
                    location.into(),
                    instance,
                    unexpected,
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
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Value::Object(item) = instance {
            let mut unexpected = vec![];
            let mut children = Vec::with_capacity(item.len());
            for (property, value) in item {
                let path = location.push(property.as_str());
                if let Some((_name, node)) = self.properties.get_key_validator(property) {
                    children.push(node.evaluate_instance(value, &path, ctx));
                    for (pattern, pattern_node) in &self.patterns {
                        if pattern.is_match(property).unwrap_or(false) {
                            children.push(pattern_node.evaluate_instance(value, &path, ctx));
                        }
                    }
                } else {
                    let mut has_match = false;
                    for (pattern, node) in &self.patterns {
                        if pattern.is_match(property).unwrap_or(false) {
                            has_match = true;
                            children.push(node.evaluate_instance(value, &path, ctx));
                        }
                    }
                    if !has_match {
                        unexpected.push(property.clone());
                    }
                }
            }
            let mut result = EvaluationResult::from_children(children);
            if !unexpected.is_empty() {
                result.mark_errored(ErrorDescription::from_validation_error(
                    &ValidationError::additional_properties(
                        self.location.clone(),
                        location.into(),
                        instance,
                        unexpected,
                    ),
                ));
            }
            result
        } else {
            EvaluationResult::valid_empty()
        }
    }
}

macro_rules! try_compile {
    ($expr:expr) => {
        match $expr {
            Ok(result) => result,
            Err(error) => return Some(Err(error)),
        }
    };
}

fn compile_pattern_non_empty<'a, R>(
    ctx: &compiler::Context,
    map: &'a Map<String, Value>,
    patterns: Vec<(R, SchemaNode)>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>>
where
    R: RegexEngine + 'static,
{
    let kctx = ctx.new_at_location("additionalProperties");
    if map.len() < 40 {
        Some(Ok(Box::new(
            AdditionalPropertiesWithPatternsNotEmptyValidator::<SmallValidatorsMap, R> {
                node: try_compile!(compiler::compile(&kctx, kctx.as_resource_ref(schema))),
                properties: try_compile!(compile_small_map(ctx, map)),
                patterns,
                pattern_properties_location: ctx.location().join("patternProperties"),
                properties_location: ctx.location().join("properties"),
            },
        )))
    } else {
        Some(Ok(Box::new(
            AdditionalPropertiesWithPatternsNotEmptyValidator::<BigValidatorsMap, R> {
                node: try_compile!(compiler::compile(&kctx, kctx.as_resource_ref(schema))),
                properties: try_compile!(compile_big_map(ctx, map)),
                patterns,
                pattern_properties_location: ctx.location().join("patternProperties"),
                properties_location: ctx.location().join("properties"),
            },
        )))
    }
}

fn compile_pattern_non_empty_false<'a, R>(
    ctx: &compiler::Context,
    map: &'a Map<String, Value>,
    patterns: Vec<(R, SchemaNode)>,
) -> Option<CompilationResult<'a>>
where
    R: RegexEngine + 'static,
{
    let kctx = ctx.new_at_location("additionalProperties");
    if map.len() < 40 {
        Some(Ok(Box::new(
            AdditionalPropertiesWithPatternsNotEmptyFalseValidator::<SmallValidatorsMap, R> {
                properties: try_compile!(compile_small_map(ctx, map)),
                patterns,
                location: kctx.location().clone(),
                properties_location: ctx.location().join("properties"),
                pattern_properties_location: ctx.location().join("patternProperties"),
            },
        )))
    } else {
        Some(Ok(Box::new(
            AdditionalPropertiesWithPatternsNotEmptyFalseValidator::<BigValidatorsMap, R> {
                properties: try_compile!(compile_big_map(ctx, map)),
                patterns,
                location: kctx.location().clone(),
                properties_location: ctx.location().join("properties"),
                pattern_properties_location: ctx.location().join("patternProperties"),
            },
        )))
    }
}

#[inline]
pub(crate) fn compile<'a>(
    ctx: &compiler::Context,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>> {
    let properties = parent.get("properties");
    if let Some(patterns) = parent.get("patternProperties") {
        if let Value::Object(obj) = patterns {
            // Compile all patterns & their validators to avoid doing work in the `patternProperties` validator
            match ctx.config().pattern_options() {
                PatternEngineOptions::FancyRegex { .. } => {
                    let patterns = match compile_fancy_regex_patterns(ctx, obj) {
                        Ok(patterns) => patterns,
                        Err(error) => return Some(Err(error)),
                    };
                    match schema {
                        Value::Bool(true) => None, // "additionalProperties" are "true" by default
                        Value::Bool(false) => {
                            if let Some(properties) = properties {
                                if let Value::Object(map) = properties {
                                    compile_pattern_non_empty_false::<fancy_regex::Regex>(
                                        ctx, map, patterns,
                                    )
                                } else {
                                    Some(Err(ValidationError::custom(
                                        Location::new(),
                                        Location::new(),
                                        properties,
                                        "Unexpected type",
                                    )))
                                }
                            } else {
                                Some(Ok(Box::new(
                                    AdditionalPropertiesWithPatternsFalseValidator {
                                        patterns,
                                        location: ctx.location().join("additionalProperties"),
                                        pattern_keyword_path: ctx
                                            .location()
                                            .join("patternProperties"),
                                        pattern_keyword_absolute_location: ctx
                                            .new_at_location("patternProperties")
                                            .base_uri(),
                                    },
                                )))
                            }
                        }
                        _ => {
                            if let Some(properties) = properties {
                                if let Value::Object(map) = properties {
                                    compile_pattern_non_empty::<fancy_regex::Regex>(
                                        ctx, map, patterns, schema,
                                    )
                                } else {
                                    Some(Err(ValidationError::custom(
                                        Location::new(),
                                        Location::new(),
                                        properties,
                                        "Unexpected type",
                                    )))
                                }
                            } else {
                                let kctx = ctx.new_at_location("additionalProperties");
                                Some(Ok(Box::new(AdditionalPropertiesWithPatternsValidator {
                                    node: try_compile!(compiler::compile(
                                        &kctx,
                                        kctx.as_resource_ref(schema),
                                    )),
                                    patterns,
                                    pattern_keyword_path: ctx.location().join("patternProperties"),
                                    pattern_keyword_absolute_location: ctx
                                        .new_at_location("patternProperties")
                                        .base_uri(),
                                })))
                            }
                        }
                    }
                }
                PatternEngineOptions::Regex { .. } => {
                    let patterns = match compile_regex_patterns(ctx, obj) {
                        Ok(patterns) => patterns,
                        Err(error) => return Some(Err(error)),
                    };
                    match schema {
                        Value::Bool(true) => None, // "additionalProperties" are "true" by default
                        Value::Bool(false) => {
                            if let Some(properties) = properties {
                                if let Value::Object(map) = properties {
                                    compile_pattern_non_empty_false::<regex::Regex>(
                                        ctx, map, patterns,
                                    )
                                } else {
                                    Some(Err(ValidationError::custom(
                                        Location::new(),
                                        Location::new(),
                                        properties,
                                        "Unexpected type",
                                    )))
                                }
                            } else {
                                Some(Ok(Box::new(
                                    AdditionalPropertiesWithPatternsFalseValidator {
                                        patterns,
                                        location: ctx.location().join("additionalProperties"),
                                        pattern_keyword_path: ctx
                                            .location()
                                            .join("patternProperties"),
                                        pattern_keyword_absolute_location: ctx
                                            .new_at_location("patternProperties")
                                            .base_uri(),
                                    },
                                )))
                            }
                        }
                        _ => {
                            if let Some(properties) = properties {
                                if let Value::Object(map) = properties {
                                    compile_pattern_non_empty::<regex::Regex>(
                                        ctx, map, patterns, schema,
                                    )
                                } else {
                                    Some(Err(ValidationError::custom(
                                        Location::new(),
                                        Location::new(),
                                        properties,
                                        "Unexpected type",
                                    )))
                                }
                            } else {
                                let kctx = ctx.new_at_location("additionalProperties");
                                Some(Ok(Box::new(AdditionalPropertiesWithPatternsValidator {
                                    node: try_compile!(compiler::compile(
                                        &kctx,
                                        kctx.as_resource_ref(schema),
                                    )),
                                    patterns,
                                    pattern_keyword_path: ctx.location().join("patternProperties"),
                                    pattern_keyword_absolute_location: ctx
                                        .new_at_location("patternProperties")
                                        .base_uri(),
                                })))
                            }
                        }
                    }
                }
            }
        } else {
            Some(Err(ValidationError::single_type_error(
                Location::new(),
                ctx.location().clone(),
                schema,
                JsonType::Object,
            )))
        }
    } else {
        match schema {
            Value::Bool(true) => None, // "additionalProperties" are "true" by default
            Value::Bool(false) => {
                if let Some(properties) = properties {
                    compile_dynamic_prop_map_validator!(
                        AdditionalPropertiesNotEmptyFalseValidator,
                        properties,
                        ctx,
                    )
                } else {
                    let location = ctx.location().join("additionalProperties");
                    Some(AdditionalPropertiesFalseValidator::compile(location))
                }
            }
            _ => {
                if let Some(properties) = properties {
                    compile_dynamic_prop_map_validator!(
                        AdditionalPropertiesNotEmptyValidator,
                        properties,
                        ctx,
                        schema,
                    )
                } else {
                    Some(AdditionalPropertiesValidator::compile(schema, ctx))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use serde_json::{json, Value};
    use test_case::test_case;

    fn schema_1() -> Value {
        // For `AdditionalPropertiesWithPatternsNotEmptyFalseValidator`
        json!({
            "additionalProperties": false,
            "properties": {
                "foo": {"type": "string"},
                "barbaz": {"type": "integer", "multipleOf": 3},
            },
            "patternProperties": {
                "^bar": {"type": "integer", "minimum": 5},
                "spam$": {"type": "integer", "maximum": 10},
            }
        })
    }

    // Another type
    #[test_case(&json!([1]))]
    // The right type
    #[test_case(&json!({}))]
    // Match `properties.foo`
    #[test_case(&json!({"foo": "a"}))]
    // Match `properties.barbaz` & `patternProperties.^bar`
    #[test_case(&json!({"barbaz": 6}))]
    // Match `patternProperties.^bar`
    #[test_case(&json!({"bar": 6}))]
    // Match `patternProperties.spam$`
    #[test_case(&json!({"spam": 7}))]
    // All `patternProperties` rules match on different values
    #[test_case(&json!({"bar": 6, "spam": 7}))]
    // All `patternProperties` rules match on the same value
    #[test_case(&json!({"barspam": 7}))]
    // All combined
    #[test_case(&json!({"barspam": 7, "bar": 6, "spam": 7, "foo": "a", "barbaz": 6}))]
    fn schema_1_valid(instance: &Value) {
        let schema = schema_1();
        tests_util::is_valid(&schema, instance);
    }

    // `properties.foo` - should be a string
    #[test_case(&json!({"foo": 3}), &["3 is not of type \"string\""], &["/properties/foo/type"])]
    // `additionalProperties` - extra keyword & not in `properties` / `patternProperties`
    #[test_case(&json!({"faz": 1}), &["Additional properties are not allowed (\'faz\' was unexpected)"], &["/additionalProperties"])]
    #[test_case(&json!({"faz": 1, "haz": 1}), &["Additional properties are not allowed (\'faz\', \'haz\' were unexpected)"], &["/additionalProperties"])]
    // `properties.foo` - should be a string & `patternProperties.^bar` - invalid
    #[test_case(&json!({"foo": 3, "bar": 4}), &["4 is less than the minimum of 5", "3 is not of type \"string\""], &["/patternProperties/^bar/minimum", "/properties/foo/type"])]
    // `properties.barbaz` - valid; `patternProperties.^bar` - invalid
    #[test_case(&json!({"barbaz": 3}), &["3 is less than the minimum of 5"], &["/patternProperties/^bar/minimum"])]
    // `patternProperties.^bar` (should be >=5)
    #[test_case(&json!({"bar": 4}), &["4 is less than the minimum of 5"], &["/patternProperties/^bar/minimum"])]
    // `patternProperties.spam$` (should be <=10)
    #[test_case(&json!({"spam": 11}), &["11 is greater than the maximum of 10"], &["/patternProperties/spam$/maximum"])]
    // `patternProperties` - both values are invalid
    #[test_case(&json!({"bar": 4, "spam": 11}), &["4 is less than the minimum of 5", "11 is greater than the maximum of 10"], &["/patternProperties/^bar/minimum", "/patternProperties/spam$/maximum"])]
    // `patternProperties` - `bar` is valid, `spam` is invalid
    #[test_case(&json!({"bar": 6, "spam": 11}), &["11 is greater than the maximum of 10"], &["/patternProperties/spam$/maximum"])]
    // `patternProperties` - `bar` is invalid, `spam` is valid
    #[test_case(&json!({"bar": 4, "spam": 8}), &["4 is less than the minimum of 5"], &["/patternProperties/^bar/minimum"])]
    // `patternProperties.^bar` - (should be >=5), but valid for `patternProperties.spam$`
    #[test_case(&json!({"barspam": 4}), &["4 is less than the minimum of 5"], &["/patternProperties/^bar/minimum"])]
    // `patternProperties.spam$` - (should be <=10), but valid for `patternProperties.^bar`
    #[test_case(&json!({"barspam": 11}), &["11 is greater than the maximum of 10"], &["/patternProperties/spam$/maximum"])]
    // All combined
    #[test_case(
      &json!({"bar": 4, "spam": 11, "foo": 3, "faz": 1}),
      &[
          "4 is less than the minimum of 5",
          "3 is not of type \"string\"",
          "11 is greater than the maximum of 10",
          "Additional properties are not allowed (\'faz\' was unexpected)"
      ],
      &[
          "/patternProperties/^bar/minimum",
          "/properties/foo/type",
          "/patternProperties/spam$/maximum",
          "/additionalProperties"
      ]
    )]
    fn schema_1_invalid(instance: &Value, expected: &[&str], locations: &[&str]) {
        let schema = schema_1();
        tests_util::is_not_valid(&schema, instance);
        tests_util::expect_errors(&schema, instance, expected);
        tests_util::assert_locations(&schema, instance, locations);
    }

    fn schema_2() -> Value {
        // For `AdditionalPropertiesWithPatternsFalseValidator`
        json!({
            "additionalProperties": false,
            "patternProperties": {
                "^bar": {"type": "integer", "minimum": 5},
                "spam$": {"type": "integer", "maximum": 10},
            }
        })
    }

    // Another type
    #[test_case(&json!([1]))]
    // The right type
    #[test_case(&json!({}))]
    // Match `patternProperties.^bar`
    #[test_case(&json!({"bar": 6}))]
    // Match `patternProperties.spam$`
    #[test_case(&json!({"spam": 7}))]
    // All `patternProperties` rules match on different values
    #[test_case(&json!({"bar": 6, "spam": 7}))]
    // All `patternProperties` rules match on the same value
    #[test_case(&json!({"barspam": 7}))]
    // All combined
    #[test_case(&json!({"barspam": 7, "bar": 6, "spam": 7}))]
    fn schema_2_valid(instance: &Value) {
        let schema = schema_2();
        tests_util::is_valid(&schema, instance);
    }

    // `additionalProperties` - extra keyword & not in `patternProperties`
    #[test_case(&json!({"faz": "a"}), &["Additional properties are not allowed (\'faz\' was unexpected)"], &["/additionalProperties"])]
    // `patternProperties.^bar` (should be >=5)
    #[test_case(&json!({"bar": 4}), &["4 is less than the minimum of 5"], &["/patternProperties/^bar/minimum"])]
    // `patternProperties.spam$` (should be <=10)
    #[test_case(&json!({"spam": 11}), &["11 is greater than the maximum of 10"], &["/patternProperties/spam$/maximum"])]
    // `patternProperties` - both values are invalid
    #[test_case(&json!({"bar": 4, "spam": 11}), &["4 is less than the minimum of 5", "11 is greater than the maximum of 10"], &["/patternProperties/^bar/minimum", "/patternProperties/spam$/maximum"])]
    // `patternProperties` - `bar` is valid, `spam` is invalid
    #[test_case(&json!({"bar": 6, "spam": 11}), &["11 is greater than the maximum of 10"], &["/patternProperties/spam$/maximum"])]
    // `patternProperties` - `bar` is invalid, `spam` is valid
    #[test_case(&json!({"bar": 4, "spam": 8}), &["4 is less than the minimum of 5"], &["/patternProperties/^bar/minimum"])]
    // `patternProperties.^bar` - (should be >=5), but valid for `patternProperties.spam$`
    #[test_case(&json!({"barspam": 4}), &["4 is less than the minimum of 5"], &["/patternProperties/^bar/minimum"])]
    // `patternProperties.spam$` - (should be <=10), but valid for `patternProperties.^bar`
    #[test_case(&json!({"barspam": 11}), &["11 is greater than the maximum of 10"], &["/patternProperties/spam$/maximum"])]
    // All combined
    #[test_case(
      &json!({"bar": 4, "spam": 11, "faz": 1}),
      &[
          "4 is less than the minimum of 5",
          "11 is greater than the maximum of 10",
          "Additional properties are not allowed (\'faz\' was unexpected)"
      ],
      &[
          "/patternProperties/^bar/minimum",
          "/patternProperties/spam$/maximum",
          "/additionalProperties"
      ]
    )]
    fn schema_2_invalid(instance: &Value, expected: &[&str], locations: &[&str]) {
        let schema = schema_2();
        tests_util::is_not_valid(&schema, instance);
        tests_util::expect_errors(&schema, instance, expected);
        tests_util::assert_locations(&schema, instance, locations);
    }

    fn schema_3() -> Value {
        // For `AdditionalPropertiesNotEmptyFalseValidator`
        json!({
            "additionalProperties": false,
            "properties": {
                "foo": {"type": "string"}
            }
        })
    }

    // Another type
    #[test_case(&json!([1]))]
    // The right type
    #[test_case(&json!({}))]
    // Match `properties`
    #[test_case(&json!({"foo": "a"}))]
    fn schema_3_valid(instance: &Value) {
        let schema = schema_3();
        tests_util::is_valid(&schema, instance);
    }

    // `properties` - should be a string
    #[test_case(&json!({"foo": 3}), &["3 is not of type \"string\""], &["/properties/foo/type"])]
    // `additionalProperties` - extra keyword & not in `properties`
    #[test_case(&json!({"faz": "a"}), &["Additional properties are not allowed (\'faz\' was unexpected)"], &["/additionalProperties"])]
    // All combined
    #[test_case(
      &json!(
        {"foo": 3, "faz": "a"}),
        &[
            "3 is not of type \"string\"",
            "Additional properties are not allowed (\'faz\' was unexpected)",
        ],
        &[
            "/properties/foo/type",
            "/additionalProperties",
        ]
    )]
    fn schema_3_invalid(instance: &Value, expected: &[&str], locations: &[&str]) {
        let schema = schema_3();
        tests_util::is_not_valid(&schema, instance);
        tests_util::expect_errors(&schema, instance, expected);
        tests_util::assert_locations(&schema, instance, locations);
    }

    fn schema_4() -> Value {
        // For `AdditionalPropertiesNotEmptyValidator`
        json!({
            "additionalProperties": {"type": "integer"},
            "properties": {
                "foo": {"type": "string"}
            }
        })
    }

    // Another type
    #[test_case(&json!([1]))]
    // The right type
    #[test_case(&json!({}))]
    // Match `properties`
    #[test_case(&json!({"foo": "a"}))]
    // Match `additionalProperties`
    #[test_case(&json!({"bar": 4}))]
    // All combined
    #[test_case(&json!({"foo": "a", "bar": 4}))]
    fn schema_4_valid(instance: &Value) {
        let schema = schema_4();
        tests_util::is_valid(&schema, instance);
    }

    // `properties` - should be a string
    #[test_case(&json!({"foo": 3}), &["3 is not of type \"string\""], &["/properties/foo/type"])]
    // `additionalProperties` - should be an integer
    #[test_case(&json!({"bar": "a"}), &["\"a\" is not of type \"integer\""], &["/additionalProperties/type"])]
    // All combined
    #[test_case(
      &json!(
        {"foo": 3, "bar": "a"}),
        &[
            "\"a\" is not of type \"integer\"",
            "3 is not of type \"string\""
        ],
        &[
            "/additionalProperties/type",
            "/properties/foo/type",
        ]
    )]
    fn schema_4_invalid(instance: &Value, expected: &[&str], locations: &[&str]) {
        let schema = schema_4();
        tests_util::is_not_valid(&schema, instance);
        tests_util::expect_errors(&schema, instance, expected);
        tests_util::assert_locations(&schema, instance, locations);
    }

    fn schema_5() -> Value {
        // For `AdditionalPropertiesWithPatternsNotEmptyValidator`
        json!({
            "additionalProperties": {"type": "integer"},
            "properties": {
                "foo": {"type": "string"},
                "barbaz": {"type": "integer", "multipleOf": 3},
            },
            "patternProperties": {
                "^bar": {"type": "integer", "minimum": 5},
                "spam$": {"type": "integer", "maximum": 10},
            }
        })
    }

    // Another type
    #[test_case(&json!([1]))]
    // The right type
    #[test_case(&json!({}))]
    // Match `properties.foo`
    #[test_case(&json!({"foo": "a"}))]
    // Match `additionalProperties`
    #[test_case(&json!({"faz": 42}))]
    // Match `properties.barbaz` & `patternProperties.^bar`
    #[test_case(&json!({"barbaz": 6}))]
    // Match `patternProperties.^bar`
    #[test_case(&json!({"bar": 6}))]
    // Match `patternProperties.spam$`
    #[test_case(&json!({"spam": 7}))]
    // All `patternProperties` rules match on different values
    #[test_case(&json!({"bar": 6, "spam": 7}))]
    // All `patternProperties` rules match on the same value
    #[test_case(&json!({"barspam": 7}))]
    // All combined
    #[test_case(&json!({"barspam": 7, "bar": 6, "spam": 7, "foo": "a", "barbaz": 6, "faz": 42}))]
    fn schema_5_valid(instance: &Value) {
        let schema = schema_5();
        tests_util::is_valid(&schema, instance);
    }

    // `properties.bar` - should be a string
    #[test_case(&json!({"foo": 3}), &["3 is not of type \"string\""], &["/properties/foo/type"])]
    // `additionalProperties` - extra keyword that doesn't match `additionalProperties`
    #[test_case(&json!({"faz": "a"}), &["\"a\" is not of type \"integer\""], &["/additionalProperties/type"])]
    #[test_case(&json!({"faz": "a", "haz": "a"}), &["\"a\" is not of type \"integer\"", "\"a\" is not of type \"integer\""], &["/additionalProperties/type", "/additionalProperties/type"])]
    // `properties.foo` - should be a string & `patternProperties.^bar` - invalid
    #[test_case(&json!({"foo": 3, "bar": 4}), &["4 is less than the minimum of 5", "3 is not of type \"string\""], &["/patternProperties/^bar/minimum", "/properties/foo/type"])]
    // `properties.barbaz` - valid; `patternProperties.^bar` - invalid
    #[test_case(&json!({"barbaz": 3}), &["3 is less than the minimum of 5"], &["/patternProperties/^bar/minimum"])]
    // `patternProperties.^bar` (should be >=5)
    #[test_case(&json!({"bar": 4}), &["4 is less than the minimum of 5"], &["/patternProperties/^bar/minimum"])]
    // `patternProperties.spam$` (should be <=10)
    #[test_case(&json!({"spam": 11}), &["11 is greater than the maximum of 10"], &["/patternProperties/spam$/maximum"])]
    // `patternProperties` - both values are invalid
    #[test_case(&json!({"bar": 4, "spam": 11}), &["4 is less than the minimum of 5", "11 is greater than the maximum of 10"], &["/patternProperties/^bar/minimum", "/patternProperties/spam$/maximum"])]
    // `patternProperties` - `bar` is valid, `spam` is invalid
    #[test_case(&json!({"bar": 6, "spam": 11}), &["11 is greater than the maximum of 10"], &["/patternProperties/spam$/maximum"])]
    // `patternProperties` - `bar` is invalid, `spam` is valid
    #[test_case(&json!({"bar": 4, "spam": 8}), &["4 is less than the minimum of 5"], &["/patternProperties/^bar/minimum"])]
    // `patternProperties.^bar` - (should be >=5), but valid for `patternProperties.spam$`
    #[test_case(&json!({"barspam": 4}), &["4 is less than the minimum of 5"], &["/patternProperties/^bar/minimum"])]
    // `patternProperties.spam$` - (should be <=10), but valid for `patternProperties.^bar`
    #[test_case(&json!({"barspam": 11}), &["11 is greater than the maximum of 10"], &["/patternProperties/spam$/maximum"])]
    // All combined + valid via `additionalProperties`
    #[test_case(
      &json!({"bar": 4, "spam": 11, "foo": 3, "faz": "a", "fam": 42}),
      &[
          "4 is less than the minimum of 5",
          "\"a\" is not of type \"integer\"",
          "3 is not of type \"string\"",
          "11 is greater than the maximum of 10",
      ],
      &[
          "/patternProperties/^bar/minimum",
          "/additionalProperties/type",
          "/properties/foo/type",
          "/patternProperties/spam$/maximum",
      ]
    )]
    fn schema_5_invalid(instance: &Value, expected: &[&str], locations: &[&str]) {
        let schema = schema_5();
        tests_util::is_not_valid(&schema, instance);
        tests_util::expect_errors(&schema, instance, expected);
        tests_util::assert_locations(&schema, instance, locations);
    }

    fn schema_6() -> Value {
        // For `AdditionalPropertiesWithPatternsValidator`
        json!({
            "additionalProperties": {"type": "integer"},
            "patternProperties": {
                "^bar": {"type": "integer", "minimum": 5},
                "spam$": {"type": "integer", "maximum": 10},
            }
        })
    }

    // Another type
    #[test_case(&json!([1]))]
    // The right type
    #[test_case(&json!({}))]
    // Match `additionalProperties`
    #[test_case(&json!({"faz": 42}))]
    // Match `patternProperties.^bar`
    #[test_case(&json!({"bar": 6}))]
    // Match `patternProperties.spam$`
    #[test_case(&json!({"spam": 7}))]
    // All `patternProperties` rules match on different values
    #[test_case(&json!({"bar": 6, "spam": 7}))]
    // All `patternProperties` rules match on the same value
    #[test_case(&json!({"barspam": 7}))]
    // All combined
    #[test_case(&json!({"barspam": 7, "bar": 6, "spam": 7, "faz": 42}))]
    fn schema_6_valid(instance: &Value) {
        let schema = schema_6();
        tests_util::is_valid(&schema, instance);
    }

    // `additionalProperties` - extra keyword that doesn't match `additionalProperties`
    #[test_case(&json!({"faz": "a"}), &["\"a\" is not of type \"integer\""], &["/additionalProperties/type"])]
    #[test_case(&json!({"faz": "a", "haz": "a"}), &["\"a\" is not of type \"integer\"", "\"a\" is not of type \"integer\""], &["/additionalProperties/type", "/additionalProperties/type"])]
    // `additionalProperties` - should be an integer & `patternProperties.^bar` - invalid
    #[test_case(&json!({"foo": "a", "bar": 4}), &["4 is less than the minimum of 5", "\"a\" is not of type \"integer\""], &["/patternProperties/^bar/minimum", "/additionalProperties/type"])]
    // `patternProperties.^bar` (should be >=5)
    #[test_case(&json!({"bar": 4}), &["4 is less than the minimum of 5"], &["/patternProperties/^bar/minimum"])]
    // `patternProperties.spam$` (should be <=10)
    #[test_case(&json!({"spam": 11}), &["11 is greater than the maximum of 10"], &["/patternProperties/spam$/maximum"])]
    // `patternProperties` - both values are invalid
    #[test_case(&json!({"bar": 4, "spam": 11}), &["4 is less than the minimum of 5", "11 is greater than the maximum of 10"], &["/patternProperties/^bar/minimum", "/patternProperties/spam$/maximum"])]
    // `patternProperties` - `bar` is valid, `spam` is invalid
    #[test_case(&json!({"bar": 6, "spam": 11}), &["11 is greater than the maximum of 10"], &["/patternProperties/spam$/maximum"])]
    // `patternProperties` - `bar` is invalid, `spam` is valid
    #[test_case(&json!({"bar": 4, "spam": 8}), &["4 is less than the minimum of 5"], &["/patternProperties/^bar/minimum"])]
    // `patternProperties.^bar` - (should be >=5), but valid for `patternProperties.spam$`
    #[test_case(&json!({"barspam": 4}), &["4 is less than the minimum of 5"], &["/patternProperties/^bar/minimum"])]
    // `patternProperties.spam$` - (should be <=10), but valid for `patternProperties.^bar`
    #[test_case(&json!({"barspam": 11}), &["11 is greater than the maximum of 10"], &["/patternProperties/spam$/maximum"])]
    // All combined + valid via `additionalProperties`
    #[test_case(
      &json!({"bar": 4, "spam": 11, "faz": "a", "fam": 42}),
      &[
          "4 is less than the minimum of 5",
          "\"a\" is not of type \"integer\"",
          "11 is greater than the maximum of 10",
      ],
      &[
          "/patternProperties/^bar/minimum",
          "/additionalProperties/type",
          "/patternProperties/spam$/maximum",
      ]
    )]
    fn schema_6_invalid(instance: &Value, expected: &[&str], locations: &[&str]) {
        let schema = schema_6();
        tests_util::is_not_valid(&schema, instance);
        tests_util::expect_errors(&schema, instance, expected);
        tests_util::assert_locations(&schema, instance, locations);
    }
}
