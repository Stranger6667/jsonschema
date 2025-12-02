use crate::paths::{LazyLocation, Location, RefTracker};

use crate::{
    error::ValidationError,
    keywords::CompilationResult,
    validator::{Validate, ValidationContext},
};
use serde_json::Value;

pub(crate) struct FalseValidator {
    location: Location,
}
impl FalseValidator {
    #[inline]
    pub(crate) fn compile<'a>(location: Location) -> CompilationResult<'a> {
        Ok(Box::new(FalseValidator { location }))
    }
}
impl Validate for FalseValidator {
    fn is_valid(&self, _: &Value, _ctx: &mut ValidationContext) -> bool {
        false
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        _ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        Err(ValidationError::false_schema(
            self.location.clone(),
            crate::paths::capture_evaluation_path(tracker, &self.location),
            location.into(),
            instance,
        ))
    }
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use serde_json::json;

    #[test]
    fn location() {
        tests_util::assert_schema_location(&json!(false), &json!(1), "");
    }
}
