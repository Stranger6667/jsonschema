mod types;

use std::borrow::Cow;

use serde_json::Value;
use smallvec::SmallVec;

use super::{
    compiler::{Instruction, Program},
    error::{ValidationError, ValidationErrorKind},
};

#[derive(Debug, Clone)]
pub(crate) struct SchemaEvaluationVM<'a> {
    values: SmallVec<[&'a Value; 8]>,
    #[cfg(feature = "internal-debug")]
    tracer: EvaluationTracer,
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
            tracer: EvaluationTracer::new(),
        }
    }

    #[cfg(feature = "internal-debug")]
    fn record_instruction(&mut self, instruction: &Instruction) {
        self.tracer.push(instruction.clone());
    }

    #[cfg(feature = "internal-debug")]
    fn report_debug_info(&self) {
        println!("Total Iterations: {}", self.tracer.instructions.len());
    }

    #[inline(always)]
    fn reset(&mut self) {
        self.values.clear();
    }

    pub fn is_valid(&mut self, program: &Program, instance: &'a Value) -> bool {
        self.reset();

        let mut ip = 0;
        let mut top = instance;
        let mut last_result = true;

        let instructions = &program.instructions;

        while ip < instructions.len() {
            let instruction = &instructions[ip];
            #[cfg(feature = "internal-debug")]
            self.record_instruction(instruction);
            match instruction {
                Instruction::TypeInteger {
                    prefetch_info,
                    value0,
                    value1,
                } => {
                    last_result = types::is_integer(top);
                    ip += 1;
                }
            }
        }
        #[cfg(feature = "internal-debug")]
        self.report_debug_info();
        last_result
    }

    pub fn validate(
        &mut self,
        program: &Program,
        instance: &'a Value,
    ) -> Result<(), ValidationError<'a>> {
        self.reset();

        let mut ip = 0;
        let mut top = instance;
        let mut last = Ok(());

        let instructions = &program.instructions;
        let locations = &program.locations;

        while ip < instructions.len() {
            let instruction = &instructions[ip];
            #[cfg(feature = "internal-debug")]
            self.record_instruction(instruction);
            match instruction {
                Instruction::TypeInteger {
                    prefetch_info,
                    value0,
                    value1,
                } => {
                    if !types::is_integer(top) {
                        last = Err(ValidationError {
                            instance: Cow::Borrowed(top),
                            kind: ValidationErrorKind::Type,
                            schema_path: locations[ip].clone(),
                        })
                    }
                    ip += 1;
                }
            }
        }
        #[cfg(feature = "internal-debug")]
        self.report_debug_info();
        last
    }
}

#[derive(Debug, Clone)]
pub struct ErrorIterator<'a, 'b> {
    ip: usize,
    top: &'a Value,
    program: &'b Program,
}

impl<'a, 'b> ErrorIterator<'a, 'b> {
    pub fn new(instance: &'a Value, program: &'b Program) -> Self {
        Self {
            ip: 0,
            top: instance,
            program,
        }
    }
}

impl<'a> Iterator for ErrorIterator<'a, '_> {
    type Item = ValidationError<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let instructions = &self.program.instructions;
        let locations = &self.program.locations;

        while self.ip < instructions.len() {
            let instruction = &instructions[self.ip];
            match instruction {
                Instruction::TypeInteger {
                    prefetch_info,
                    value0,
                    value1,
                } => {
                    let ip = self.ip;
                    self.ip += 1;
                    if !types::is_integer(self.top) {
                        return Some(ValidationError {
                            instance: Cow::Borrowed(self.top),
                            kind: ValidationErrorKind::Type,
                            schema_path: locations[ip].clone(),
                        });
                    }
                }
            }
        }
        None
    }
}

#[cfg(feature = "internal-debug")]
struct EvaluationTracer {
    instructions: Vec<Instruction>,
}

#[cfg(feature = "internal-debug")]
impl EvaluationTracer {
    fn new() -> EvaluationTracer {
        EvaluationTracer {
            instructions: Vec::new(),
        }
    }
}
