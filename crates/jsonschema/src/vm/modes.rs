use serde_json::Value;

use crate::{paths::LazyLocation, ValidationError};

use super::{meta::NodeMetadata, Instruction};

pub(crate) trait ExecutorMode {
    type Output<'i>;

    fn execute<'i>(
        instruction: &Instruction,
        metadata: &NodeMetadata,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> Self::Output<'i>;

    fn combine<'i>(results: impl Iterator<Item = Self::Output<'i>>) -> Self::Output<'i>;
}

pub(crate) struct ValidateMode;

impl ExecutorMode for ValidateMode {
    type Output<'i> = Result<(), ValidationError<'i>>;

    fn execute<'i>(
        instruction: &Instruction,
        metadata: &NodeMetadata,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> Self::Output<'i> {
        if instruction.execute(instance) {
            Ok(())
        } else {
            Err(instruction.create_error(metadata, instance, location))
        }
    }

    fn combine<'i>(results: impl Iterator<Item = Self::Output<'i>>) -> Self::Output<'i> {
        for result in results {
            result?;
        }
        Ok(())
    }
}
