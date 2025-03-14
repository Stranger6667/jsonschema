use compiler::Program;
use error::ValidationError;
use serde_json::Value;
use vm::ErrorIterator;

mod compiler;
mod error;
mod instructions;
mod vm;

pub struct ValidatorV2 {
    program: Program,
}

impl ValidatorV2 {
    pub fn new(schema: &Value) -> ValidatorV2 {
        ValidatorV2 {
            program: Program::compile(schema),
        }
    }
    pub fn is_valid(&self, instance: &Value) -> bool {
        let mut vm = vm::SchemaEvaluationVM::new();
        vm.is_valid(&self.program, instance)
    }
    pub fn validate<'a>(&self, instance: &'a Value) -> Result<(), ValidationError<'a>> {
        let mut vm = vm::SchemaEvaluationVM::new();
        vm.validate(&self.program, instance)
    }
    pub fn iter_errors<'a, 'b>(&'b self, instance: &'a Value) -> ErrorIterator<'a, 'b> {
        ErrorIterator::new(instance, &self.program)
    }
}

#[cfg(test)]
mod tests {
    use super::compiler::*;
    use super::vm::*;
    use serde_json::json;
    use serde_json::Value;
    use test_case::test_case;

    #[test_case(
        json!({"type": "integer"}),
        json!(42),
        json!("abc"),
        &[
            Instruction::TypeInteger { prefetch_info: PrefetchInfo::new(), value0: 0, value1: 0}
        ],
        &["/type"],
        &[]
    )]
    fn test_compilation(
        schema: Value,
        valid: Value,
        invalid: Value,
        instructions: &[Instruction],
        locations: &[&str],
        constants: &[Value],
    ) {
        let program = Program::compile(&schema);
        assert_eq!(program.instructions, instructions);
        assert_eq!(program.locations, locations);
        assert_eq!(program.constants, constants);
        let mut vm = SchemaEvaluationVM::new();
        assert!(vm.is_valid(&program, &valid));
        assert!(!vm.is_valid(&program, &invalid));
        assert!(vm.validate(&program, &valid).is_ok());
        assert!(vm.validate(&program, &invalid).is_err());

        assert!(ErrorIterator::new(&valid, &program).next().is_none());
        assert!(ErrorIterator::new(&invalid, &program).next().is_some());
    }
}
