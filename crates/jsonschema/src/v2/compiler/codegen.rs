use super::{instructions::Instructions, numeric, types::JsonTypeSet};
use serde_json::Value;

use super::{instructions::Instruction, location::LocationContext};

pub(super) type Constants = Vec<Value>;

/// Provides a way to generate a program for the VM.
pub(crate) struct CodeGenerator {
    instructions: Instructions,
    locations: LocationContext,
    constants: Vec<Value>,
}

impl CodeGenerator {
    pub(super) fn new() -> Self {
        Self {
            instructions: Instructions::new(),
            locations: LocationContext::new(),
            constants: Vec::new(),
        }
    }

    pub(super) fn finish(self) -> (Instructions, Constants) {
        (self.instructions, self.constants)
    }

    pub(super) fn compile_schema(&mut self, schema: &Value) {
        let types = if let Some(types) = schema.get("type") {
            JsonTypeSet::from_value(types)
        } else {
            JsonTypeSet::new()
        };
        numeric::compile(self, types, schema);
    }

    pub(super) fn emit_integer_type(
        &mut self,
        prefetch: numeric::PrefetchInfo,
        data: numeric::InlineData2x,
    ) {
        self.instructions.add_with_location(
            Instruction::type_integer(prefetch, data),
            self.locations.join("type"),
        );
    }

    pub(super) fn emit_minimum(
        &mut self,
        prefetch: numeric::PrefetchInfo,
        value: numeric::NumericValue,
        data: numeric::InlineData1x,
    ) {
        self.instructions.add_with_location(
            Instruction::minimum(prefetch, value, data),
            self.locations.join("minimum"),
        );
    }
}
