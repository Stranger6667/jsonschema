use serde_json::Value;

use super::{
    codegen::{CodeGenerator, Scope},
    context::CompilationContext,
};

pub(super) fn compile(
    codegen: &mut CodeGenerator,
    ctx: &mut CompilationContext<'_>,
    schema: &Value,
) {
    if let Some(Value::Array(subschemas)) = schema.get("allOf") {
        codegen.start_scope(Scope::And);
        codegen.enter_location("allOf");
        for (idx, subschema) in subschemas.iter().enumerate() {
            codegen.enter_location(idx);
            codegen.compile_schema(ctx, subschema);
            codegen.short_circuit();
            codegen.exit_location();
        }
        codegen.end_scope();
        codegen.short_circuit();
        codegen.exit_location();
    }
    if let Some(Value::Array(subschemas)) = schema.get("anyOf") {
        codegen.start_scope(Scope::Or);
        codegen.enter_location("anyOf");
        for (idx, subschema) in subschemas.iter().enumerate() {
            codegen.enter_location(idx);
            codegen.compile_schema(ctx, subschema);
            codegen.short_circuit();
            codegen.exit_location();
        }
        codegen.end_scope();
        codegen.short_circuit();
        codegen.exit_location();
    }
    if let Some(Value::Array(subschemas)) = schema.get("oneOf") {
        codegen.start_scope(Scope::Xor);
        codegen.emit_push_one_of();
        codegen.enter_location("oneOf");
        match subschemas.as_slice() {
            [subschema, rest @ ..] => {
                codegen.enter_location(0);
                codegen.compile_schema(ctx, subschema);
                codegen.emit_set_one_valid();
                codegen.exit_location();
                for (idx, subschema) in rest.iter().enumerate() {
                    codegen.enter_location(idx + 1);
                    codegen.compile_schema(ctx, subschema);
                    codegen.short_circuit();
                    codegen.exit_location();
                }
            }
            [] => unreachable!(),
        };
        codegen.end_scope();
        codegen.emit_pop_one_of();
        codegen.short_circuit();
        codegen.exit_location();
    }
}
