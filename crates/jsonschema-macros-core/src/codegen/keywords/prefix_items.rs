use super::super::{compile_schema, expr::ValidateBlock, CompileContext, CompiledExpr};
use crate::codegen::emit::ValueEmitter;
use quote::{format_ident, quote};
use serde_json::Value;

/// Returns the compiled check and the prefix length, or `None` if the value is
/// not a non-empty array.
pub(crate) fn compile<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    value: &Value,
) -> Option<(CompiledExpr, usize)> {
    let Value::Array(schemas) = value else {
        return None;
    };
    let prefix_len = schemas.len();
    if prefix_len == 0 {
        return None;
    }
    let compiled: Vec<CompiledExpr> = schemas
        .iter()
        .enumerate()
        .map(|(idx, schema)| {
            let idx_str = idx.to_string();
            let validation = ctx.with_schema_path_segment("prefixItems", |ctx| {
                ctx.with_schema_path_segment(&idx_str, |ctx| {
                    ctx.with_instance_scope(|ctx| compile_schema(ctx, schema))
                })
            });
            if validation.is_trivially_true() {
                return CompiledExpr::always_true();
            }
            let is_valid = validation.is_valid_token_stream();
            match &validation.validate {
                ValidateBlock::Expr(expr) => {
                    let child_collect = validation.collect.as_token_stream();
                    let get_expr = E::array_get(format_ident!("arr"), idx);
                    CompiledExpr::with_validate_and_collect_blocks(
                        quote! { #get_expr.map_or(true, |instance| #is_valid) },
                        quote! {
                            if let Some(instance) = #get_expr {
                                let __path = &__path.push(#idx);
                                #expr
                            }
                        },
                        quote! {
                            if let Some(instance) = #get_expr {
                                if !(#is_valid) {
                                    let __path = &__path.push(#idx);
                                    #child_collect
                                }
                            }
                        },
                    )
                }
                ValidateBlock::AlwaysValid => CompiledExpr::always_true(),
            }
        })
        .collect();
    Some((CompiledExpr::combine_and(compiled), prefix_len))
}
