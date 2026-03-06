use crate::{
    compiler,
    error::{no_error, ErrorIterator, ValidationError},
    keywords::CompilationResult,
    paths::{LazyLocation, Location, RefTracker},
    types::JsonType,
    validator::{Validate, ValidationContext},
};
use serde_json::{Map, Value};

pub(crate) struct RequiredValidator {
    required: Vec<String>,
    location: Location,
}

impl RequiredValidator {
    #[inline]
    pub(crate) fn compile(items: &[Value], location: Location) -> CompilationResult<'_> {
        let mut required = Vec::with_capacity(items.len());
        for item in items {
            match item {
                Value::String(string) => required.push(string.clone()),
                _ => {
                    return Err(ValidationError::single_type_error(
                        location.clone(),
                        location,
                        Location::new(),
                        item,
                        JsonType::String,
                    ))
                }
            }
        }
        Ok(Box::new(RequiredValidator { required, location }))
    }
}

impl Validate for RequiredValidator {
    fn is_valid(&self, instance: &Value, _ctx: &mut ValidationContext) -> bool {
        if let Value::Object(item) = instance {
            if item.len() < self.required.len() {
                return false;
            }
            self.required
                .iter()
                .all(|property_name| item.contains_key(property_name))
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
        if let Value::Object(item) = instance {
            for property_name in &self.required {
                if !item.contains_key(property_name) {
                    return Err(ValidationError::required(
                        self.location.clone(),
                        crate::paths::capture_evaluation_path(tracker, &self.location),
                        location.into(),
                        instance,
                        Value::String(property_name.clone()),
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
        if let Value::Object(item) = instance {
            let mut errors = vec![];
            let eval_path = crate::paths::capture_evaluation_path(tracker, &self.location);
            for property_name in &self.required {
                if !item.contains_key(property_name) {
                    errors.push(ValidationError::required(
                        self.location.clone(),
                        eval_path.clone(),
                        location.into(),
                        instance,
                        Value::String(property_name.clone()),
                    ));
                }
            }
            if !errors.is_empty() {
                return ErrorIterator::from_iterator(errors.into_iter());
            }
        }
        no_error()
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
        instance_path: &LazyLocation,
        callback: crate::tracing::TracingCallback<'_>,
        _ctx: &mut ValidationContext,
    ) -> bool {
        if let Value::Object(item) = instance {
            let mut is_valid = true;
            for (idx, property_name) in self.required.iter().enumerate() {
                let present = item.contains_key(property_name);
                if !present {
                    is_valid = false;
                }
                // Trace at index-based path: /required/0, /required/1, etc.
                let item_path = self.location.join(idx);
                crate::tracing::TracingContext::new(instance_path, &item_path, present)
                    .call(callback);
            }
            crate::tracing::TracingContext::new(instance_path, &self.location, is_valid)
                .call(callback);
            is_valid
        } else {
            crate::tracing::TracingContext::new(instance_path, &self.location, None).call(callback);
            true
        }
    }
}

pub(crate) struct SingleItemRequiredValidator {
    value: String,
    location: Location,
}

impl SingleItemRequiredValidator {
    #[inline]
    pub(crate) fn compile(value: &str, location: Location) -> CompilationResult<'_> {
        Ok(Box::new(SingleItemRequiredValidator {
            value: value.to_string(),
            location,
        }))
    }
}

impl Validate for SingleItemRequiredValidator {
    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if !self.is_valid(instance, ctx) {
            return Err(ValidationError::required(
                self.location.clone(),
                crate::paths::capture_evaluation_path(tracker, &self.location),
                location.into(),
                instance,
                Value::String(self.value.clone()),
            ));
        }
        Ok(())
    }

    fn is_valid(&self, instance: &Value, _ctx: &mut ValidationContext) -> bool {
        if let Value::Object(item) = instance {
            if item.is_empty() {
                return false;
            }
            item.contains_key(&self.value)
        } else {
            true
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
        instance_path: &LazyLocation,
        callback: crate::tracing::TracingCallback<'_>,
        _ctx: &mut ValidationContext,
    ) -> bool {
        if let Value::Object(item) = instance {
            let present = item.contains_key(&self.value);
            // Trace at index 0: /required/0 (or /dependencies/email/0)
            let item_path = self.location.join(0usize);
            crate::tracing::TracingContext::new(instance_path, &item_path, present).call(callback);
            // Trace container
            crate::tracing::TracingContext::new(instance_path, &self.location, present)
                .call(callback);
            present
        } else {
            crate::tracing::TracingContext::new(instance_path, &self.location, None).call(callback);
            true
        }
    }
}

/// Specialized validator for exactly 2 required properties.
/// Uses fixed-size array and unrolled checks to avoid Vec/iterator overhead.
pub(crate) struct Required2Validator {
    first: String,
    second: String,
    location: Location,
}

impl Required2Validator {
    #[inline]
    pub(crate) fn compile(
        first: String,
        second: String,
        location: Location,
    ) -> CompilationResult<'static> {
        Ok(Box::new(Required2Validator {
            first,
            second,
            location,
        }))
    }
}

impl Validate for Required2Validator {
    #[inline]
    fn is_valid(&self, instance: &Value, _ctx: &mut ValidationContext) -> bool {
        if let Value::Object(item) = instance {
            item.len() >= 2 && item.contains_key(&self.first) && item.contains_key(&self.second)
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
        if let Value::Object(item) = instance {
            if !item.contains_key(&self.first) {
                return Err(ValidationError::required(
                    self.location.clone(),
                    crate::paths::capture_evaluation_path(tracker, &self.location),
                    location.into(),
                    instance,
                    Value::String(self.first.clone()),
                ));
            }
            if !item.contains_key(&self.second) {
                return Err(ValidationError::required(
                    self.location.clone(),
                    crate::paths::capture_evaluation_path(tracker, &self.location),
                    location.into(),
                    instance,
                    Value::String(self.second.clone()),
                ));
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
        if let Value::Object(item) = instance {
            let eval_path = crate::paths::capture_evaluation_path(tracker, &self.location);
            let mut errors = Vec::new();
            if !item.contains_key(&self.first) {
                errors.push(ValidationError::required(
                    self.location.clone(),
                    eval_path.clone(),
                    location.into(),
                    instance,
                    Value::String(self.first.clone()),
                ));
            }
            if !item.contains_key(&self.second) {
                errors.push(ValidationError::required(
                    self.location.clone(),
                    eval_path,
                    location.into(),
                    instance,
                    Value::String(self.second.clone()),
                ));
            }
            if !errors.is_empty() {
                return ErrorIterator::from_iterator(errors.into_iter());
            }
        }
        no_error()
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
        instance_path: &LazyLocation,
        callback: crate::tracing::TracingCallback<'_>,
        _ctx: &mut ValidationContext,
    ) -> bool {
        if let Value::Object(item) = instance {
            let first_present = item.contains_key(&self.first);
            let second_present = item.contains_key(&self.second);
            crate::tracing::TracingContext::new(
                instance_path,
                &self.location.join(0usize),
                first_present,
            )
            .call(callback);
            crate::tracing::TracingContext::new(
                instance_path,
                &self.location.join(1usize),
                second_present,
            )
            .call(callback);
            let is_valid = first_present && second_present;
            crate::tracing::TracingContext::new(instance_path, &self.location, is_valid)
                .call(callback);
            is_valid
        } else {
            crate::tracing::TracingContext::new(instance_path, &self.location, None).call(callback);
            true
        }
    }
}

/// Specialized validator for exactly 3 required properties.
/// Uses fixed-size fields and unrolled checks to avoid Vec/iterator overhead.
pub(crate) struct Required3Validator {
    first: String,
    second: String,
    third: String,
    location: Location,
}

impl Required3Validator {
    #[inline]
    pub(crate) fn compile(
        first: String,
        second: String,
        third: String,
        location: Location,
    ) -> CompilationResult<'static> {
        Ok(Box::new(Required3Validator {
            first,
            second,
            third,
            location,
        }))
    }
}

impl Validate for Required3Validator {
    #[inline]
    fn is_valid(&self, instance: &Value, _ctx: &mut ValidationContext) -> bool {
        if let Value::Object(item) = instance {
            item.len() >= 3
                && item.contains_key(&self.first)
                && item.contains_key(&self.second)
                && item.contains_key(&self.third)
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
        if let Value::Object(item) = instance {
            if !item.contains_key(&self.first) {
                return Err(ValidationError::required(
                    self.location.clone(),
                    crate::paths::capture_evaluation_path(tracker, &self.location),
                    location.into(),
                    instance,
                    Value::String(self.first.clone()),
                ));
            }
            if !item.contains_key(&self.second) {
                return Err(ValidationError::required(
                    self.location.clone(),
                    crate::paths::capture_evaluation_path(tracker, &self.location),
                    location.into(),
                    instance,
                    Value::String(self.second.clone()),
                ));
            }
            if !item.contains_key(&self.third) {
                return Err(ValidationError::required(
                    self.location.clone(),
                    crate::paths::capture_evaluation_path(tracker, &self.location),
                    location.into(),
                    instance,
                    Value::String(self.third.clone()),
                ));
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
        if let Value::Object(item) = instance {
            let eval_path = crate::paths::capture_evaluation_path(tracker, &self.location);
            let mut errors = Vec::new();
            if !item.contains_key(&self.first) {
                errors.push(ValidationError::required(
                    self.location.clone(),
                    eval_path.clone(),
                    location.into(),
                    instance,
                    Value::String(self.first.clone()),
                ));
            }
            if !item.contains_key(&self.second) {
                errors.push(ValidationError::required(
                    self.location.clone(),
                    eval_path.clone(),
                    location.into(),
                    instance,
                    Value::String(self.second.clone()),
                ));
            }
            if !item.contains_key(&self.third) {
                errors.push(ValidationError::required(
                    self.location.clone(),
                    eval_path,
                    location.into(),
                    instance,
                    Value::String(self.third.clone()),
                ));
            }
            if !errors.is_empty() {
                return ErrorIterator::from_iterator(errors.into_iter());
            }
        }
        no_error()
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
        instance_path: &LazyLocation,
        callback: crate::tracing::TracingCallback<'_>,
        _ctx: &mut ValidationContext,
    ) -> bool {
        if let Value::Object(item) = instance {
            let p0 = item.contains_key(&self.first);
            let p1 = item.contains_key(&self.second);
            let p2 = item.contains_key(&self.third);
            crate::tracing::TracingContext::new(instance_path, &self.location.join(0usize), p0)
                .call(callback);
            crate::tracing::TracingContext::new(instance_path, &self.location.join(1usize), p1)
                .call(callback);
            crate::tracing::TracingContext::new(instance_path, &self.location.join(2usize), p2)
                .call(callback);
            let is_valid = p0 && p1 && p2;
            crate::tracing::TracingContext::new(instance_path, &self.location, is_valid)
                .call(callback);
            is_valid
        } else {
            crate::tracing::TracingContext::new(instance_path, &self.location, None).call(callback);
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
    // Check if fused validators handle this case
    if let Value::Array(items) = schema {
        let has_properties = parent.contains_key("properties");
        let has_pattern_properties = parent.contains_key("patternProperties");
        let additional_props_false =
            matches!(parent.get("additionalProperties"), Some(Value::Bool(false)));

        // Case 1: properties + additionalProperties: false + required: [1 item], no patternProperties
        // Handled by AdditionalPropertiesNotEmptyFalseWithRequired1Validator
        if items.len() == 1 && additional_props_false && has_properties && !has_pattern_properties {
            return None;
        }

        // Case 2: properties + required: [2 items], no additionalProperties: false, no patternProperties
        // Handled by SmallPropertiesWithRequired2Validator
        if items.len() == 2 && has_properties && !additional_props_false && !has_pattern_properties
        {
            return None;
        }
    }
    let location = ctx.location().join("required");
    compile_with_path(schema, location)
}

#[inline]
pub(crate) fn compile_with_path(
    schema: &Value,
    location: Location,
) -> Option<CompilationResult<'_>> {
    // IMPORTANT: If this function will ever return `None`, adjust `dependencies.rs` accordingly
    match schema {
        Value::Array(items) => match items.len() {
            1 => {
                let item = &items[0];
                if let Value::String(item) = item {
                    Some(SingleItemRequiredValidator::compile(item, location))
                } else {
                    Some(Err(ValidationError::single_type_error(
                        location.clone(),
                        location,
                        Location::new(),
                        item,
                        JsonType::String,
                    )))
                }
            }
            2 => {
                let (first, second) = (&items[0], &items[1]);
                match (first, second) {
                    (Value::String(first), Value::String(second)) => Some(
                        Required2Validator::compile(first.clone(), second.clone(), location),
                    ),
                    (Value::String(_), other) | (other, _) => {
                        Some(Err(ValidationError::single_type_error(
                            location.clone(),
                            location,
                            Location::new(),
                            other,
                            JsonType::String,
                        )))
                    }
                }
            }
            3 => {
                let (first, second, third) = (&items[0], &items[1], &items[2]);
                match (first, second, third) {
                    (Value::String(first), Value::String(second), Value::String(third)) => {
                        Some(Required3Validator::compile(
                            first.clone(),
                            second.clone(),
                            third.clone(),
                            location,
                        ))
                    }
                    (Value::String(_), Value::String(_), other)
                    | (Value::String(_), other, _)
                    | (other, _, _) => Some(Err(ValidationError::single_type_error(
                        location.clone(),
                        location,
                        Location::new(),
                        other,
                        JsonType::String,
                    ))),
                }
            }
            _ => Some(RequiredValidator::compile(items, location)),
        },
        _ => Some(Err(ValidationError::single_type_error(
            location.clone(),
            location,
            Location::new(),
            schema,
            JsonType::Array,
        ))),
    }
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use serde_json::{json, Value};
    use test_case::test_case;

    #[test_case(&json!({"required": ["a"]}), &json!({}), "/required")]
    #[test_case(&json!({"required": ["a", "b"]}), &json!({}), "/required")]
    #[test_case(&json!({"required": ["a", "b", "c"]}), &json!({}), "/required")]
    fn location(schema: &Value, instance: &Value, expected: &str) {
        tests_util::assert_schema_location(schema, instance, expected);
    }

    // Required2Validator tests
    #[test_case(&json!({"a": 1, "b": 2}), true)]
    #[test_case(&json!({"a": 1, "b": 2, "c": 3}), true)]
    #[test_case(&json!({"a": 1}), false)]
    #[test_case(&json!({"b": 2}), false)]
    #[test_case(&json!({}), false)]
    #[test_case(&json!([1, 2]), true)] // Non-object passes
    fn required_2(instance: &Value, expected: bool) {
        let schema = json!({"required": ["a", "b"]});
        let validator = crate::validator_for(&schema).unwrap();
        assert_eq!(validator.is_valid(instance), expected);
    }

    // Required3Validator tests
    #[test_case(&json!({"a": 1, "b": 2, "c": 3}), true)]
    #[test_case(&json!({"a": 1, "b": 2, "c": 3, "d": 4}), true)]
    #[test_case(&json!({"a": 1, "b": 2}), false)]
    #[test_case(&json!({"a": 1, "c": 3}), false)]
    #[test_case(&json!({"b": 2, "c": 3}), false)]
    #[test_case(&json!({}), false)]
    #[test_case(&json!("string"), true)] // Non-object passes
    fn required_3(instance: &Value, expected: bool) {
        let schema = json!({"required": ["a", "b", "c"]});
        let validator = crate::validator_for(&schema).unwrap();
        assert_eq!(validator.is_valid(instance), expected);
    }

    #[test]
    fn required_2_iter_errors() {
        let schema = json!({"required": ["a", "b"]});
        let validator = crate::validator_for(&schema).unwrap();

        // Missing both
        let instance = json!({});
        let errors: Vec<_> = validator.iter_errors(&instance).collect();
        assert_eq!(errors.len(), 2);

        // Missing one
        let instance = json!({"a": 1});
        let errors: Vec<_> = validator.iter_errors(&instance).collect();
        assert_eq!(errors.len(), 1);

        // All present
        let instance = json!({"a": 1, "b": 2});
        let errors: Vec<_> = validator.iter_errors(&instance).collect();
        assert!(errors.is_empty());
    }

    #[test]
    fn required_3_iter_errors() {
        let schema = json!({"required": ["a", "b", "c"]});
        let validator = crate::validator_for(&schema).unwrap();

        // Missing all
        let instance = json!({});
        let errors: Vec<_> = validator.iter_errors(&instance).collect();
        assert_eq!(errors.len(), 3);

        // Missing two
        let instance = json!({"a": 1});
        let errors: Vec<_> = validator.iter_errors(&instance).collect();
        assert_eq!(errors.len(), 2);

        // Missing one
        let instance = json!({"a": 1, "b": 2});
        let errors: Vec<_> = validator.iter_errors(&instance).collect();
        assert_eq!(errors.len(), 1);

        // All present
        let instance = json!({"a": 1, "b": 2, "c": 3});
        let errors: Vec<_> = validator.iter_errors(&instance).collect();
        assert!(errors.is_empty());
    }

    #[test]
    fn trace_required2_has_schema_path() {
        let schema = serde_json::json!({"required": ["a", "b"]});
        let validator = crate::validator_for(&schema).unwrap();
        let instance = serde_json::json!({"a": 1, "b": 2});
        let mut schema_locations: Vec<String> = Vec::new();
        let _ = validator.trace(&instance, &mut |ctx| {
            schema_locations.push(ctx.schema_location.as_str().to_string());
        });
        assert!(
            schema_locations.iter().any(|s| s == "/required"),
            "expected /required in {schema_locations:?}"
        );
        assert!(
            schema_locations.iter().any(|s| s == "/required/0"),
            "expected /required/0 in {schema_locations:?}"
        );
        assert!(
            schema_locations.iter().any(|s| s == "/required/1"),
            "expected /required/1 in {schema_locations:?}"
        );
    }

    #[test]
    fn trace_required3_has_schema_path() {
        let schema = serde_json::json!({"required": ["a", "b", "c"]});
        let validator = crate::validator_for(&schema).unwrap();
        let instance = serde_json::json!({"a": 1, "b": 2, "c": 3});
        let mut schema_locations: Vec<String> = Vec::new();
        let _ = validator.trace(&instance, &mut |ctx| {
            schema_locations.push(ctx.schema_location.as_str().to_string());
        });
        assert!(
            schema_locations.iter().any(|s| s == "/required"),
            "expected /required in {schema_locations:?}"
        );
        assert!(
            schema_locations.iter().any(|s| s == "/required/2"),
            "expected /required/2 in {schema_locations:?}"
        );
    }
}
