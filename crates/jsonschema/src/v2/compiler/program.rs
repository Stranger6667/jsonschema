use serde_json::Value;

use super::{codegen::CodeGenerator, instructions::Instructions};

#[cfg_attr(feature = "internal-debug", derive(Debug))]
#[derive(Clone)]
pub struct Program {
    pub(crate) instructions: Instructions,
    pub(crate) constants: Vec<Value>,
}

impl Program {
    pub(crate) fn compile(schema: &Value) -> Program {
        let mut codegen = CodeGenerator::new();
        codegen.compile_schema(schema);
        let (instructions, constants) = codegen.finish();
        Program {
            instructions,
            constants,
        }
    }
}
