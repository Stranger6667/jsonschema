use super::super::{compile_schema, expr::ValidateBlock, CompileContext, CompiledExpr};
use quote::quote;
use serde_json::Value;

/// Returns the compiled check and the prefix length, or `None` if the value is
/// not a non-empty array.
pub(crate) fn compile(
    ctx: &mut CompileContext<'_>,
    value: &Value,
) -> Option<(CompiledExpr, usize)> {
    let Value::Array(schemas) = value else {
        return None;
    };
    let prefix_len = schemas.len();
    if prefix_len == 0 {
        return None;
    }
    let schema_path = ctx.schema_path_for_keyword("prefixItems");
    let compiled: Vec<CompiledExpr> = schemas
        .iter()
        .enumerate()
        .map(|(idx, schema)| {
            let idx_str = idx.to_string();
            let validation = ctx.with_schema_path_segment("prefixItems", |ctx| {
                ctx.with_schema_path_segment(&idx_str, |ctx| compile_schema(ctx, schema))
            });
            if validation.is_trivially_true() {
                return CompiledExpr::always_true();
            }
            let is_valid_ts = validation.is_valid_ts();
            match (&validation.validate, &validation.iter_errors) {
                (ValidateBlock::Expr(v), ValidateBlock::Expr(ie)) => {
                    CompiledExpr::with_validate_blocks(
                        quote! { arr.get(#idx).map_or(true, |instance| #is_valid_ts) },
                        quote! {
                            if let Some(instance) = arr.get(#idx) {
                                let __path = __path.join(#idx);
                                #v
                            }
                        },
                        quote! {
                            if let Some(instance) = arr.get(#idx) {
                                let __path = __path.join(#idx);
                                #ie
                            }
                        },
                    )
                }
                (ValidateBlock::AlwaysValid, ValidateBlock::AlwaysValid) => {
                    CompiledExpr::always_true()
                }
                _ => CompiledExpr::from_bool_expr(
                    quote! { arr.get(#idx).map_or(true, |instance| #is_valid_ts) },
                    &schema_path,
                ),
            }
        })
        .collect();
    Some((CompiledExpr::combine_and(compiled), prefix_len))
}
