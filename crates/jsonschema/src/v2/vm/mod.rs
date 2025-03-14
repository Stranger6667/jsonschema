mod types;

use std::borrow::Cow;

use serde_json::Value;
use smallvec::SmallVec;
#[cfg(feature = "internal-debug")]
mod tracker;

use super::{
    compiler::{instructions::Instruction, Program},
    error::{ValidationError, ValidationErrorKind},
};

#[derive(Debug, Clone)]
pub(crate) struct SchemaEvaluationVM<'a> {
    values: SmallVec<[&'a Value; 8]>,
    #[cfg(feature = "internal-debug")]
    tracker: tracker::EvaluationTracker,
}

impl Default for SchemaEvaluationVM<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> SchemaEvaluationVM<'a> {
    pub fn new() -> Self {
        Self {
            values: SmallVec::new(),
            #[cfg(feature = "internal-debug")]
            tracker: tracker::EvaluationTracker::new(),
        }
    }

    #[inline(always)]
    fn reset(&mut self) {
        self.values.clear();
        #[cfg(feature = "internal-debug")]
        self.tracker.reset();
    }

    pub fn is_valid(&mut self, program: &Program, instance: &'a Value) -> bool {
        self.reset();

        let mut pc = 0;
        let mut top = instance;
        let mut last = true;

        let instructions = &program.instructions;

        while let Some(instr) = instructions.get(pc) {
            #[cfg(feature = "internal-debug")]
            self.tracker.track(instr);
            match instr {
                Instruction::TypeInteger { .. } => {
                    last = types::is_integer(top);
                    pc += 1;
                }
                Instruction::MinimumU64 { inner, .. } => {
                    if let Value::Number(value) = top {
                        last = inner.is_valid(value)
                    }
                    pc += 1;
                }
            }
        }
        #[cfg(feature = "internal-debug")]
        self.tracker.report();
        last
    }

    pub fn validate(
        &mut self,
        program: &Program,
        instance: &'a Value,
    ) -> Result<(), ValidationError<'a>> {
        self.reset();

        let mut pc = 0;
        let mut top = instance;
        let mut last = Ok(());

        let instructions = &program.instructions;

        while let Some(instr) = instructions.get(pc) {
            #[cfg(feature = "internal-debug")]
            self.tracker.track(instr);
            match instr {
                Instruction::TypeInteger { .. } => {
                    if !types::is_integer(top) {
                        last = Err(ValidationError {
                            instance: Cow::Borrowed(top),
                            kind: ValidationErrorKind::Type,
                            schema_path: instructions
                                .get_location(pc)
                                .expect("Instruction not found"),
                        })
                    }
                    pc += 1;
                }
                Instruction::MinimumU64 { inner, .. } => {
                    if let Value::Number(value) = top {
                        if !inner.is_valid(value) {
                            last = Err(ValidationError {
                                instance: Cow::Borrowed(top),
                                kind: ValidationErrorKind::Minimum,
                                schema_path: instructions
                                    .get_location(pc)
                                    .expect("Instruction not found"),
                            })
                        }
                    }
                    pc += 1;
                }
            }
        }
        #[cfg(feature = "internal-debug")]
        self.tracker.report();
        last
    }
}

#[cfg_attr(feature = "internal-debug", derive(Debug))]
#[derive(Clone)]
pub struct ErrorIterator<'a, 'b> {
    pc: u32,
    top: &'a Value,
    program: &'b Program,
}

impl<'a, 'b> ErrorIterator<'a, 'b> {
    pub(crate) fn new(instance: &'a Value, program: &'b Program) -> Self {
        Self {
            pc: 0,
            top: instance,
            program,
        }
    }
}

impl<'a> Iterator for ErrorIterator<'a, '_> {
    type Item = ValidationError<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let instructions = &self.program.instructions;

        while let Some(instr) = instructions.get(self.pc) {
            match instr {
                Instruction::TypeInteger { .. } => {
                    if types::is_integer(self.top) {
                        self.pc += 1;
                    } else {
                        let schema_path = instructions
                            .get_location(self.pc)
                            .expect("Instruction not found");
                        self.pc += 1;
                        return Some(ValidationError {
                            instance: Cow::Borrowed(self.top),
                            kind: ValidationErrorKind::Type,
                            schema_path,
                        });
                    }
                }
                Instruction::MinimumU64 { inner, .. } => {
                    if let Value::Number(value) = self.top {
                        if inner.is_valid(value) {
                            self.pc += 1;
                        } else {
                            let schema_path = instructions
                                .get_location(self.pc)
                                .expect("Instruction not found");
                            self.pc += 1;
                            return Some(ValidationError {
                                instance: Cow::Borrowed(self.top),
                                kind: ValidationErrorKind::Minimum,
                                schema_path,
                            });
                        }
                    }
                }
            }
        }
        None
    }
}
