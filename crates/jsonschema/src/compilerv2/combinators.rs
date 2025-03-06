use serde_json::{Map, Value};

use super::{EvaluationScope, Instruction, SchemaCompiler};

pub(super) fn compile(compiler: &mut SchemaCompiler, obj: &Map<String, Value>) {
    if let Some(Value::Array(value)) = obj.get("anyOf") {
        compile_impl(compiler, value, EvaluationScope::OrSearching);
    }
    if let Some(Value::Array(value)) = obj.get("allOf") {
        compile_impl(compiler, value, EvaluationScope::AndValid);
    }
    if let Some(Value::Array(value)) = obj.get("oneOf") {
        compile_impl(compiler, value, EvaluationScope::XorEmpty);
    }
}

fn compile_impl(compiler: &mut SchemaCompiler, schemas: &[Value], scope: EvaluationScope) {
    assert!(!schemas.is_empty());
    compiler.emit(Instruction::PushScope(scope));
    for schema in schemas {
        compiler.compile_impl(schema);
    }
    compiler.emit(Instruction::PopScope);
}
