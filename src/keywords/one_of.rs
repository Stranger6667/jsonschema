use super::{CompilationResult, Validate, Validators};
use crate::{
    compilation::{compile_validators, CompilationContext, JSONSchema},
    error::{error, no_error, CompilationError, ErrorIterator, ValidationError},
};
use serde_json::{Map, Value};

pub struct OneOfValidator {
    schemas: Vec<Validators>,
}

impl OneOfValidator {
    #[inline]
    pub(crate) fn compile(schema: &Value, context: &CompilationContext) -> CompilationResult {
        if let Value::Array(items) = schema {
            let mut schemas = Vec::with_capacity(items.len());
            for item in items {
                schemas.push(compile_validators(item, context)?)
            }
            return Ok(Box::new(OneOfValidator { schemas }));
        }
        Err(CompilationError::SchemaError)
    }

    fn get_first_valid(
        &self,
        schema: &JSONSchema,
        instance: &Value,
    ) -> (Option<&Validators>, Option<usize>) {
        let mut first_valid = None;
        let mut first_valid_idx = None;
        for (idx, validators) in self.schemas.iter().enumerate() {
            if validators
                .iter()
                .all(|validator| validator.is_valid(schema, instance))
            {
                first_valid = Some(validators);
                first_valid_idx = Some(idx);
                break;
            }
        }
        (first_valid, first_valid_idx)
    }

    fn are_others_valid(&self, schema: &JSONSchema, instance: &Value, idx: Option<usize>) -> bool {
        for validators in self.schemas.iter().skip(idx.unwrap() + 1) {
            if validators
                .iter()
                .all(|validator| validator.is_valid(schema, instance))
            {
                return true;
            }
        }
        false
    }
}

impl Validate for OneOfValidator {
    fn validate<'a>(&self, schema: &'a JSONSchema, instance: &'a Value) -> ErrorIterator<'a> {
        let (first_valid, first_valid_idx) = self.get_first_valid(schema, instance);
        if first_valid.is_none() {
            return error(ValidationError::one_of_not_valid(instance));
        }
        if self.are_others_valid(schema, instance, first_valid_idx) {
            return error(ValidationError::one_of_multiple_valid(instance));
        }
        no_error()
    }
    fn is_valid(&self, schema: &JSONSchema, instance: &Value) -> bool {
        let (first_valid, first_valid_idx) = self.get_first_valid(schema, instance);
        if first_valid.is_none() {
            return false;
        }
        !self.are_others_valid(schema, instance, first_valid_idx)
    }
    fn name(&self) -> String {
        format!("<one of: {:?}>", self.schemas)
    }
}

#[inline]
pub fn compile(
    _: &Map<String, Value>,
    schema: &Value,
    context: &CompilationContext,
) -> Option<CompilationResult> {
    Some(OneOfValidator::compile(schema, context))
}
