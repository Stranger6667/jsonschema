use serde_json::Value;
use smallvec::SmallVec;
#[cfg(feature = "internal-debug")]
mod tracker;

use super::{
    compiler::{instructions::Instruction, Program},
    error::ValidationErrorV2,
    ext::numeric,
};

#[derive(Debug, Clone)]
pub struct SchemaEvaluationVM<'a> {
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

        macro_rules! is_valid_number {
            ($inner:expr) => {{
                if let Value::Number(value) = top {
                    last = $inner.is_valid(value);
                }
                pc += 1;
            }};
        }

        while let Some(instr) = instructions.get(pc) {
            #[cfg(feature = "internal-debug")]
            self.tracker.track(instr);
            match instr {
                Instruction::TypeInteger { prefetch, data } => {
                    last = if let Value::Number(value) = top {
                        if value.is_i64() || value.is_u64() {
                            true
                            //numeric::ge(value, data[0]) || numeric::le(value, data[1])
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    //last = types::is_integer(top);
                    pc += 1;
                }
                Instruction::TypeNumber { prefetch, data } => {
                    last = if let Value::Number(value) = top {
                        true
                        //numeric::ge(value, data[0]) || numeric::le(value, data[1])
                    } else {
                        false
                    };
                    pc += 1;
                }
                Instruction::MinimumU64 { inner, .. } => {
                    is_valid_number!(inner)
                }
                Instruction::MinimumI64 { inner, .. } => {
                    is_valid_number!(inner)
                }
                Instruction::MinimumF64 { inner, .. } => {
                    is_valid_number!(inner)
                }
                Instruction::MaximumU64 { inner, .. } => {
                    is_valid_number!(inner)
                }
                Instruction::MaximumI64 { inner, .. } => {
                    is_valid_number!(inner)
                }
                Instruction::MaximumF64 { inner, .. } => {
                    is_valid_number!(inner)
                }
                Instruction::ExclusiveMinimumU64 { inner, .. } => {
                    is_valid_number!(inner)
                }
                Instruction::ExclusiveMinimumI64 { inner, .. } => {
                    is_valid_number!(inner)
                }
                Instruction::ExclusiveMinimumF64 { inner, .. } => {
                    is_valid_number!(inner)
                }
                Instruction::ExclusiveMaximumU64 { inner, .. } => {
                    is_valid_number!(inner)
                }
                Instruction::ExclusiveMaximumI64 { inner, .. } => {
                    is_valid_number!(inner)
                }
                Instruction::ExclusiveMaximumF64 { inner, .. } => {
                    is_valid_number!(inner)
                }
                Instruction::MultipleOfFloat { inner, .. } => {
                    is_valid_number!(inner)
                }
                Instruction::MultipleOfInteger { inner, .. } => {
                    is_valid_number!(inner)
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
    ) -> Result<(), ValidationErrorV2<'a>> {
        self.reset();

        let mut pc = 0;
        let mut top = instance;
        let mut last = Ok(());

        let instructions = &program.instructions;

        macro_rules! validate_number {
            ($inner:expr, $method:ident) => {{
                if let Value::Number(value) = top {
                    if !$inner.is_valid(value) {
                        let schema_path = instructions
                            .get_location(pc)
                            .expect("Instruction not found");
                        last = Err(ValidationErrorV2::$method(instance, schema_path))
                    }
                }
                pc += 1;
            }};
        }

        while let Some(instr) = instructions.get(pc) {
            #[cfg(feature = "internal-debug")]
            self.tracker.track(instr);
            match instr {
                Instruction::TypeInteger { .. } => {
                    if !numeric::is_integer(top) {
                        let schema_path = instructions
                            .get_location(pc)
                            .expect("Instruction not found");
                        last = Err(ValidationErrorV2::ty(top, schema_path));
                    }
                    pc += 1;
                }
                Instruction::TypeNumber { .. } => {
                    if !matches!(top, Value::Number(_)) {
                        let schema_path = instructions
                            .get_location(pc)
                            .expect("Instruction not found");
                        last = Err(ValidationErrorV2::ty(top, schema_path));
                    }
                    pc += 1;
                }
                Instruction::MinimumU64 { inner, .. } => {
                    validate_number!(inner, minimum)
                }
                Instruction::MinimumI64 { inner, .. } => {
                    validate_number!(inner, minimum)
                }
                Instruction::MinimumF64 { inner, .. } => {
                    validate_number!(inner, minimum)
                }
                Instruction::MaximumU64 { inner, .. } => {
                    validate_number!(inner, maximum)
                }
                Instruction::MaximumI64 { inner, .. } => {
                    validate_number!(inner, maximum)
                }
                Instruction::MaximumF64 { inner, .. } => {
                    validate_number!(inner, maximum)
                }
                Instruction::ExclusiveMinimumU64 { inner, .. } => {
                    validate_number!(inner, minimum)
                }
                Instruction::ExclusiveMinimumI64 { inner, .. } => {
                    validate_number!(inner, minimum)
                }
                Instruction::ExclusiveMinimumF64 { inner, .. } => {
                    validate_number!(inner, minimum)
                }
                Instruction::ExclusiveMaximumU64 { inner, .. } => {
                    validate_number!(inner, maximum)
                }
                Instruction::ExclusiveMaximumI64 { inner, .. } => {
                    validate_number!(inner, maximum)
                }
                Instruction::ExclusiveMaximumF64 { inner, .. } => {
                    validate_number!(inner, maximum)
                }
                Instruction::MultipleOfInteger { inner, .. } => {
                    validate_number!(inner, multiple_of)
                }
                Instruction::MultipleOfFloat { inner, .. } => {
                    validate_number!(inner, multiple_of)
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
pub struct ErrorIteratorV2<'a, 'b> {
    pc: u32,
    top: &'a Value,
    program: &'b Program,
}

impl<'a, 'b> ErrorIteratorV2<'a, 'b> {
    pub fn new(instance: &'a Value, program: &'b Program) -> Self {
        Self {
            pc: 0,
            top: instance,
            program,
        }
    }
}

impl<'a> Iterator for ErrorIteratorV2<'a, '_> {
    type Item = ValidationErrorV2<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let instructions = &self.program.instructions;

        macro_rules! next_number {
            ($inner:expr, $method:ident) => {{
                if let Value::Number(value) = self.top {
                    if $inner.is_valid(value) {
                        self.pc += 1;
                    } else {
                        let schema_path = instructions
                            .get_location(self.pc)
                            .expect("Instruction not found");
                        self.pc += 1;
                        return Some(ValidationErrorV2::$method(self.top, schema_path));
                    }
                }
            }};
        }

        while let Some(instr) = instructions.get(self.pc) {
            match instr {
                Instruction::TypeInteger { .. } => {
                    if numeric::is_integer(self.top) {
                        self.pc += 1;
                    } else {
                        let schema_path = instructions
                            .get_location(self.pc)
                            .expect("Instruction not found");
                        self.pc += 1;
                        return Some(ValidationErrorV2::ty(self.top, schema_path));
                    }
                }
                Instruction::TypeNumber { .. } => {
                    if matches!(self.top, Value::Number(_)) {
                        self.pc += 1;
                    } else {
                        let schema_path = instructions
                            .get_location(self.pc)
                            .expect("Instruction not found");
                        self.pc += 1;
                        return Some(ValidationErrorV2::ty(self.top, schema_path));
                    }
                }
                Instruction::MinimumU64 { inner, .. } => {
                    next_number!(inner, minimum)
                }
                Instruction::MinimumI64 { inner, .. } => {
                    next_number!(inner, minimum)
                }
                Instruction::MinimumF64 { inner, .. } => {
                    next_number!(inner, minimum)
                }
                Instruction::MaximumU64 { inner, .. } => {
                    next_number!(inner, maximum)
                }
                Instruction::MaximumI64 { inner, .. } => {
                    next_number!(inner, maximum)
                }
                Instruction::MaximumF64 { inner, .. } => {
                    next_number!(inner, maximum)
                }
                Instruction::ExclusiveMinimumU64 { inner, .. } => {
                    next_number!(inner, minimum)
                }
                Instruction::ExclusiveMinimumI64 { inner, .. } => {
                    next_number!(inner, minimum)
                }
                Instruction::ExclusiveMinimumF64 { inner, .. } => {
                    next_number!(inner, minimum)
                }
                Instruction::ExclusiveMaximumU64 { inner, .. } => {
                    next_number!(inner, maximum)
                }
                Instruction::ExclusiveMaximumI64 { inner, .. } => {
                    next_number!(inner, maximum)
                }
                Instruction::ExclusiveMaximumF64 { inner, .. } => {
                    next_number!(inner, maximum)
                }
                Instruction::MultipleOfInteger { inner, .. } => {
                    next_number!(inner, multiple_of)
                }
                Instruction::MultipleOfFloat { inner, .. } => {
                    next_number!(inner, multiple_of)
                }
            }
        }
        None
    }
}
