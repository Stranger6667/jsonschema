use super::{instructions::Instructions, numeric};
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
        let ty = schema.get("type");
        numeric::compile(self, schema);
    }

    pub(super) fn emit_integer_type(&mut self, prefetch_info: numeric::PrefetchInfo) {
        self.instructions.add_with_location(
            Instruction::TypeInteger {
                prefetch_info,
                value0: 0,
                value1: 0,
            },
            self.locations.join("type"),
        );
    }
}
