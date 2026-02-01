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
}

#[inline]
pub(crate) fn compile<'a>(
    ctx: &compiler::Context,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>> {
    // Check if fused validator in additional_properties handles this case:
    // properties + additionalProperties: false + required: [single_item]
    // Also check there's no patternProperties (which uses different validators)
    if let Value::Array(items) = schema {
        if items.len() == 1
            && matches!(parent.get("additionalProperties"), Some(Value::Bool(false)))
            && parent.contains_key("properties")
            && !parent.contains_key("patternProperties")
        {
            // Fused validator handles this - skip separate required validation
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
}
