use crate::{
    compiler,
    error::{no_error, ErrorIterator, ValidationError},
    keywords::CompilationResult,
    paths::{LazyLocation, Location, RefTracker},
    types::JsonType,
    validator::{Validate, ValidationContext},
};
use ahash::AHashSet;
use serde_json::{Map, Value};

pub(crate) struct RequiredValidator {
    required: Vec<String>,
    required_set: AHashSet<String>,
    location: Location,
}

impl RequiredValidator {
    #[inline]
    pub(crate) fn compile(items: &[Value], location: Location) -> CompilationResult<'_> {
        let mut required = Vec::with_capacity(items.len());
        let mut required_set = AHashSet::with_capacity(items.len());
        for item in items {
            match item {
                Value::String(string) => {
                    required.push(string.clone());
                    required_set.insert(string.clone());
                }
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
        Ok(Box::new(RequiredValidator {
            required,
            required_set,
            location,
        }))
    }
}

impl Validate for RequiredValidator {
    fn is_valid(&self, instance: &Value, _ctx: &mut ValidationContext) -> bool {
        if let Value::Object(item) = instance {
            if item.len() < self.required.len() {
                return false;
            }
            let mut found = 0;
            for key in item.keys() {
                if self.required_set.contains(key) {
                    found += 1;
                    if found == self.required.len() {
                        return true;
                    }
                }
            }
            false
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

#[inline]
pub(crate) fn compile<'a>(
    ctx: &compiler::Context,
    _: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>> {
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
        Value::Array(items) => {
            if items.len() == 1 {
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
            } else {
                Some(RequiredValidator::compile(items, location))
            }
        }
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
    fn location(schema: &Value, instance: &Value, expected: &str) {
        tests_util::assert_schema_location(schema, instance, expected);
    }
}
