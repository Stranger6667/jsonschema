use serde_json::Value;

use super::codegen::CodeGenerator;

pub(super) fn compile(codegen: &mut CodeGenerator, schema: &Value) {
    if let Some(Value::Array(subschemas)) = schema.get("allOf") {
        codegen.start_and_scope();
        codegen.enter_location("allOf");
        for (idx, subschema) in subschemas.iter().enumerate() {
            codegen.enter_location(idx);
            codegen.compile_schema(subschema);
            codegen.short_circuit_and();
            codegen.exit_location();
        }
        codegen.end_and_scope();
        codegen.short_circuit_and();
        codegen.exit_location();
    }
    if let Some(Value::Array(subschemas)) = schema.get("anyOf") {
        codegen.start_or_scope();
        codegen.enter_location("anyOf");
        for (idx, subschema) in subschemas.iter().enumerate() {
            codegen.enter_location(idx);
            codegen.compile_schema(subschema);
            codegen.short_circuit_or();
            codegen.exit_location();
        }
        codegen.end_or_scope();
        codegen.short_circuit_and();
        codegen.exit_location();
    }
    if let Some(Value::Array(subschemas)) = schema.get("oneOf") {
        codegen.start_xor_scope();
        codegen.enter_location("oneOf");
        match subschemas.as_slice() {
            [subschema, rest @ ..] => {
                codegen.enter_location(0);
                codegen.compile_schema(subschema);
                codegen.emit_set_one_valid();
                codegen.exit_location();
                for (idx, subschema) in rest.iter().enumerate() {
                    codegen.enter_location(idx + 1);
                    codegen.compile_schema(subschema);
                    codegen.short_circuit_xor();
                    codegen.exit_location();
                }
            }
            [] => unreachable!(),
        };
        codegen.end_xor_scope();
        codegen.short_circuit_and();
        codegen.exit_location();
    }
}
