use referencing::{Draft, Registry};
use serde_json::Value;

use crate::compiler::DEFAULT_ROOT_URL;

use super::{codegen::CodeGenerator, context::CompilationContext, instructions::Instructions};

#[cfg_attr(feature = "internal-debug", derive(Debug))]
#[derive(Clone)]
pub struct Program {
    pub(crate) instructions: Instructions,
    pub(crate) constants: Vec<Value>,
}

impl Program {
    pub(crate) fn compile(schema: &Value) -> Program {
        let draft = Draft::default().detect(schema).unwrap();
        let resource_ref = draft.create_resource_ref(schema);
        let resource = draft.create_resource(schema.clone());
        let base_uri = resource_ref.id().unwrap_or(DEFAULT_ROOT_URL);

        let registry = Registry::options()
            .draft(draft)
            .build([(base_uri, resource)])
            .unwrap();

        let resolver = registry.try_resolver(base_uri).unwrap();
        let ctx = CompilationContext::new(resolver);
        let mut codegen = CodeGenerator::new();
        codegen.compile_schema(ctx, schema);
        let (instructions, constants) = codegen.finish();
        Program {
            instructions,
            constants,
        }
    }
}
