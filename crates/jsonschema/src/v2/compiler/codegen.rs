use crate::paths::LocationSegment;

use super::{
    combinators,
    context::CompilationContext,
    instructions::{Instruction, Instructions},
    location::LocationContext,
    numeric, refs,
    subroutines::{SubroutineId, Subroutines},
    types::{self, JsonType, JsonTypeSet},
};
use referencing::{Registry, Resolver};
use serde_json::Value;

pub(super) type Constants = Vec<Value>;

/// Provides a way to generate a program for the VM.
pub(crate) struct CodeGenerator {
    pub(super) instructions: Instructions,
    locations: LocationContext,
    pending_scopes: Vec<PendingScope>,
    pub(super) subroutines: Subroutines,
    constants: Vec<Value>,
}

pub(super) enum Scope {
    And,
    Or,
    Xor,
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
            subroutines: Subroutines::new(),
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

    pub(super) fn compile_schema(&mut self, ctx: CompilationContext<'_>, schema: &Value) {
        match schema {
            Value::Bool(true) => self.emit_true(),
            Value::Bool(false) => self.emit_false(),
            Value::Object(obj) if obj.is_empty() => self.emit_true(),
            Value::Object(_) => {
                self.start_scope(Scope::And);
                refs::compile(self, ctx.clone(), schema);
                types::compile(self, schema);
                combinators::compile(self, ctx.clone(), schema);
                numeric::compile(self, schema);
                self.end_scope();
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

    pub(super) fn start_scope(&mut self, scope: Scope) {
        let pending = match scope {
            Scope::And => PendingScope::And { jumps: Vec::new() },
            Scope::Or => PendingScope::Or { jumps: Vec::new() },
            Scope::Xor => PendingScope::Xor { jumps: Vec::new() },
        };
        self.pending_scopes.push(pending);
    }
    pub(super) fn short_circuit(&mut self) {
        match self.pending_scopes.last_mut().expect("Missing scope") {
            PendingScope::And { jumps } => {
                jumps.push(self.instructions.add(Instruction::JumpIfFalseOrPop(!0)));
            }
            PendingScope::Or { jumps } => {
                jumps.push(self.instructions.add(Instruction::JumpIfTrueOrPop(!0)));
            }
            PendingScope::Xor { jumps } => {
                jumps.push(self.instructions.add(Instruction::JumpIfTrueTrueOrPop(!0)));
            }
        }
    }
    pub(super) fn end_scope(&mut self) {
        let end = self.next_instruction();

        macro_rules! update_jumps {
            ($jumps:expr, $variant:ident) => {
                for instr in $jumps {
                    match self.instructions.get_mut(instr) {
                        Some(Instruction::$variant(ref mut target)) => {
                            *target = end;
                        }
                        _ => unreachable!(),
                    }
                }
            };
        }

        match self.pending_scopes.pop().expect("Missing scope") {
            PendingScope::And { jumps } => {
                update_jumps!(jumps, JumpIfFalseOrPop)
            }
            PendingScope::Or { jumps } => {
                update_jumps!(jumps, JumpIfTrueOrPop)
            }
            PendingScope::Xor { jumps } => {
                update_jumps!(jumps, JumpIfTrueTrueOrPop)
            }
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
    pub(super) fn emit_type(&mut self, ty: JsonType) {
        let instr = match ty {
            JsonType::Array => Instruction::TypeArray,
            JsonType::Boolean => Instruction::TypeBoolean,
            JsonType::Integer => Instruction::TypeInteger,
            JsonType::Null => Instruction::TypeNull,
            JsonType::Number => Instruction::TypeNumber,
            JsonType::Object => Instruction::TypeObject,
            JsonType::String => Instruction::TypeString,
        };
        self.instructions
            .add_with_location(instr, self.locations.join("type"));
    }
    pub(super) fn emit_types(&mut self, types: JsonTypeSet) {
        self.instructions
            .add_with_location(Instruction::TypeSet(types), self.locations.join("type"));
    }

    define_emit_fn!(
        emit_minimum => minimum, "minimum",
        emit_maximum => maximum, "maximum",
        emit_exclusive_minimum => exclusive_minimum, "exclusiveMinimum",
        emit_exclusive_maximum => exclusive_maximum, "exclusiveMaximum",
        emit_multiple_of => multiple_of, "multipleOf",
    );

    pub(crate) fn compile_subroutine(
        &mut self,
        ctx: CompilationContext<'_>,
        reference: &str,
    ) -> SubroutineId {
        let id = self.subroutines.get_next_id(reference);
        let resolved = ctx.resolver().lookup(reference).unwrap();
        dbg!(&resolved);
        // TODO: Should it be a different resolver in that context?
        self.subroutines.set_in_progress(id);
        // TODO: it should be compiled into a different instance of `Instructions`
        // TODO: Where to emit RETURN?
        self.compile_schema(ctx, resolved.contents());
        self.subroutines.unset_in_progress(id);

        id
    }

    pub(crate) fn emit_call(&mut self, id: SubroutineId) {
        self.instructions.add(Instruction::Call(id));
    }
    pub(crate) fn emit_return(&mut self) {
        self.instructions.add(Instruction::Return);
    }
}
