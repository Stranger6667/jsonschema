use super::super::{
    compile_schema,
    errors::{invalid_schema_non_empty_array_expression, invalid_schema_type_expression},
    CompileContext, CompiledExpr,
};
use serde_json::Value;

pub(crate) fn compile(ctx: &mut CompileContext<'_>, value: &Value) -> CompiledExpr {
    let Some(schemas) = value.as_array() else {
        return invalid_schema_type_expression(value, &["array"]);
    };
    if schemas.is_empty() {
        return invalid_schema_non_empty_array_expression();
    }
    let mut compiled = Vec::with_capacity(schemas.len());
    for (idx, schema) in schemas.iter().enumerate() {
        let idx_str = idx.to_string();
        let expr = ctx.with_schema_path_segment("allOf", |ctx| {
            ctx.with_schema_path_segment(&idx_str, |ctx| compile_schema(ctx, schema))
        });
        compiled.push(expr);
    }
    CompiledExpr::combine_and(compiled)
}
