use super::{instructions::Instructions, numeric, types::JsonTypeSet};
use serde_json::Value;

use super::{instructions::Instruction, location::LocationContext};

pub(super) type Constants = Vec<Value>;

/// Provides a way to generate a program for the VM.
pub(crate) struct CodeGenerator {
    pub(super) instructions: Instructions,
    locations: LocationContext,
    constants: Vec<Value>,
}

macro_rules! define_emit_fn {
    ($( $fn_name:ident => $instr_name:ident, $location:literal ),* $(,)?) => {
        $(
            pub(super) fn $fn_name(
                &mut self,
                prefetch: numeric::PrefetchInfo,
                value: numeric::NumericValue,
                data: numeric::InlineData1x,
            ) {
                self.instructions.add_with_location(
                    Instruction::$instr_name(prefetch, value, data),
                    self.locations.join($location),
                );
            }
        )*
    };
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

    pub(super) fn emit_number_type(
        &mut self,
        prefetch: numeric::PrefetchInfo,
        data: numeric::InlineData2x,
    ) {
        self.instructions.add_with_location(
            Instruction::type_number(prefetch, data),
            self.locations.join("type"),
        );
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

    define_emit_fn!(
        emit_minimum => minimum, "minimum",
        emit_maximum => maximum, "maximum",
        emit_exclusive_minimum => exclusive_minimum, "exclusiveMinimum",
        emit_exclusive_maximum => exclusive_maximum, "exclusiveMaximum",
        emit_multiple_of => multiple_of, "multipleOf",
    );
}
