use crate::codegen::emit::ValueEmitter;
use quote::{format_ident, quote};
use serde_json::{Map, Value};

use crate::context::CompileContext;

use super::{
    super::{
        compile_schema,
        draft::DraftExt,
        stack_emit::{pop_recursive_item_eval, push_recursive_item_eval},
        CompiledExpr,
    },
    unevaluated::{compile_index_evaluated_expr, GuardHoist},
};

/// Compile the "unevaluatedItems" keyword.
pub(crate) fn compile<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    schema: &Map<String, Value>,
) -> Option<CompiledExpr> {
    let err_instance = E::err_instance(format_ident!("instance"));
    if !ctx.supports_unevaluated_items() {
        return None;
    }

    let unevaluated = schema.get("unevaluatedItems")?;
    if unevaluated.as_bool() == Some(true) {
        return None;
    }
    let node = E::node_param(None);
    let array = E::array_param();

    let mut hoist = GuardHoist::hoisting();
    let evaluated_expr = compile_index_evaluated_expr(ctx, schema, &mut hoist);
    let guard_bindings = hoist.bindings();
    let unevaluated_check = if unevaluated.as_bool() == Some(false) {
        quote! { false }
    } else {
        let schema_check = ctx.with_instance_scope(|ctx| compile_schema(ctx, unevaluated));
        quote! { (|instance: #node| #schema_check)(item) }
    };

    let schema_path = ctx.schema_path_for_keyword("unevaluatedItems");
    let iter_expr = E::array_iter(format_ident!("arr"));
    let unexpected_item = E::node_to_json_string(format_ident!("item"));
    if ctx.uses_recursive_ref
        && ctx.draft.supports_recursive_ref_keyword()
        && schema.get("$recursiveAnchor").and_then(Value::as_bool) == Some(true)
    {
        let root_item_eval_ident =
            proc_macro2::Ident::new("__root_item_eval", proc_macro2::Span::call_site());
        let recursive_push = push_recursive_item_eval(&root_item_eval_ident, true);
        let recursive_pop = pop_recursive_item_eval();
        let is_valid = quote! {
            {
                let __root_item_eval: fn(
                    #node,
                    #array,
                    usize,
                    #node
                ) -> bool = |instance, arr, idx, item| { #(#guard_bindings)* #evaluated_expr };
                #recursive_push
                let __result = #iter_expr.enumerate().all(|(idx, item)| {
                    if __root_item_eval(instance, arr, idx, item) {
                        true
                    } else {
                        #unevaluated_check
                    }
                });
                #recursive_pop
                __result
            }
        };
        let validate = E::if_array(
            format_ident!("instance"),
            quote! {
                let __root_item_eval: fn(
                    #node,
                    #array,
                    usize,
                    #node
                ) -> bool = |instance, arr, idx, item| { #(#guard_bindings)* #evaluated_expr };
                #recursive_push
                let mut __unexpected: Vec<String> = Vec::new();
                for (idx, item) in #iter_expr.enumerate() {
                    if !__root_item_eval(instance, arr, idx, item) && !(#unevaluated_check) {
                        __unexpected.push(#unexpected_item);
                    }
                }
                #recursive_pop
                if !__unexpected.is_empty() {
                    return Some(__err::unevaluated_items(
                        #schema_path, __path.into(), #err_instance, __unexpected,
                    ));
                }
            },
        );
        Some(CompiledExpr::with_validate_blocks(is_valid, validate))
    } else {
        let all_expr = quote! {
            #iter_expr.enumerate().all(|(idx, item)| {
                if #evaluated_expr {
                    true
                } else {
                    #unevaluated_check
                }
            })
        };
        let is_valid = if guard_bindings.is_empty() {
            all_expr
        } else {
            quote! { { #(#guard_bindings)* #all_expr } }
        };
        let validate = E::if_array(
            format_ident!("instance"),
            quote! {
                #(#guard_bindings)*
                let mut __unexpected: Vec<String> = Vec::new();
                for (idx, item) in #iter_expr.enumerate() {
                    if !(#evaluated_expr) && !(#unevaluated_check) {
                        __unexpected.push(#unexpected_item);
                    }
                }
                if !__unexpected.is_empty() {
                    return Some(__err::unevaluated_items(
                        #schema_path, __path.into(), #err_instance, __unexpected,
                    ));
                }
            },
        );
        Some(CompiledExpr::with_validate_blocks(is_valid, validate))
    }
}
