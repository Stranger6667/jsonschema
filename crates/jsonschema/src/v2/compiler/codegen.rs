use crate::paths::LocationSegment;

use super::{
    combinators,
    instructions::{Instruction, Instructions},
    location::LocationContext,
    numeric,
    types::JsonTypeSet,
};
use serde_json::Value;

pub(super) type Constants = Vec<Value>;

/// Provides a way to generate a program for the VM.
pub(crate) struct CodeGenerator {
    pub(super) instructions: Instructions,
    locations: LocationContext,
    pending_scopes: Vec<PendingScope>,
    constants: Vec<Value>,
}

enum PendingScope {
    And { jumps: Vec<u32> },
    Or { jumps: Vec<u32> },
    Xor { jumps: Vec<u32> },
}

macro_rules! define_emit_fn {
    ($( $fn_name:ident => $instr_name:ident, $location:literal ),* $(,)?) => {
        $(
            pub(super) fn $fn_name(&mut self, value: numeric::NumericValue) {
                self.instructions.add_with_location(
                    Instruction::$instr_name(value),
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
            pending_scopes: Vec::new(),
            constants: Vec::new(),
        }
    }

    pub(super) fn finish(self) -> (Instructions, Constants) {
        (self.instructions, self.constants)
    }

    /// Return the next instruction index.
    pub(super) fn next_instruction(&self) -> u32 {
        self.instructions.len() as u32
    }

    pub(super) fn compile_schema(&mut self, schema: &Value) {
        match schema {
            Value::Bool(true) => self.emit_true(),
            Value::Bool(false) => self.emit_false(),
            Value::Object(obj) if obj.is_empty() => self.emit_true(),
            Value::Object(_) => {
                let types = if let Some(types) = schema.get("type") {
                    JsonTypeSet::from_value(types)
                } else {
                    JsonTypeSet::new()
                };
                combinators::compile(self, schema);
                numeric::compile(self, types, schema);
            }
            _ => todo!(),
        }
    }
    pub(super) fn enter_location<'a>(&mut self, key: impl Into<LocationSegment<'a>>) {
        self.locations.push(key);
    }
    pub(super) fn exit_location(&mut self) {
        self.locations.pop();
    }
    pub(super) fn start_all_of(&mut self) {
        self.pending_scopes
            .push(PendingScope::And { jumps: Vec::new() });
        self.enter_location("allOf");
    }
    pub(super) fn end_all_of(&mut self) {
        let end = self.next_instruction();
        if let Some(PendingScope::And { jumps }) = self.pending_scopes.pop() {
            for instr in jumps {
                match self.instructions.get_mut(instr) {
                    Some(&mut Instruction::JumpIfFalseOrPop(ref mut target)) => {
                        *target = end;
                    }
                    _ => unreachable!(),
                }
            }
        }
        self.exit_location();
    }
    pub(super) fn start_any_of(&mut self) {
        self.pending_scopes
            .push(PendingScope::Or { jumps: Vec::new() });
        self.enter_location("anyOf");
    }
    pub(super) fn end_any_of(&mut self) {
        let end = self.next_instruction();
        if let Some(PendingScope::Or { jumps }) = self.pending_scopes.pop() {
            for instr in jumps {
                match self.instructions.get_mut(instr) {
                    Some(&mut Instruction::JumpIfTrueOrPop(ref mut target)) => {
                        *target = end;
                    }
                    _ => unreachable!(),
                }
            }
        }
        self.exit_location();
    }
    pub(super) fn start_one_of(&mut self) {
        self.pending_scopes
            .push(PendingScope::Xor { jumps: Vec::new() });
        self.enter_location("oneOf");
        self.emit_push_one_of();
    }
    pub(super) fn end_one_of(&mut self) {
        let end = self.next_instruction();
        if let Some(PendingScope::Xor { jumps }) = self.pending_scopes.pop() {
            for instr in jumps {
                match self.instructions.get_mut(instr) {
                    Some(&mut Instruction::JumpIfTrueTrueOrPop(ref mut target)) => {
                        *target = end;
                    }
                    _ => unreachable!(),
                }
            }
        }
        self.emit_pop_one_of();
        self.exit_location();
    }
    pub(super) fn short_circuit_all_of(&mut self) {
        if let Some(&mut PendingScope::And { ref mut jumps }) = self.pending_scopes.last_mut() {
            jumps.push(self.instructions.add(Instruction::JumpIfFalseOrPop(!0)));
        } else {
            unreachable!();
        }
    }
    pub(super) fn short_circuit_any_of(&mut self) {
        if let Some(&mut PendingScope::Or { ref mut jumps }) = self.pending_scopes.last_mut() {
            jumps.push(self.instructions.add(Instruction::JumpIfTrueOrPop(!0)));
        } else {
            unreachable!();
        }
    }
    pub(super) fn short_circuit_one_of(&mut self) {
        if let Some(&mut PendingScope::Xor { ref mut jumps }) = self.pending_scopes.last_mut() {
            jumps.push(self.instructions.add(Instruction::JumpIfTrueTrueOrPop(!0)));
        } else {
            unreachable!();
        }
    }
    pub(super) fn emit_push_one_of(&mut self) {
        self.instructions.add(Instruction::PushOneOf);
    }
    pub(super) fn emit_set_one_valid(&mut self) {
        self.instructions.add(Instruction::SetOneValid);
    }
    pub(super) fn emit_pop_one_of(&mut self) {
        self.instructions.add(Instruction::PopOneOf);
    }
    pub(super) fn emit_true(&mut self) {
        self.instructions
            .add_with_location(Instruction::True, self.locations.top());
    }
    pub(super) fn emit_false(&mut self) {
        self.instructions
            .add_with_location(Instruction::False, self.locations.top());
    }
    pub(super) fn emit_number_type(&mut self) {
        self.instructions
            .add_with_location(Instruction::TypeNumber, self.locations.join("type"));
    }
    pub(super) fn emit_integer_type(&mut self) {
        self.instructions
            .add_with_location(Instruction::TypeInteger, self.locations.join("type"));
    }

    define_emit_fn!(
        emit_minimum => minimum, "minimum",
        emit_maximum => maximum, "maximum",
        emit_exclusive_minimum => exclusive_minimum, "exclusiveMinimum",
        emit_exclusive_maximum => exclusive_maximum, "exclusiveMaximum",
        emit_multiple_of => multiple_of, "multipleOf",
    );
}
