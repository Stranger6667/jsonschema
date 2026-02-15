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
    for (i, s) in schemas.iter().enumerate() {
        let i_str = i.to_string();
        let expr = ctx.with_schema_path_segment("allOf", |ctx| {
            ctx.with_schema_path_segment(&i_str, |ctx| compile_schema(ctx, s))
        });
        compiled.push(expr);
    }
    CompiledExpr::combine_and(compiled)
}
