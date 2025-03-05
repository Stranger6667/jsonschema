pub(crate) mod instructions;
pub(crate) mod meta;
pub(crate) mod modes;

use instructions::maximum::Maximum;
pub(crate) use meta::NodeMetadata;
use modes::ExecutorMode;
use serde_json::Value;

use crate::{paths::LazyLocation, ValidationError};

#[derive(Debug, Clone)]
pub(crate) enum Instruction {
    MaximumU64(Maximum<u64>),
    MaximumI64(Maximum<i64>),
    MaximumF64(Maximum<f64>),
}

impl Instruction {
    pub(crate) fn execute(&self, instance: &Value) -> bool {
        dbg!(self, instance);
        match instance {
            Value::Number(n) => match self {
                Instruction::MaximumU64(max) => max.execute(n),
                Instruction::MaximumI64(max) => max.execute(n),
                Instruction::MaximumF64(max) => max.execute(n),
            },
            _ => true,
        }
    }
    pub(crate) fn create_error<'i>(
        &self,
        metadata: &NodeMetadata,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> ValidationError<'i> {
        match self {
            Instruction::MaximumU64(_)
            | Instruction::MaximumI64(_)
            | Instruction::MaximumF64(_) => ValidationError::maximum(
                metadata.location.clone(),
                location.into(),
                instance,
                metadata.keyword_value.clone(),
            ),
        }
    }
}

pub(crate) fn execute(instructions: &[Instruction], instance: &Value) -> bool {
    instructions
        .iter()
        .all(|instruction| instruction.execute(instance))
}

pub(crate) fn execute_with_metadata<'i, E: ExecutorMode>(
    instructions: &[Instruction],
    metadata: &[NodeMetadata],
    instance: &'i Value,
    location: &LazyLocation,
) -> E::Output<'i> {
    E::combine(
        instructions
            .iter()
            .zip(metadata)
            .map(|(instruction, metadata)| E::execute(instruction, metadata, instance, location)),
    )
}
