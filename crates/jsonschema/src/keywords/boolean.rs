use crate::paths::{LazyLocation, Location};

use crate::{error::ValidationError, keywords::CompilationResult, validator::Validate};
use serde_json::Value;
use std::sync::Arc;

pub(crate) struct FalseValidator {
    location: Location,
    absolute_path: Option<Arc<referencing::Uri<String>>>,
}
impl FalseValidator {
    #[inline]
    pub(crate) fn compile<'a>(
        location: Location,
        absolute_path: Option<Arc<referencing::Uri<String>>>,
    ) -> CompilationResult<'a> {
        Ok(Box::new(FalseValidator {
            location,
            absolute_path,
        }))
    }
}
impl Validate for FalseValidator {
    fn is_valid(&self, _: &Value) -> bool {
        false
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> Result<(), ValidationError<'i>> {
        Err(ValidationError::false_schema(
            self.location.clone(),
            location.into(),
            instance,
            self.absolute_path.clone(),
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
