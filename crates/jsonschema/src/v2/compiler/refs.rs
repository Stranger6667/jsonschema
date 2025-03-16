use serde_json::Value;

use super::codegen::CodeGenerator;

pub(super) fn compile(codegen: &mut CodeGenerator, schema: &Value) {
    if let Some(Value::String(reference)) = schema.get("$ref") {
        // TODO:
        //   - Base URI is also needed to detect compiled ones
        if let Some(id) = codegen.subroutines.get(reference) {
            codegen.emit_call(id);
            return;
        }
        // TODO: cycle detection - insert into the currently compiling list.
        let id = codegen.compile_subroutine(reference);
        codegen.emit_call(id);
    }
}
