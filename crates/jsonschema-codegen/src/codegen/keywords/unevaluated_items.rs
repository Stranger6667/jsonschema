use quote::quote;
use serde_json::{Map, Value};

use crate::context::CompileContext;

use super::{
    super::{
        compile_schema,
        stack_emit::{pop_recursive_item_eval, push_recursive_item_eval},
        supports_recursive_ref_keyword, supports_unevaluated_items_keyword_for_context,
        CompiledExpr,
    },
    unevaluated::compile_index_evaluated_expr,
};

/// Compile the "unevaluatedItems" keyword.
pub(crate) fn compile(
    ctx: &mut CompileContext<'_>,
    schema: &Map<String, Value>,
) -> Option<CompiledExpr> {
    if !supports_unevaluated_items_keyword_for_context(ctx) {
        return None;
    }

    let unevaluated = schema.get("unevaluatedItems")?;
    if unevaluated.as_bool() == Some(true) {
        return None;
    }
    let emit_symbols = ctx.config.backend.emit_symbols();
    let value_ty = emit_symbols.value_ty();
    let value_slice_ty = emit_symbols.value_slice_ty();

    let evaluated_expr = compile_index_evaluated_expr(ctx, schema);
    let unevaluated_check = if unevaluated.as_bool() == Some(false) {
        quote! { false }
    } else {
        let schema_check = compile_schema(ctx, unevaluated);
        quote! { (|instance: &#value_ty| #schema_check)(item) }
    };

    let schema_path = ctx.schema_path_for_keyword("unevaluatedItems");
    if ctx.uses_recursive_ref
        && supports_recursive_ref_keyword(ctx.draft)
        && schema.get("$recursiveAnchor").and_then(Value::as_bool) == Some(true)
    {
        let root_item_eval_ident =
            proc_macro2::Ident::new("__root_item_eval", proc_macro2::Span::call_site());
        let recursive_push = push_recursive_item_eval(&root_item_eval_ident, true);
        let recursive_pop = pop_recursive_item_eval();
        Some(CompiledExpr::from_bool_expr(
            quote! {
                {
                    let __root_item_eval: fn(
                        &#value_ty,
                        &#value_slice_ty,
                        usize,
                        &#value_ty
                    ) -> bool = |instance, arr, idx, item| { #evaluated_expr };
                    #recursive_push
                    let __result = arr.iter().enumerate().all(|(idx, item)| {
                        if __root_item_eval(instance, arr, idx, item) {
                            true
                        } else {
                            #unevaluated_check
                        }
                    });
                    #recursive_pop
                    __result
                }
            },
            &schema_path,
        ))
    } else {
        Some(CompiledExpr::from_bool_expr(
            quote! {
                arr.iter().enumerate().all(|(idx, item)| {
                    if #evaluated_expr {
                        true
                    } else {
                        #unevaluated_check
                    }
                })
            },
            &schema_path,
        ))
    }
}
