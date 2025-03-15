use serde_json::Value;

use super::codegen::CodeGenerator;

pub(super) fn compile(codegen: &mut CodeGenerator, schema: &Value) {
    if let Some(Value::Array(subschemas)) = schema.get("allOf") {
        codegen.start_all_of();
        for (idx, subschema) in subschemas.iter().enumerate() {
            codegen.enter_location(idx);
            codegen.compile_schema(subschema);
            codegen.short_circuit_all_of();
            codegen.exit_location();
        }
        codegen.end_all_of();
    }
    if let Some(Value::Array(subschemas)) = schema.get("anyOf") {
        codegen.start_any_of();
        for (idx, subschema) in subschemas.iter().enumerate() {
            codegen.enter_location(idx);
            codegen.compile_schema(subschema);
            codegen.short_circuit_any_of();
            codegen.exit_location();
        }
        codegen.end_any_of();
    }
    if let Some(Value::Array(subschemas)) = schema.get("oneOf") {
        codegen.start_one_of();
        match subschemas.as_slice() {
            [subschema, rest @ ..] => {
                codegen.enter_location(0);
                codegen.compile_schema(subschema);
                codegen.emit_set_one_valid();
                codegen.exit_location();
                for (idx, subschema) in rest.iter().enumerate() {
                    codegen.enter_location(idx + 1);
                    codegen.compile_schema(subschema);
                    codegen.short_circuit_one_of();
                    codegen.exit_location();
                }
            }
            [] => unreachable!(),
        };
        codegen.end_one_of();
    }
}
