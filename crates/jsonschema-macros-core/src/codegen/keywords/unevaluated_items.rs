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
    unevaluated::{compile_index_evaluated_expr, GuardHoist, ReportGuards},
};

/// Code pushing to `__unevaluated` the positions of the items of an array `instance` that `schema`
/// leaves unevaluated, as its errors count them.
pub(crate) fn unevaluated_items<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    schema: &Map<String, Value>,
) -> proc_macro2::TokenStream {
    // The keyword's own subschema evaluates items for a parent schema only.
    let mut others = schema.clone();
    others.remove("unevaluatedItems");
    let mut report = ReportGuards::default();
    let mut hoist = GuardHoist::hoisting();
    let reported = compile_index_evaluated_expr(ctx, &others, &mut hoist, Some(&mut report));
    let (bindings, cells) = (hoist.bindings(), report.cells());
    let iter = E::array_iter(format_ident!("arr"));
    E::if_array(
        format_ident!("instance"),
        quote! {
            #(#bindings)*
            #(#cells)*
            for (idx, item) in #iter.enumerate() {
                if !(#reported) {
                    __unevaluated.push(idx);
                }
            }
        },
    )
}

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
    let evaluated_expr = compile_index_evaluated_expr(ctx, schema, &mut hoist, None);
    let guard_bindings = hoist.bindings();
    // Errors count a subschema the schema applies in place only where it validates, which needs
    // its own expression when the schema has one.
    let mut report = ReportGuards::default();
    let mut report_hoist = GuardHoist::hoisting();
    let reported = [
        "$ref",
        "$dynamicRef",
        "$recursiveRef",
        "allOf",
        "then",
        "else",
    ]
    .iter()
    .any(|keyword| schema.contains_key(*keyword))
    .then(|| compile_index_evaluated_expr(ctx, schema, &mut report_hoist, Some(&mut report)));
    let (reported_expr, reported_bindings) = match &reported {
        Some(expr) if !report.is_empty() => (expr, report_hoist.bindings()),
        _ => (&evaluated_expr, guard_bindings),
    };
    let report_cells = report.cells();
    let unevaluated_check = if unevaluated.as_bool() == Some(false) {
        quote! { false }
    } else {
        let schema_check = compile_schema(ctx, unevaluated);
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
        let first_error = E::if_array(
            format_ident!("instance"),
            quote! {
                let __root_item_eval: fn(
                    #node,
                    #array,
                    usize,
                    #node
                ) -> bool = |instance, arr, idx, item| { #(#guard_bindings)* #evaluated_expr };
                #recursive_push
                #(#reported_bindings)*
                #(#report_cells)*
                let mut __unexpected: Vec<String> = Vec::new();
                for (idx, item) in #iter_expr.enumerate() {
                    if !(#reported_expr) && !(#unevaluated_check) {
                        __unexpected.push(#unexpected_item);
                    }
                }
                #recursive_pop
                if !__unexpected.is_empty() {
                    break '__first_error Some(__err::unevaluated_items(
                        #schema_path, __path.into(), #err_instance, __unexpected,
                    ));
                }
            },
        );
        Some(CompiledExpr::from_first_error(is_valid, first_error))
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
        let first_error = E::if_array(
            format_ident!("instance"),
            quote! {
                #(#reported_bindings)*
                #(#report_cells)*
                let mut __unexpected: Vec<String> = Vec::new();
                for (idx, item) in #iter_expr.enumerate() {
                    if !(#reported_expr) && !(#unevaluated_check) {
                        __unexpected.push(#unexpected_item);
                    }
                }
                if !__unexpected.is_empty() {
                    break '__first_error Some(__err::unevaluated_items(
                        #schema_path, __path.into(), #err_instance, __unexpected,
                    ));
                }
            },
        );
        Some(CompiledExpr::from_first_error(is_valid, first_error))
    }
}
