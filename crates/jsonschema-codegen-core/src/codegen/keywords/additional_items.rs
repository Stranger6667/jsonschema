use super::super::{compile_schema, expr::ValidateBlock, CompileContext, CompiledExpr};
use quote::quote;
use serde_json::Value;

pub(crate) fn compile(
    ctx: &mut CompileContext<'_>,
    items: &Value,
    schema: Option<&Value>,
) -> Option<CompiledExpr> {
    let tuple_len = if let Some(Value::Array(items)) = schema {
        items.len()
    } else {
        return None;
    };
    let schema_path = ctx.schema_path_for_keyword("additionalItems");
    match items {
        Value::Bool(false) => {
            let check = quote! { arr.len() <= #tuple_len };
            let validate = quote! {
                if !(#check) {
                    return Some(jsonschema::__private::error::additional_items(
                        #schema_path, __path.into(), instance, #tuple_len,
                    ));
                }
            };
            Some(CompiledExpr::with_validate_blocks(check, validate))
        }
        Value::Bool(true) => None,
        schema => {
            let compiled = ctx.with_schema_path_segment("additionalItems", |ctx| {
                ctx.with_instance_scope(|ctx| compile_schema(ctx, schema))
            });
            if compiled.is_trivially_true() {
                return None;
            }
            let is_valid = compiled.is_valid_token_stream();
            match &compiled.validate {
                ValidateBlock::Expr(expr) => Some(CompiledExpr::with_validate_blocks(
                    quote! { arr.iter().skip(#tuple_len).all(|instance| #is_valid) },
                    quote! {
                        for (idx, item) in arr.iter().enumerate().skip(#tuple_len) {
                            let instance = item;
                            let __path = &__path.push(idx);
                            #expr
                        }
                    },
                )),
                ValidateBlock::AlwaysValid => None,
            }
        }
    }
}
