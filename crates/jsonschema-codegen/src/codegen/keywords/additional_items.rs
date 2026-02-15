use super::super::{compile_schema, expr::ValidateBlock, CompileContext, CompiledExpr};
use quote::quote;
use serde_json::Value;

pub(crate) fn compile(
    ctx: &mut CompileContext<'_>,
    additional_items: &Value,
    items_schema: Option<&Value>,
) -> Option<CompiledExpr> {
    let tuple_len = if let Some(Value::Array(items)) = items_schema {
        items.len()
    } else {
        return None;
    };
    let schema_path = ctx.schema_path_for_keyword("additionalItems");
    match additional_items {
        Value::Bool(false) => Some(CompiledExpr::from_bool_expr(
            quote! { arr.len() <= #tuple_len },
            &schema_path,
        )),
        Value::Bool(true) => None,
        schema => {
            let compiled =
                ctx.with_schema_path_segment("additionalItems", |ctx| compile_schema(ctx, schema));
            if compiled.is_trivially_true() {
                return None;
            }
            let is_valid_ts = compiled.is_valid_ts();
            match (&compiled.validate, &compiled.iter_errors) {
                (ValidateBlock::Expr(v), ValidateBlock::Expr(ie)) => {
                    Some(CompiledExpr::with_validate_blocks(
                        quote! { arr.iter().skip(#tuple_len).all(|instance| #is_valid_ts) },
                        quote! {
                            for (idx, item) in arr.iter().enumerate().skip(#tuple_len) {
                                let instance = item;
                                let __path = __path.join(idx);
                                #v
                            }
                        },
                        quote! {
                            for (idx, item) in arr.iter().enumerate().skip(#tuple_len) {
                                let instance = item;
                                let __path = __path.join(idx);
                                #ie
                            }
                        },
                    ))
                }
                (ValidateBlock::AlwaysValid, ValidateBlock::AlwaysValid) => None,
                _ => Some(CompiledExpr::from_bool_expr(
                    quote! { arr.iter().skip(#tuple_len).all(|instance| #is_valid_ts) },
                    &schema_path,
                )),
            }
        }
    }
}
