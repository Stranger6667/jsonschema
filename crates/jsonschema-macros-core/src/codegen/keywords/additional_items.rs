use super::super::{compile_schema, expr::ErrorsBlock, CompileContext, CompiledExpr};
use crate::codegen::emit::ValueEmitter;
use quote::{format_ident, quote};
use serde_json::Value;

pub(crate) fn compile<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    items: &Value,
    schema: Option<&Value>,
    max_items: Option<&Value>,
) -> Option<CompiledExpr> {
    let err_instance = E::err_instance(format_ident!("instance"));
    let tuple_len = if let Some(Value::Array(items)) = schema {
        items.len()
    } else {
        return None;
    };
    let schema_path = ctx.schema_path_for_keyword("additionalItems");
    match items {
        Value::Bool(false) => {
            if max_items
                .and_then(Value::as_u64)
                .is_some_and(|max| max <= tuple_len as u64)
            {
                return None;
            }
            let len_expr = E::array_len(format_ident!("arr"));
            let check = quote! { #len_expr <= #tuple_len };
            let first_error = quote! {
                if !(#check) {
                    return Some(__err::additional_items(
                        #schema_path, __path.into(), #err_instance, #tuple_len,
                    ));
                }
            };
            Some(CompiledExpr::from_first_error(check, first_error))
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
            match &compiled.errors {
                ErrorsBlock::Expr(_) => {
                    let child_errors = compiled.errors.as_token_stream();
                    let iter_expr = E::array_iter(format_ident!("arr"));
                    Some(CompiledExpr::with_errors(
                        quote! { #iter_expr.skip(#tuple_len).all(|instance| #is_valid) },
                        quote! {
                            for (idx, item) in #iter_expr.enumerate().skip(#tuple_len) {
                                let instance = item;
                                let __path = &__path.push(idx);
                                #child_errors
                            }
                        },
                    ))
                }
                ErrorsBlock::AlwaysValid => None,
            }
        }
    }
}
