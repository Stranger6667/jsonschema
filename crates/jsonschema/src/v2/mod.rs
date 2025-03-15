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
        compiler::{instructions::*, numeric::*, Program},
        vm::*,
    };
    use serde_json::{json, Value};
    use test_case::test_case;

    #[test_case(
        json!({"type": "integer"}),
        json!(42),
        json!("abc"),
        &[
            Instruction::type_integer(PrefetchInfo::new(), [0, 0]),
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
            Instruction::minimum(PrefetchInfo::from_unchecked(0b100000000000000), 5u64.into(), 0),
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
            Instruction::maximum(PrefetchInfo::from_unchecked(0b000100000000000), 5u64.into(), 0),
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
            Instruction::type_integer(PrefetchInfo::from_unchecked(0b100000000000000), [5, 0]),
            Instruction::minimum(PrefetchInfo::from_unchecked(0b100000000000000), 5u64.into(), 0),
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
            Instruction::type_integer(PrefetchInfo::from_unchecked(0b000100000000000), [5, 0]),
            Instruction::maximum(PrefetchInfo::from_unchecked(0b000100000000000), 5u64.into(), 0),
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
            Instruction::type_integer(PrefetchInfo::from_unchecked(0b000000000000100), [5, 0]),
            Instruction::multiple_of(PrefetchInfo::from_unchecked(0b000000000000100), 5.0.into(), 0),
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
            Instruction::minimum(PrefetchInfo::from_unchecked(0b100100000000000), 5u64.into(), 10),
            Instruction::maximum(PrefetchInfo::from_unchecked(0b100100000000000), 10u64.into(), 0),
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
            Instruction::type_integer(PrefetchInfo::from_unchecked(0b100100000000000), [5, 10]),
            Instruction::minimum(PrefetchInfo::from_unchecked(0b100100000000000), 5u64.into(), 10),
            Instruction::maximum(PrefetchInfo::from_unchecked(0b100100000000000), 10u64.into(), 0),
        ],
        &["/type", "/minimum", "/maximum"],
        &[];
        "integer type + minimum + maximum"
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
