use compiler::program::Program;
use serde_json::Value;

mod compiler;
mod error;
mod ext;
mod vm;

pub use error::ValidationErrorV2;
pub use vm::{ErrorIteratorV2, SchemaEvaluationVM};

pub struct ValidatorV2 {
    pub program: Program,
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
    pub fn validate<'a>(&self, instance: &'a Value) -> Result<(), ValidationErrorV2<'a>> {
        let mut vm = vm::SchemaEvaluationVM::new();
        vm.validate(&self.program, instance)
    }
    pub fn iter_errors<'a, 'b>(&'b self, instance: &'a Value) -> ErrorIteratorV2<'a, 'b> {
        ErrorIteratorV2::new(instance, &self.program)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        compiler::{instructions::*, Program},
        vm::*,
    };
    use serde_json::{json, Value};
    use test_case::test_case;

    #[test_case(
        json!({"type": "integer"}),
        json!(42),
        json!("abc"),
        &[
            Instruction::TypeInteger,
        ],
        &["/type"],
        &[];
        "only integer type"
    )]
    #[test_case(
        json!({"minimum": 5}),
        json!(42),
        json!(3),
        &[
            Instruction::minimum(5u64.into()),
        ],
        &["/minimum"],
        &[];
        "only minimum"
    )]
    #[test_case(
        json!({"maximum": 5}),
        json!(3),
        json!(7),
        &[
            Instruction::maximum(5u64.into()),
        ],
        &["/maximum"],
        &[];
        "only maximum"
    )]
    #[test_case(
        json!({"type": "integer", "minimum": 5}),
        json!(6),
        json!(3),
        &[
            Instruction::TypeInteger,
            Instruction::minimum(5u64.into()),
        ],
        &["/type", "/minimum"],
        &[];
        "integer type + minimum"
    )]
    #[test_case(
        json!({"type": "integer", "maximum": 5}),
        json!(3),
        json!(7),
        &[
            Instruction::TypeInteger,
            Instruction::maximum(5u64.into()),
        ],
        &["/type", "/maximum"],
        &[];
        "integer type + maximum"
    )]
    #[test_case(
        json!({"type": "integer", "multipleOf": 5}),
        json!(10),
        json!(7),
        &[
            Instruction::TypeInteger,
            Instruction::multiple_of(5.0.into()),
        ],
        &["/type", "/multipleOf"],
        &[];
        "integer type + multipleOf"
    )]
    #[test_case(
        json!({"minimum": 5, "maximum": 10}),
        json!(7),
        json!(12),
        &[
            Instruction::minimum(5u64.into()),
            Instruction::maximum(10u64.into()),
        ],
        &["/minimum", "/maximum"],
        &[];
        "minimum + maximum"
    )]
    #[test_case(
        json!({"type": "integer", "minimum": 5, "maximum": 10}),
        json!(7),
        json!(12),
        &[
            Instruction::TypeInteger,
            Instruction::minimum(5u64.into()),
            Instruction::maximum(10u64.into()),
        ],
        &["/type", "/minimum", "/maximum"],
        &[];
        "integer type + minimum + maximum"
    )]
    #[test_case(
        json!({"allOf": [{"minimum": 1}, {"maximum": 10}]}),
        json!(7),
        json!(12),
        &[
            Instruction::minimum(1u64.into()),
            Instruction::JumpIfFalseOrPop(4),
            Instruction::maximum(10u64.into()),
            Instruction::JumpIfFalseOrPop(4),
        ],
        &["/allOf/0/minimum", "", "/allOf/1/maximum", ""],
        &[];
        "allOf + minimum + maximum"
    )]
    #[test_case(
        json!({"anyOf": [{"anyOf": [{"maximum": 10}]}]}),
        json!(7),
        json!(12),
        &[
            Instruction::maximum(10u64.into()),
            Instruction::JumpIfTrueOrPop(2),
            Instruction::JumpIfTrueOrPop(3),
        ],
        &["/anyOf/0/anyOf/0/maximum", "", ""],
        &[];
        "nested anyOf"
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
        assert_eq!(program.instructions.instructions, instructions);
        assert_eq!(program.instructions.locations, locations);
        assert_eq!(program.constants, constants);
        let mut vm = SchemaEvaluationVM::new();
        assert!(vm.is_valid(&program, &valid));
        assert!(!vm.is_valid(&program, &invalid));
        assert!(vm.validate(&program, &valid).is_ok());
        assert!(vm.validate(&program, &invalid).is_err());
        assert!(ErrorIteratorV2::new(&valid, &program).next().is_none());
        assert!(ErrorIteratorV2::new(&invalid, &program).next().is_some());
    }
}
