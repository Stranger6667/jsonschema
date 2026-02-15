use quote::quote;
use serde_json::{Map, Value};

use crate::context::CompileContext;

use super::{
    super::{
        compile_schema,
        draft::DraftExt,
        stack_emit::{pop_recursive_key_eval, push_recursive_key_eval},
        CompiledExpr,
    },
    unevaluated::compile_key_evaluated_expr,
};

/// Compile the "unevaluatedProperties" keyword.
pub(crate) fn compile(
    ctx: &mut CompileContext<'_>,
    schema: &Map<String, Value>,
) -> Option<CompiledExpr> {
    let unevaluated = schema.get("unevaluatedProperties")?;
    if unevaluated.as_bool() == Some(true) {
        return None;
    }
    let value_ty = crate::codegen::emit_serde::value_ty();
    let map_ty = crate::codegen::emit_serde::map_ty();
    let key_as_str = crate::codegen::emit_serde::key_as_str(quote! { __key });

    let uses_recursive_stack = ctx.uses_recursive_ref
        && ctx.draft.supports_recursive_ref_keyword()
        && schema.get("$recursiveAnchor").and_then(Value::as_bool) == Some(true);
    let evaluated_expr = compile_key_evaluated_expr(ctx, schema, uses_recursive_stack);
    if !uses_recursive_stack && evaluated_expr.to_string() == "true" {
        return None;
    }
    let unevaluated_check = if unevaluated.as_bool() == Some(false) {
        quote! { false }
    } else {
        let schema_check = ctx.with_instance_scope(|ctx| compile_schema(ctx, unevaluated));
        quote! {
            (|instance: &#value_ty| #schema_check)(__value)
        }
    };

    let schema_path = ctx.schema_path_for_keyword("unevaluatedProperties");
    if uses_recursive_stack {
        let root_key_eval_ident =
            proc_macro2::Ident::new("__root_key_eval", proc_macro2::Span::call_site());
        let recursive_push = push_recursive_key_eval(&root_key_eval_ident, true);
        let recursive_pop = pop_recursive_key_eval();
        let is_valid = quote! {
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
        };
        let validate = quote! {
            if let #value_ty::Object(obj) = instance {
                let __root_key_eval: fn(
                    &#value_ty,
                    &#map_ty,
                    &str
                ) -> bool = |instance, obj, key_str| { #evaluated_expr };
                #recursive_push
                let mut __unexpected: Vec<String> = Vec::new();
                for (__key, __value) in obj.iter() {
                    let key_str = #key_as_str;
                    if !__root_key_eval(instance, obj, key_str) && !(#unevaluated_check) {
                        __unexpected.push(__key.clone());
                    }
                }
                #recursive_pop
                if !__unexpected.is_empty() {
                    return Some(jsonschema::__private::error::unevaluated_properties(
                        #schema_path, __path.into(), instance, __unexpected,
                    ));
                }
            }
        };
        Some(CompiledExpr::with_validate_blocks(is_valid, validate))
    } else {
        let is_valid = quote! {
            obj.iter().all(|(__key, __value)| {
                let key_str = #key_as_str;
                if #evaluated_expr {
                    true
                } else {
                    #unevaluated_check
                }
            })
        };
        let validate = quote! {
            if let #value_ty::Object(obj) = instance {
                let mut __unexpected: Vec<String> = Vec::new();
                for (__key, __value) in obj.iter() {
                    let key_str = #key_as_str;
                    if !(#evaluated_expr) && !(#unevaluated_check) {
                        __unexpected.push(__key.clone());
                    }
                }
                if !__unexpected.is_empty() {
                    return Some(jsonschema::__private::error::unevaluated_properties(
                        #schema_path, __path.into(), instance, __unexpected,
                    ));
                }
            }
        };
        Some(CompiledExpr::with_validate_blocks(is_valid, validate))
    }
}
