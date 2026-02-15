use proc_macro2::TokenStream;
use quote::quote;
use serde_json::{Map, Value};

use crate::{
    codegen::{
        compile_key_evaluated_expr, compile_schema,
        stack_emit::{pop_recursive_key_eval, push_recursive_key_eval},
        supports_recursive_ref_keyword, supports_unevaluated_properties_keyword_for_context,
    },
    context::CompileContext,
};

pub(in crate::codegen) fn compile_unevaluated_properties(
    ctx: &mut CompileContext<'_>,
    schema: &Map<String, Value>,
) -> Option<TokenStream> {
    if !supports_unevaluated_properties_keyword_for_context(ctx) {
        return None;
    }
    let unevaluated = schema.get("unevaluatedProperties")?;
    if unevaluated.as_bool() == Some(true) {
        return None;
    }
    let emit_symbols = ctx.config.backend.emit_symbols();
    let value_ty = emit_symbols.value_ty();
    let map_ty = emit_symbols.map_ty();
    let key_as_str = ctx.config.backend.key_as_str(quote! { __key });

    let evaluated_expr = compile_key_evaluated_expr(ctx, schema);
    let unevaluated_check = if unevaluated.as_bool() == Some(false) {
        quote! { false }
    } else {
        let schema_check = compile_schema(ctx, unevaluated);
        quote! {
            (|instance: &#value_ty| #schema_check)(__value)
        }
    };

    if ctx.uses_recursive_ref
        && supports_recursive_ref_keyword(ctx.draft)
        && schema.get("$recursiveAnchor").and_then(Value::as_bool) == Some(true)
    {
        let root_key_eval_ident =
            proc_macro2::Ident::new("__root_key_eval", proc_macro2::Span::call_site());
        let recursive_push = push_recursive_key_eval(&root_key_eval_ident, true);
        let recursive_pop = pop_recursive_key_eval();
        Some(quote! {
            {
                let __root_key_eval: fn(
                    &#value_ty,
                    &#map_ty,
                    &str
                ) -> bool = |instance, obj, key_str| { #evaluated_expr };
                #recursive_push
                let __result = obj.iter().all(|(__key, __value)| {
                    let key_str = #key_as_str;
                    if __root_key_eval(instance, obj, key_str) {
                        true
                    } else {
                        #unevaluated_check
                    }
                });
                #recursive_pop
                __result
            }
        })
    } else {
        Some(quote! {
            obj.iter().all(|(__key, __value)| {
                let key_str = #key_as_str;
                if #evaluated_expr {
                    true
                } else {
                    #unevaluated_check
                }
            })
        })
    }
}
