use crate::{
    compiler,
    error::ValidationError,
    keywords::CompilationResult,
    paths::{LazyLocation, Location},
    primitive_type::PrimitiveType,
    validator::Validate,
};
use fraction::{BigFraction, BigUint};
use serde_json::{Map, Value};

pub(crate) struct MultipleOfFloatValidator {
    multiple_of: f64,
    location: Location,
}

impl MultipleOfFloatValidator {
    #[inline]
    pub(crate) fn compile<'a>(multiple_of: f64, location: Location) -> CompilationResult<'a> {
        Ok(Box::new(MultipleOfFloatValidator {
            multiple_of,
            location,
        }))
    }
}

impl Validate for MultipleOfFloatValidator {
    fn is_valid(&self, instance: &Value) -> bool {
        if let Value::Number(item) = instance {
            let item = item.as_f64().expect("Always valid");
            let remainder = (item / self.multiple_of) % 1.;
            if remainder.is_nan() {
                // Involves heap allocations via the underlying `BigUint` type
                let fraction = BigFraction::from(item) / BigFraction::from(self.multiple_of);
                if let Some(denom) = fraction.denom() {
                    denom == &BigUint::from(1_u8)
                } else {
                    true
                }
            } else {
                remainder < f64::EPSILON
            }
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> Result<(), ValidationError<'i>> {
        if !self.is_valid(instance) {
            return Err(ValidationError::multiple_of(
                self.location.clone(),
                location.into(),
                instance,
                self.multiple_of,
            ));
        }
        Ok(())
    }
}

pub(crate) struct MultipleOfIntegerValidator {
    multiple_of: f64,
    location: Location,
}

impl MultipleOfIntegerValidator {
    #[inline]
    pub(crate) fn compile<'a>(multiple_of: f64, location: Location) -> CompilationResult<'a> {
        Ok(Box::new(MultipleOfIntegerValidator {
            multiple_of,
            location,
        }))
    }
}

impl Validate for MultipleOfIntegerValidator {
    fn is_valid(&self, instance: &Value) -> bool {
        if let Value::Number(item) = instance {
            let item = item.as_f64().expect("Always valid");
            // As the divisor has its fractional part as zero, then any value with a non-zero
            // fractional part can't be a multiple of this divisor, therefore it is short-circuited
            item.fract() == 0. && (item % self.multiple_of) == 0.
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> Result<(), ValidationError<'i>> {
        if !self.is_valid(instance) {
            return Err(ValidationError::multiple_of(
                self.location.clone(),
                location.into(),
                instance,
                self.multiple_of,
            ));
        }
        Ok(())
    }
}

#[inline]
pub(crate) fn compile<'a>(
    ctx: &compiler::Context,
    _: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>> {
    if let Value::Number(multiple_of) = schema {
        let multiple_of = multiple_of.as_f64().expect("Always valid");
        let location = ctx.location().join("multipleOf");
        if multiple_of.fract() == 0. {
            Some(MultipleOfIntegerValidator::compile(multiple_of, location))
        } else {
            Some(MultipleOfFloatValidator::compile(multiple_of, location))
        }
    } else {
        Some(Err(ValidationError::single_type_error(
            Location::new(),
            ctx.location().clone(),
            schema,
            PrimitiveType::Number,
        )))
    }
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use serde_json::{json, Value};
    use test_case::test_case;

    #[test_case(&json!({"multipleOf": 2}), &json!(4))]
    #[test_case(&json!({"multipleOf": 1.0}), &json!(4.0))]
    #[test_case(&json!({"multipleOf": 1.5}), &json!(3.0))]
    #[test_case(&json!({"multipleOf": 1.5}), &json!(4.5))]
    fn multiple_of_is_valid(schema: &Value, instance: &Value) {
        tests_util::is_valid(schema, instance)
    }

    #[test_case(&json!({"multipleOf": 1.0}), &json!(4.5))]
    fn multiple_of_is_not_valid(schema: &Value, instance: &Value) {
        tests_util::is_not_valid(schema, instance)
    }

    #[test_case(&json!({"multipleOf": 2}), &json!(3), "/multipleOf")]
    #[test_case(&json!({"multipleOf": 1.5}), &json!(5), "/multipleOf")]
    fn location(schema: &Value, instance: &Value, expected: &str) {
        tests_util::assert_schema_location(schema, instance, expected)
    }
}
