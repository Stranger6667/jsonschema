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
    keywords::CompilationResult,
    node::SchemaNode,
    output::{Annotations, BasicOutput, OutputUnit},
    paths::{LazyLocation, Location},
    properties::*,
    types::JsonType,
    validator::{PartialApplication, Validate},
    TracingCallback, TracingContext,
};
use referencing::Uri;
use serde_json::{Map, Value};

macro_rules! is_valid {
    ($node:expr, $value:ident) => {{
        $node.is_valid($value)
    }};
}

macro_rules! is_valid_pattern_schema {
    ($node:expr, $value:ident) => {{
        if $node.is_valid($value) {
            // Matched & valid - check the next pattern
            continue;
        }
        // Invalid - there is no reason to check other patterns
        return false;
    }};
}

macro_rules! is_valid_patterns {
    ($patterns:expr, $property:ident, $value:ident) => {{
        // One property may match multiple patterns, therefore we need to check them all
        let mut has_match = false;
        for (re, node) in $patterns {
            // If there is a match, then the value should match the sub-schema
            if re.is_match($property).unwrap_or(false) {
                has_match = true;
                is_valid_pattern_schema!(node, $value)
            }
        }
        if !has_match {
            // No pattern matched - INVALID property
            return false;
        }
    }};
}

macro_rules! iter_errors {
    ($node:expr, $value:ident, $instance_path:expr, $property_name:expr) => {{
        let location = $instance_path.push($property_name.as_str());
        $node.iter_errors($value, &location)
    }};
}

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
    #[allow(clippy::needless_collect)]
    fn iter_errors<'i>(&self, instance: &'i Value, location: &LazyLocation) -> ErrorIterator<'i> {
        if let Value::Object(item) = instance {
            let errors: Vec<_> = item
                .iter()
                .flat_map(|(name, value)| iter_errors!(self.node, value, location, name))
                .collect();
            Box::new(errors.into_iter())
        } else {
            no_error()
        }
    }

    fn is_valid(&self, instance: &Value) -> bool {
        if let Value::Object(item) = instance {
            item.values().all(|i| self.node.is_valid(i))
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Object(item) = instance {
            for (name, value) in item.iter() {
                self.node.validate(value, &location.push(name))?;
            }
        }
        Ok(())
    }

    fn apply<'a>(&'a self, instance: &Value, location: &LazyLocation) -> PartialApplication<'a> {
        if let Value::Object(item) = instance {
            let mut matched_props = Vec::with_capacity(item.len());
            let mut output = BasicOutput::default();
            for (name, value) in item {
                let path = location.push(name.as_str());
                output += self.node.apply_rooted(value, &path);
                matched_props.push(name.clone());
            }
            let mut result: PartialApplication = output.into();
            result.annotate(Value::from(matched_props).into());
            result
        } else {
            PartialApplication::valid_empty()
        }
    }
    fn matches_type(&self, instance: &Value) -> bool {
        matches!(instance, Value::Object(_))
    }
    fn schema_path(&self) -> &Location {
        self.node.location()
    }
    fn trace(
        &self,
        instance: &Value,
        location: &LazyLocation,
        callback: TracingCallback<'_>,
    ) -> bool {
        if let Value::Object(item) = instance {
            let mut is_valid = true;
            for (name, value) in item.iter() {
                is_valid &= self.node.trace(value, &location.push(name), callback);
            }
            let rv = if !item.is_empty() {
                Some(is_valid)
            } else {
                None
            };
            TracingContext::new(location, self.schema_path(), rv).call(callback);
            is_valid
        } else {
            TracingContext::new(location, self.schema_path(), None).call(callback);
            true
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
    fn is_valid(&self, instance: &Value) -> bool {
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
    fn matches_type(&self, instance: &Value) -> bool {
        matches!(instance, Value::Object(_))
    }
    fn schema_path(&self) -> &Location {
        &self.location
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
    fn iter_errors<'i>(&self, instance: &'i Value, location: &LazyLocation) -> ErrorIterator<'i> {
        if let Value::Object(item) = instance {
            let mut errors = vec![];
            let mut unexpected = vec![];
            for (property, value) in item {
                if let Some((name, node)) = self.properties.get_key_validator(property) {
                    // When a property is in `properties`, then it should be VALID
                    errors.extend(iter_errors!(node, value, location, name));
                } else {
                    // No extra properties are allowed
                    unexpected.push(property.clone());
                }
            }
            if !unexpected.is_empty() {
                errors.push(ValidationError::additional_properties(
                    self.additional_properties_location.clone(),
                    location.into(),
                    instance,
                    unexpected,
                ))
            }
            Box::new(errors.into_iter())
        } else {
            no_error()
        }
    }

    fn is_valid(&self, instance: &Value) -> bool {
        if let Value::Object(props) = instance {
            are_properties_valid(&self.properties, props, |_| false)
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Object(item) = instance {
            for (property, value) in item {
                if let Some((name, node)) = self.properties.get_key_validator(property) {
                    node.validate(value, &location.push(name))?;
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

    fn apply<'a>(&'a self, instance: &Value, location: &LazyLocation) -> PartialApplication<'a> {
        if let Value::Object(item) = instance {
            let mut unexpected = Vec::with_capacity(item.len());
            let mut output = BasicOutput::default();
            for (property, value) in item {
                if let Some((_name, node)) = self.properties.get_key_validator(property) {
                    let path = location.push(property.as_str());
                    output += node.apply_rooted(value, &path);
                } else {
                    unexpected.push(property.clone())
                }
            }
            let mut result: PartialApplication = output.into();
            if !unexpected.is_empty() {
                result.mark_errored(
                    ValidationError::additional_properties(
                        self.additional_properties_location.clone(),
                        location.into(),
                        instance,
                        unexpected,
                    )
                    .into(),
                );
            }
            result
        } else {
            PartialApplication::valid_empty()
        }
    }

    fn matches_type(&self, instance: &Value) -> bool {
        matches!(instance, Value::Object(_))
    }
    fn schema_path(&self) -> &Location {
        &self.additional_properties_location
    }
    fn trace(
        &self,
        instance: &Value,
        location: &LazyLocation,
        callback: TracingCallback<'_>,
    ) -> bool {
        if let Value::Object(item) = instance {
            let mut properties_result: Option<bool> = None;
            let mut has_unexpected_properties = false;

            // Check each property in the instance
            for (property, value) in item {
                let property_path = location.push(property);
                if let Some(node) = self.properties.get_validator(property) {
                    // Validate the property first
                    let schema_is_valid = node.trace(value, &property_path, callback);

                    // Report validation for this property
                    TracingContext::new(&property_path, node.schema_path(), schema_is_valid)
                        .call(callback);

                    // Accumulate validation result
                    properties_result =
                        Some(properties_result.map_or(schema_is_valid, |c| c && schema_is_valid));
                } else {
                    has_unexpected_properties = true;
                }
            }

            // Report `properties` result
            TracingContext::new(location, &self.properties_location, properties_result)
                .call(callback);

            // Report `additionalProperties` result
            let additional_props_valid = !has_unexpected_properties;
            TracingContext::new(
                location,
                &self.additional_properties_location,
                additional_props_valid,
            )
            .call(callback);
            properties_result.unwrap_or(true) && additional_props_valid
        } else {
            TracingContext::new(location, self.schema_path(), None).call(callback);
            true
        }
    }
}

impl<M: PropertiesValidatorsMap> core::fmt::Display
    for AdditionalPropertiesNotEmptyFalseValidator<M>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        "additionalProperties: false".fmt(f)
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
    fn iter_errors<'i>(&self, instance: &'i Value, location: &LazyLocation) -> ErrorIterator<'i> {
        if let Value::Object(map) = instance {
            let mut errors = vec![];
            for (property, value) in map {
                if let Some((name, property_validators)) =
                    self.properties.get_key_validator(property)
                {
                    errors.extend(iter_errors!(property_validators, value, location, name))
                } else {
                    errors.extend(iter_errors!(self.node, value, location, property))
                }
            }
            Box::new(errors.into_iter())
        } else {
            no_error()
        }
    }

    fn is_valid(&self, instance: &Value) -> bool {
        if let Value::Object(props) = instance {
            are_properties_valid(&self.properties, props, |instance| {
                self.node.is_valid(instance)
            })
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Object(props) = instance {
            for (property, instance) in props.iter() {
                if let Some(validator) = self.properties.get_validator(property) {
                    validator.validate(instance, &location.push(property))?;
                } else {
                    self.node.validate(instance, &location.push(property))?;
                }
            }
        }
        Ok(())
    }

    fn apply<'a>(&'a self, instance: &Value, location: &LazyLocation) -> PartialApplication<'a> {
        if let Value::Object(map) = instance {
            let mut matched_propnames = Vec::with_capacity(map.len());
            let mut output = BasicOutput::default();
            for (property, value) in map {
                let path = location.push(property.as_str());
                if let Some((_name, property_validators)) =
                    self.properties.get_key_validator(property)
                {
                    output += property_validators.apply_rooted(value, &path);
                } else {
                    output += self.node.apply_rooted(value, &path);
                    matched_propnames.push(property.clone());
                }
            }
            let mut result: PartialApplication = output.into();
            if !matched_propnames.is_empty() {
                result.annotate(Value::from(matched_propnames).into());
            }
            result
        } else {
            PartialApplication::valid_empty()
        }
    }
    fn matches_type(&self, instance: &Value) -> bool {
        matches!(instance, Value::Object(_))
    }
    fn schema_path(&self) -> &Location {
        self.node.location()
    }
    fn trace(
        &self,
        instance: &Value,
        location: &LazyLocation,
        callback: TracingCallback<'_>,
    ) -> bool {
        if let Value::Object(item) = instance {
            let mut properties_result: Option<bool> = None;
            let mut additional_props_result: Option<bool> = None;

            // Check each property in the instance
            for (property, value) in item {
                let property_path = location.push(property);
                if let Some(node) = self.properties.get_validator(property) {
                    // Validate the property first
                    let schema_is_valid = node.trace(value, &property_path, callback);

                    // Report validation for this property
                    TracingContext::new(&property_path, node.schema_path(), schema_is_valid)
                        .call(callback);

                    // Accumulate validation result for properties
                    properties_result =
                        Some(properties_result.map_or(schema_is_valid, |c| c && schema_is_valid));
                } else {
                    // Validate additional property using the schema node
                    let schema_is_valid = self.node.trace(value, &property_path, callback);

                    // Accumulate validation result for additional properties
                    additional_props_result = Some(
                        additional_props_result.map_or(schema_is_valid, |c| c && schema_is_valid),
                    );
                }
            }

            // Report `properties` result
            TracingContext::new(location, &self.properties_location, properties_result)
                .call(callback);

            // Report `additionalProperties` result
            TracingContext::new(location, self.node.schema_path(), additional_props_result)
                .call(callback);

            // Overall validation is successful if both properties and additional properties are valid
            properties_result.unwrap_or(true) && additional_props_result.unwrap_or(true)
        } else {
            TracingContext::new(location, self.schema_path(), None).call(callback);
            true
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
pub(crate) struct AdditionalPropertiesWithPatternsValidator {
    node: SchemaNode,
    patterns: PatternedValidators,
    /// We need this because `compiler::compile` uses the additionalProperties keyword to compile
    /// this validator. That means that the schema node which contains this validator has
    /// "additionalProperties" as it's path. However, we need to produce annotations which have the
    /// patternProperties keyword as their path so we store the paths here.
    pattern_keyword_path: Location,
    pattern_keyword_absolute_location: Option<Uri<String>>,
}
impl AdditionalPropertiesWithPatternsValidator {
    #[inline]
    pub(crate) fn compile<'a>(
        ctx: &compiler::Context,
        schema: &'a Value,
        patterns: PatternedValidators,
    ) -> CompilationResult<'a> {
        let kctx = ctx.new_at_location("additionalProperties");
        Ok(Box::new(AdditionalPropertiesWithPatternsValidator {
            node: compiler::compile(&kctx, kctx.as_resource_ref(schema))?,
            patterns,
            pattern_keyword_path: ctx.location().join("patternProperties"),
            pattern_keyword_absolute_location: ctx.new_at_location("patternProperties").base_uri(),
        }))
    }
}
impl Validate for AdditionalPropertiesWithPatternsValidator {
    fn iter_errors<'i>(&self, instance: &'i Value, location: &LazyLocation) -> ErrorIterator<'i> {
        if let Value::Object(item) = instance {
            let mut errors = vec![];
            for (property, value) in item {
                let mut has_match = false;
                errors.extend(
                    self.patterns
                        .iter()
                        .filter(|(re, _)| re.is_match(property).unwrap_or(false))
                        .flat_map(|(_, node)| {
                            has_match = true;
                            iter_errors!(node, value, location, property)
                        }),
                );
                if !has_match {
                    errors.extend(iter_errors!(self.node, value, location, property))
                }
            }
            Box::new(errors.into_iter())
        } else {
            no_error()
        }
    }

    fn is_valid(&self, instance: &Value) -> bool {
        if let Value::Object(item) = instance {
            for (property, value) in item {
                let mut has_match = false;
                for (re, node) in &self.patterns {
                    if re.is_match(property).unwrap_or(false) {
                        has_match = true;
                        is_valid_pattern_schema!(node, value)
                    }
                }
                if !has_match && !is_valid!(self.node, value) {
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
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Object(item) = instance {
            for (property, value) in item {
                let mut has_match = false;
                for (re, node) in self.patterns.iter() {
                    if re.is_match(property).unwrap_or(false) {
                        has_match = true;
                        node.validate(value, &location.push(property))?;
                    }
                }
                if !has_match {
                    self.node.validate(value, &location.push(property))?;
                }
            }
        }
        Ok(())
    }

    fn apply<'a>(&'a self, instance: &Value, location: &LazyLocation) -> PartialApplication<'a> {
        if let Value::Object(item) = instance {
            let mut output = BasicOutput::default();
            let mut pattern_matched_propnames = Vec::with_capacity(item.len());
            let mut additional_matched_propnames = Vec::with_capacity(item.len());
            for (property, value) in item {
                let path = location.push(property.as_str());
                let mut has_match = false;
                for (pattern, node) in &self.patterns {
                    if pattern.is_match(property).unwrap_or(false) {
                        has_match = true;
                        pattern_matched_propnames.push(property.clone());
                        output += node.apply_rooted(value, &path)
                    }
                }
                if !has_match {
                    additional_matched_propnames.push(property.clone());
                    output += self.node.apply_rooted(value, &path)
                }
            }
            if !pattern_matched_propnames.is_empty() {
                output += OutputUnit::<Annotations<'_>>::annotations(
                    self.pattern_keyword_path.clone(),
                    location.into(),
                    self.pattern_keyword_absolute_location.clone(),
                    Value::from(pattern_matched_propnames).into(),
                )
                .into();
            }
            let mut result: PartialApplication = output.into();
            if !additional_matched_propnames.is_empty() {
                result.annotate(Value::from(additional_matched_propnames).into())
            }
            result
        } else {
            PartialApplication::valid_empty()
        }
    }
    fn matches_type(&self, instance: &Value) -> bool {
        matches!(instance, Value::Object(_))
    }
    fn schema_path(&self) -> &Location {
        self.node.location()
    }
    fn trace(
        &self,
        instance: &Value,
        location: &LazyLocation,
        callback: TracingCallback<'_>,
    ) -> bool {
        if let Value::Object(item) = instance {
            let mut pattern_props_results: Option<bool> = None;
            let mut additional_props_results: Option<bool> = None;

            // Check each property in the instance
            for (property, value) in item {
                let property_path = location.push(property);
                let mut has_pattern_match = false;

                // Check if property matches any patterns
                for (re, node) in self.patterns.iter() {
                    if re.is_match(property).unwrap_or(false) {
                        has_pattern_match = true;

                        // Validate against pattern schema - this internally reports individual validations
                        let schema_is_valid = node.trace(value, &property_path, callback);

                        // Report validation for this pattern
                        TracingContext::new(&property_path, node.schema_path(), schema_is_valid)
                            .call(callback);

                        // Accumulate results for pattern properties
                        pattern_props_results = Some(
                            pattern_props_results
                                .map_or(schema_is_valid, |prev| prev && schema_is_valid),
                        );
                    }
                }

                // If no pattern matched, use additionalProperties validator
                if !has_pattern_match {
                    // Validate `additionalProperties` - this internally reports individual validations
                    let schema_is_valid = self.node.trace(value, &property_path, callback);

                    additional_props_results = Some(
                        additional_props_results
                            .map_or(schema_is_valid, |prev| prev && schema_is_valid),
                    );
                }
            }

            // Report `patternProperties`
            TracingContext::new(location, &self.pattern_keyword_path, pattern_props_results)
                .call(callback);

            // Report `additionalProperties`
            TracingContext::new(location, self.node.schema_path(), additional_props_results)
                .call(callback);

            pattern_props_results.unwrap_or(true) && additional_props_results.unwrap_or(true)
        } else {
            TracingContext::new(location, self.schema_path(), None).call(callback);
            true
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
pub(crate) struct AdditionalPropertiesWithPatternsFalseValidator {
    patterns: PatternedValidators,
    location: Location,
    pattern_keyword_path: Location,
    pattern_keyword_absolute_location: Option<Uri<String>>,
}
impl AdditionalPropertiesWithPatternsFalseValidator {
    #[inline]
    pub(crate) fn compile<'a>(
        ctx: &compiler::Context,
        patterns: PatternedValidators,
    ) -> CompilationResult<'a> {
        Ok(Box::new(AdditionalPropertiesWithPatternsFalseValidator {
            patterns,
            location: ctx.location().join("additionalProperties"),
            pattern_keyword_path: ctx.location().join("patternProperties"),
            pattern_keyword_absolute_location: ctx.new_at_location("patternProperties").base_uri(),
        }))
    }
}
impl Validate for AdditionalPropertiesWithPatternsFalseValidator {
    fn iter_errors<'i>(&self, instance: &'i Value, location: &LazyLocation) -> ErrorIterator<'i> {
        if let Value::Object(item) = instance {
            let mut errors = vec![];
            let mut unexpected = vec![];
            for (property, value) in item {
                let mut has_match = false;
                errors.extend(
                    self.patterns
                        .iter()
                        .filter(|(re, _)| re.is_match(property).unwrap_or(false))
                        .flat_map(|(_, node)| {
                            has_match = true;
                            iter_errors!(node, value, location, property)
                        }),
                );
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
                ))
            }
            Box::new(errors.into_iter())
        } else {
            no_error()
        }
    }

    fn is_valid(&self, instance: &Value) -> bool {
        if let Value::Object(item) = instance {
            // No properties are allowed, except ones defined in `patternProperties`
            for (property, value) in item {
                is_valid_patterns!(&self.patterns, property, value);
            }
        }
        true
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Object(item) = instance {
            for (property, value) in item {
                let mut has_match = false;
                for (re, node) in self.patterns.iter() {
                    if re.is_match(property).unwrap_or(false) {
                        has_match = true;
                        node.validate(value, &location.push(property))?;
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

    fn apply<'a>(&'a self, instance: &Value, location: &LazyLocation) -> PartialApplication<'a> {
        if let Value::Object(item) = instance {
            let mut output = BasicOutput::default();
            let mut unexpected = Vec::with_capacity(item.len());
            let mut pattern_matched_props = Vec::with_capacity(item.len());
            for (property, value) in item {
                let path = location.push(property.as_str());
                let mut has_match = false;
                for (pattern, node) in &self.patterns {
                    if pattern.is_match(property).unwrap_or(false) {
                        has_match = true;
                        pattern_matched_props.push(property.clone());
                        output += node.apply_rooted(value, &path);
                    }
                }
                if !has_match {
                    unexpected.push(property.clone());
                }
            }
            if !pattern_matched_props.is_empty() {
                output += OutputUnit::<Annotations<'_>>::annotations(
                    self.pattern_keyword_path.clone(),
                    location.into(),
                    self.pattern_keyword_absolute_location.clone(),
                    Value::from(pattern_matched_props).into(),
                )
                .into();
            }
            let mut result: PartialApplication = output.into();
            if !unexpected.is_empty() {
                result.mark_errored(
                    ValidationError::additional_properties(
                        self.location.clone(),
                        location.into(),
                        instance,
                        unexpected,
                    )
                    .into(),
                );
            }
            result
        } else {
            PartialApplication::valid_empty()
        }
    }
    fn matches_type(&self, instance: &Value) -> bool {
        matches!(instance, Value::Object(_))
    }
    fn schema_path(&self) -> &Location {
        &self.location
    }
    fn trace(
        &self,
        instance: &Value,
        location: &LazyLocation,
        callback: TracingCallback<'_>,
    ) -> bool {
        if let Value::Object(item) = instance {
            let mut pattern_props_results: Option<bool> = None;
            let mut has_unexpected_properties = false;

            // Check each property in the instance
            for (property, value) in item {
                let property_path = location.push(property);
                let mut has_pattern_match = false;

                // Check if property matches any patterns
                for (re, node) in self.patterns.iter() {
                    if re.is_match(property).unwrap_or(false) {
                        has_pattern_match = true;

                        // Validate against pattern schema
                        let schema_is_valid = node.trace(value, &property_path, callback);

                        // Report validation for this specific pattern match
                        TracingContext::new(&property_path, node.schema_path(), schema_is_valid)
                            .call(callback);

                        // Accumulate results for pattern properties
                        pattern_props_results = Some(
                            pattern_props_results
                                .map_or(schema_is_valid, |prev| prev && schema_is_valid),
                        );
                    }
                }

                // If no pattern matched, the property is unexpected (invalid since additionalProperties is false)
                if !has_pattern_match {
                    has_unexpected_properties = true;
                }
            }

            // Report `patternProperties`
            TracingContext::new(location, &self.pattern_keyword_path, pattern_props_results)
                .call(callback);

            // Report `additionalProperties`
            let additional_props_valid = !has_unexpected_properties;
            TracingContext::new(location, &self.location, Some(additional_props_valid))
                .call(callback);

            pattern_props_results.unwrap_or(true) && additional_props_valid
        } else {
            // For non-objects, this validator doesn't apply
            TracingContext::new(location, self.schema_path(), None).call(callback);
            true
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
pub(crate) struct AdditionalPropertiesWithPatternsNotEmptyValidator<M: PropertiesValidatorsMap> {
    node: SchemaNode,
    properties: M,
    patterns: PatternedValidators,
    pattern_properties_location: Location,
    properties_location: Location,
}
impl AdditionalPropertiesWithPatternsNotEmptyValidator<SmallValidatorsMap> {
    #[inline]
    pub(crate) fn compile<'a>(
        map: &'a Map<String, Value>,
        ctx: &compiler::Context,
        schema: &'a Value,
        patterns: PatternedValidators,
    ) -> CompilationResult<'a> {
        let kctx = ctx.new_at_location("additionalProperties");
        Ok(Box::new(
            AdditionalPropertiesWithPatternsNotEmptyValidator {
                node: compiler::compile(&kctx, kctx.as_resource_ref(schema))?,
                properties: compile_small_map(ctx, map)?,
                patterns,
                pattern_properties_location: ctx.location().join("patternProperties"),
                properties_location: ctx.location().join("properties"),
            },
        ))
    }
}
impl AdditionalPropertiesWithPatternsNotEmptyValidator<BigValidatorsMap> {
    #[inline]
    pub(crate) fn compile<'a>(
        map: &'a Map<String, Value>,
        ctx: &compiler::Context,
        schema: &'a Value,
        patterns: PatternedValidators,
    ) -> CompilationResult<'a> {
        let kctx = ctx.new_at_location("additionalProperties");
        Ok(Box::new(
            AdditionalPropertiesWithPatternsNotEmptyValidator {
                node: compiler::compile(&kctx, kctx.as_resource_ref(schema))?,
                properties: compile_big_map(ctx, map)?,
                patterns,
                pattern_properties_location: ctx.location().join("patternProperties"),
                properties_location: ctx.location().join("properties"),
            },
        ))
    }
}
impl<M: PropertiesValidatorsMap> Validate for AdditionalPropertiesWithPatternsNotEmptyValidator<M> {
    fn iter_errors<'i>(&self, instance: &'i Value, location: &LazyLocation) -> ErrorIterator<'i> {
        if let Value::Object(item) = instance {
            let mut errors = vec![];
            for (property, value) in item {
                if let Some((name, node)) = self.properties.get_key_validator(property) {
                    errors.extend(iter_errors!(node, value, location, name));
                    errors.extend(
                        self.patterns
                            .iter()
                            .filter(|(re, _)| re.is_match(property).unwrap_or(false))
                            .flat_map(|(_, node)| iter_errors!(node, value, location, name)),
                    );
                } else {
                    let mut has_match = false;
                    errors.extend(
                        self.patterns
                            .iter()
                            .filter(|(re, _)| re.is_match(property).unwrap_or(false))
                            .flat_map(|(_, node)| {
                                has_match = true;
                                iter_errors!(node, value, location, property)
                            }),
                    );
                    if !has_match {
                        errors.extend(iter_errors!(self.node, value, location, property))
                    }
                }
            }
            Box::new(errors.into_iter())
        } else {
            no_error()
        }
    }

    fn is_valid(&self, instance: &Value) -> bool {
        if let Value::Object(item) = instance {
            for (property, value) in item {
                if let Some(node) = self.properties.get_validator(property) {
                    if is_valid!(node, value) {
                        // Valid for `properties`, check `patternProperties`
                        for (re, node) in &self.patterns {
                            // If there is a match, then the value should match the sub-schema
                            if re.is_match(property).unwrap_or(false) {
                                is_valid_pattern_schema!(node, value)
                            }
                        }
                    } else {
                        // INVALID, no reason to check the next one
                        return false;
                    }
                } else {
                    let mut has_match = false;
                    for (re, node) in &self.patterns {
                        // If there is a match, then the value should match the sub-schema
                        if re.is_match(property).unwrap_or(false) {
                            has_match = true;
                            is_valid_pattern_schema!(node, value)
                        }
                    }
                    if !has_match && !is_valid!(self.node, value) {
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
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Object(item) = instance {
            for (property, value) in item {
                if let Some((name, node)) = self.properties.get_key_validator(property) {
                    node.validate(value, &location.push(name))?;
                    for (re, node) in self.patterns.iter() {
                        if re.is_match(property).unwrap_or(false) {
                            node.validate(value, &location.push(name))?;
                        }
                    }
                } else {
                    let mut has_match = false;
                    for (re, node) in self.patterns.iter() {
                        if re.is_match(property).unwrap_or(false) {
                            has_match = true;
                            node.validate(value, &location.push(property))?;
                        }
                    }

                    if !has_match {
                        self.node.validate(value, &location.push(property))?;
                    }
                }
            }
        }
        Ok(())
    }

    fn apply<'a>(&'a self, instance: &Value, location: &LazyLocation) -> PartialApplication<'a> {
        if let Value::Object(item) = instance {
            let mut output = BasicOutput::default();
            let mut additional_matches = Vec::with_capacity(item.len());
            for (property, value) in item {
                let path = location.push(property.as_str());
                if let Some((_name, node)) = self.properties.get_key_validator(property) {
                    output += node.apply_rooted(value, &path);
                    for (pattern, node) in &self.patterns {
                        if pattern.is_match(property).unwrap_or(false) {
                            output += node.apply_rooted(value, &path);
                        }
                    }
                } else {
                    let mut has_match = false;
                    for (pattern, node) in &self.patterns {
                        if pattern.is_match(property).unwrap_or(false) {
                            has_match = true;
                            output += node.apply_rooted(value, &path);
                        }
                    }
                    if !has_match {
                        additional_matches.push(property.clone());
                        output += self.node.apply_rooted(value, &path);
                    }
                }
            }
            let mut result: PartialApplication = output.into();
            result.annotate(Value::from(additional_matches).into());
            result
        } else {
            PartialApplication::valid_empty()
        }
    }
    fn matches_type(&self, instance: &Value) -> bool {
        matches!(instance, Value::Object(_))
    }
    fn schema_path(&self) -> &Location {
        self.node.location()
    }
    fn trace(
        &self,
        instance: &Value,
        location: &LazyLocation,
        callback: TracingCallback<'_>,
    ) -> bool {
        if let Value::Object(item) = instance {
            let mut properties_result: Option<bool> = None;
            let mut pattern_props_result: Option<bool> = None;
            let mut additional_props_result: Option<bool> = None;

            // Check each property in the instance
            for (property, value) in item {
                let property_path = location.push(property);

                // First check if it's a defined property
                if let Some(node) = self.properties.get_validator(property) {
                    // Validate property against its schema
                    let prop_valid = node.trace(value, &property_path, callback);
                    TracingContext::new(&property_path, node.schema_path(), prop_valid)
                        .call(callback);

                    // Accumulate properties validation result
                    properties_result =
                        Some(properties_result.map_or(prop_valid, |prev| prev && prop_valid));

                    // Properties can also match pattern properties
                    for (re, pattern_node) in self.patterns.iter() {
                        if re.is_match(property).unwrap_or(false) {
                            // Validate against pattern schema
                            let pattern_valid = pattern_node.trace(value, &property_path, callback);
                            TracingContext::new(
                                &property_path,
                                pattern_node.schema_path(),
                                pattern_valid,
                            )
                            .call(callback);

                            // Accumulate pattern properties result
                            pattern_props_result = Some(
                                pattern_props_result
                                    .map_or(pattern_valid, |prev| prev && pattern_valid),
                            );
                        }
                    }
                } else {
                    // Not in properties, check patterns
                    let mut pattern_matched = false;
                    for (re, node) in self.patterns.iter() {
                        if re.is_match(property).unwrap_or(false) {
                            pattern_matched = true;

                            // Validate against pattern schema
                            let pattern_valid = node.trace(value, &property_path, callback);
                            TracingContext::new(&property_path, node.schema_path(), pattern_valid)
                                .call(callback);

                            // Accumulate pattern properties result
                            pattern_props_result = Some(
                                pattern_props_result
                                    .map_or(pattern_valid, |prev| prev && pattern_valid),
                            );
                        }
                    }

                    // If no pattern matched, it's an additional property
                    if !pattern_matched {
                        // Validate against additionalProperties schema
                        let additional_valid = self.node.trace(value, &property_path, callback);

                        // Accumulate additional properties result
                        additional_props_result = Some(
                            additional_props_result
                                .map_or(additional_valid, |prev| prev && additional_valid),
                        );
                    }
                }
            }

            // Report `properties`
            TracingContext::new(location, &self.properties_location, properties_result)
                .call(callback);
            // Report `patternProperties`
            TracingContext::new(
                location,
                &self.pattern_properties_location,
                pattern_props_result,
            )
            .call(callback);
            // Report `additionalProperties`
            TracingContext::new(location, self.node.schema_path(), additional_props_result)
                .call(callback);

            // Overall validation success requires all components to be valid
            properties_result.unwrap_or(true)
                && pattern_props_result.unwrap_or(true)
                && additional_props_result.unwrap_or(true)
        } else {
            // For non-objects, this validator doesn't apply
            TracingContext::new(location, self.schema_path(), None).call(callback);
            true
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
pub(crate) struct AdditionalPropertiesWithPatternsNotEmptyFalseValidator<M: PropertiesValidatorsMap>
{
    properties: M,
    patterns: PatternedValidators,
    location: Location,
    properties_location: Location,
    pattern_properties_location: Location,
}
impl AdditionalPropertiesWithPatternsNotEmptyFalseValidator<SmallValidatorsMap> {
    #[inline]
    pub(crate) fn compile<'a>(
        map: &'a Map<String, Value>,
        ctx: &compiler::Context,
        patterns: PatternedValidators,
    ) -> CompilationResult<'a> {
        Ok(Box::new(
            AdditionalPropertiesWithPatternsNotEmptyFalseValidator::<SmallValidatorsMap> {
                properties: compile_small_map(ctx, map)?,
                patterns,
                location: ctx.location().join("additionalProperties"),
                properties_location: ctx.location().join("properties"),
                pattern_properties_location: ctx.location().join("patternProperties"),
            },
        ))
    }
}
impl AdditionalPropertiesWithPatternsNotEmptyFalseValidator<BigValidatorsMap> {
    #[inline]
    pub(crate) fn compile<'a>(
        map: &'a Map<String, Value>,
        ctx: &compiler::Context,
        patterns: PatternedValidators,
    ) -> CompilationResult<'a> {
        Ok(Box::new(
            AdditionalPropertiesWithPatternsNotEmptyFalseValidator {
                properties: compile_big_map(ctx, map)?,
                patterns,
                location: ctx.location().join("additionalProperties"),
                properties_location: ctx.location().join("properties"),
                pattern_properties_location: ctx.location().join("patternProperties"),
            },
        ))
    }
}

impl<M: PropertiesValidatorsMap> Validate
    for AdditionalPropertiesWithPatternsNotEmptyFalseValidator<M>
{
    fn iter_errors<'i>(&self, instance: &'i Value, location: &LazyLocation) -> ErrorIterator<'i> {
        if let Value::Object(item) = instance {
            let mut errors = vec![];
            let mut unexpected = vec![];
            // No properties are allowed, except ones defined in `properties` or `patternProperties`
            for (property, value) in item {
                if let Some((name, node)) = self.properties.get_key_validator(property) {
                    errors.extend(iter_errors!(node, value, location, name));
                    errors.extend(
                        self.patterns
                            .iter()
                            .filter(|(re, _)| re.is_match(property).unwrap_or(false))
                            .flat_map(|(_, node)| iter_errors!(node, value, location, name)),
                    );
                } else {
                    let mut has_match = false;
                    errors.extend(
                        self.patterns
                            .iter()
                            .filter(|(re, _)| re.is_match(property).unwrap_or(false))
                            .flat_map(|(_, node)| {
                                has_match = true;
                                iter_errors!(node, value, location, property)
                            }),
                    );
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
                ))
            }
            Box::new(errors.into_iter())
        } else {
            no_error()
        }
    }

    fn is_valid(&self, instance: &Value) -> bool {
        if let Value::Object(item) = instance {
            // No properties are allowed, except ones defined in `properties` or `patternProperties`
            for (property, value) in item {
                if let Some(node) = self.properties.get_validator(property) {
                    if is_valid!(node, value) {
                        // Valid for `properties`, check `patternProperties`
                        for (re, node) in &self.patterns {
                            // If there is a match, then the value should match the sub-schema
                            if re.is_match(property).unwrap_or(false) {
                                is_valid_pattern_schema!(node, value)
                            }
                        }
                    } else {
                        // INVALID, no reason to check the next one
                        return false;
                    }
                } else {
                    is_valid_patterns!(&self.patterns, property, value);
                }
            }
        }
        true
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Object(item) = instance {
            // No properties are allowed, except ones defined in `properties` or `patternProperties`
            for (property, value) in item {
                if let Some((name, node)) = self.properties.get_key_validator(property) {
                    node.validate(value, &location.push(name))?;
                    for (re, node) in self.patterns.iter() {
                        if re.is_match(property).unwrap_or(false) {
                            node.validate(value, &location.push(name))?;
                        }
                    }
                } else {
                    let mut has_match = false;
                    for (re, node) in self.patterns.iter() {
                        if re.is_match(property).unwrap_or(false) {
                            has_match = true;
                            node.validate(value, &location.push(property))?;
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

    fn apply<'a>(&'a self, instance: &Value, location: &LazyLocation) -> PartialApplication<'a> {
        if let Value::Object(item) = instance {
            let mut output = BasicOutput::default();
            let mut unexpected = vec![];
            // No properties are allowed, except ones defined in `properties` or `patternProperties`
            for (property, value) in item {
                let path = location.push(property.as_str());
                if let Some((_name, node)) = self.properties.get_key_validator(property) {
                    output += node.apply_rooted(value, &path);
                    for (pattern, node) in &self.patterns {
                        if pattern.is_match(property).unwrap_or(false) {
                            output += node.apply_rooted(value, &path);
                        }
                    }
                } else {
                    let mut has_match = false;
                    for (pattern, node) in &self.patterns {
                        if pattern.is_match(property).unwrap_or(false) {
                            has_match = true;
                            output += node.apply_rooted(value, &path);
                        }
                    }
                    if !has_match {
                        unexpected.push(property.clone());
                    }
                }
            }
            let mut result: PartialApplication = output.into();
            if !unexpected.is_empty() {
                result.mark_errored(
                    ValidationError::additional_properties(
                        self.location.clone(),
                        location.into(),
                        instance,
                        unexpected,
                    )
                    .into(),
                )
            }
            result
        } else {
            PartialApplication::valid_empty()
        }
    }
    fn matches_type(&self, instance: &Value) -> bool {
        matches!(instance, Value::Object(_))
    }
    fn schema_path(&self) -> &Location {
        &self.location
    }
    fn trace(
        &self,
        instance: &Value,
        location: &LazyLocation,
        callback: TracingCallback<'_>,
    ) -> bool {
        if let Value::Object(item) = instance {
            let mut properties_result: Option<bool> = None;
            let mut pattern_props_result: Option<bool> = None;
            let mut has_unexpected_properties = false;

            // Check each property in the instance
            for (property, value) in item {
                let property_path = location.push(property);
                let mut is_known_or_pattern = false;

                // First check if it's a defined property
                if let Some(node) = self.properties.get_validator(property) {
                    is_known_or_pattern = true;

                    // Validate property against its schema
                    let prop_valid = node.trace(value, &property_path, callback);
                    TracingContext::new(&property_path, node.schema_path(), prop_valid)
                        .call(callback);

                    // Accumulate properties validation result
                    properties_result =
                        Some(properties_result.map_or(prop_valid, |prev| prev && prop_valid));

                    // Properties can also match pattern properties
                    for (re, pattern_node) in self.patterns.iter() {
                        if re.is_match(property).unwrap_or(false) {
                            // Validate against pattern schema
                            let pattern_valid = pattern_node.trace(value, &property_path, callback);
                            TracingContext::new(
                                &property_path,
                                pattern_node.schema_path(),
                                pattern_valid,
                            )
                            .call(callback);

                            // Accumulate pattern properties result
                            pattern_props_result = Some(
                                pattern_props_result
                                    .map_or(pattern_valid, |prev| prev && pattern_valid),
                            );
                        }
                    }
                } else {
                    // Not in properties, check patterns
                    for (re, node) in self.patterns.iter() {
                        if re.is_match(property).unwrap_or(false) {
                            is_known_or_pattern = true;

                            // Validate against pattern schema
                            let pattern_valid = node.trace(value, &property_path, callback);
                            TracingContext::new(&property_path, node.schema_path(), pattern_valid)
                                .call(callback);

                            // Accumulate pattern properties result
                            pattern_props_result = Some(
                                pattern_props_result
                                    .map_or(pattern_valid, |prev| prev && pattern_valid),
                            );
                        }
                    }
                }

                // If property is neither in properties nor matches any pattern, it's unexpected
                if !is_known_or_pattern {
                    has_unexpected_properties = true;
                }
            }

            // Report `properties`
            TracingContext::new(location, &self.properties_location, properties_result)
                .call(callback);

            // Report `patternProperties`
            TracingContext::new(
                location,
                &self.pattern_properties_location,
                pattern_props_result,
            )
            .call(callback);

            // Report `additionalProperties` - always false if unexpected properties found
            let additional_props_valid = !has_unexpected_properties;
            TracingContext::new(location, &self.location, Some(additional_props_valid))
                .call(callback);

            // Overall validation succeeds only if:
            // 1. All properties match their schemas
            // 2. All pattern properties match their schemas
            // 3. No additional properties exist
            properties_result.unwrap_or(true)
                && pattern_props_result.unwrap_or(true)
                && additional_props_valid
        } else {
            // For non-objects, this validator doesn't apply
            TracingContext::new(location, self.schema_path(), None).call(callback);
            true
        }
    }
}

impl<M: PropertiesValidatorsMap> core::fmt::Display
    for AdditionalPropertiesWithPatternsNotEmptyFalseValidator<M>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        "additionalProperties: false".fmt(f)
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
            let compiled_patterns = match compile_patterns(ctx, obj) {
                Ok(patterns) => patterns,
                Err(error) => return Some(Err(error)),
            };
            match schema {
                Value::Bool(true) => None, // "additionalProperties" are "true" by default
                Value::Bool(false) => {
                    if let Some(properties) = properties {
                        compile_dynamic_prop_map_validator!(
                            AdditionalPropertiesWithPatternsNotEmptyFalseValidator,
                            properties,
                            ctx,
                            compiled_patterns,
                        )
                    } else {
                        Some(AdditionalPropertiesWithPatternsFalseValidator::compile(
                            ctx,
                            compiled_patterns,
                        ))
                    }
                }
                _ => {
                    if let Some(properties) = properties {
                        compile_dynamic_prop_map_validator!(
                            AdditionalPropertiesWithPatternsNotEmptyValidator,
                            properties,
                            ctx,
                            schema,
                            compiled_patterns,
                        )
                    } else {
                        Some(AdditionalPropertiesWithPatternsValidator::compile(
                            ctx,
                            schema,
                            compiled_patterns,
                        ))
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
        tests_util::is_valid(&schema, instance)
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
        tests_util::assert_locations(&schema, instance, locations)
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
        tests_util::is_valid(&schema, instance)
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
        tests_util::assert_locations(&schema, instance, locations)
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
        tests_util::is_valid(&schema, instance)
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
        tests_util::assert_locations(&schema, instance, locations)
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
        tests_util::is_valid(&schema, instance)
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
        tests_util::assert_locations(&schema, instance, locations)
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
        tests_util::is_valid(&schema, instance)
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
        tests_util::assert_locations(&schema, instance, locations)
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
        tests_util::is_valid(&schema, instance)
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
        tests_util::assert_locations(&schema, instance, locations)
    }
}
