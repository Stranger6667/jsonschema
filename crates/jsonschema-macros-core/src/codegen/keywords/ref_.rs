use super::super::{
    expr::{call_errors, ErrorsBlock},
    helpers::{
        collect_dynamic_anchor_bindings, dynamic_ref_anchor_name, get_or_create_is_valid_fn,
    },
    refs::resolve_ref,
    stack_emit::{
        pop_dynamic_errors_n, pop_dynamic_is_valid_n, push_dynamic_errors, push_dynamic_is_valid,
    },
    CompileContext, CompiledExpr,
};
use crate::codegen::emit::ValueEmitter;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use serde_json::Value;

/// Break a self-referential `$dynamicRef`/`$recursiveRef` `is_valid` cycle: a re-entry on
/// the same (helper fn pointer, instance pointer) is valid, so the outer call's checks decide.
fn cycle_guarded_is_valid(node_address: &TokenStream, call: &TokenStream) -> TokenStream {
    let mark_expr = quote! { (target as usize, #node_address) };
    shared_cycle_guarded_is_valid(&mark_expr, call)
}

fn shared_cycle_guarded_is_valid(mark_expr: &TokenStream, call: &TokenStream) -> TokenStream {
    quote! {
        {
            let __mark = #mark_expr;
            let __reentered = __JSONSCHEMA_VALIDATE_MARK.with(|__marks| {
                let mut __marks = __marks.borrow_mut();
                if __marks.contains(&__mark) {
                    true
                } else {
                    __marks.push(__mark);
                    false
                }
            });
            if __reentered {
                true
            } else {
                let __valid = #call;
                __JSONSCHEMA_VALIDATE_MARK.with(|__marks| __marks.borrow_mut().pop());
                __valid
            }
        }
    }
}

/// Same cycle guard for the error-reporting dispatch; a re-entry reports nothing.
fn cycle_guarded_errors(node_address: &TokenStream, call: &TokenStream) -> TokenStream {
    let mark_expr = quote! { (target_errors as usize, #node_address) };
    shared_cycle_guarded_errors(&mark_expr, call)
}

fn shared_cycle_guarded_errors(mark_expr: &TokenStream, call: &TokenStream) -> TokenStream {
    quote! {
        {
            let __mark = #mark_expr;
            let __reentered = __JSONSCHEMA_VALIDATE_MARK.with(|__marks| {
                let mut __marks = __marks.borrow_mut();
                if __marks.contains(&__mark) {
                    true
                } else {
                    __marks.push(__mark);
                    false
                }
            });
            if !__reentered {
                let __stop = #call;
                __JSONSCHEMA_VALIDATE_MARK.with(|__marks| __marks.borrow_mut().pop());
                if __stop {
                    return true;
                }
            }
        }
    }
}

fn cycle_guarded_ref_errors(
    node: &TokenStream,
    node_address: &TokenStream,
    func_ident: &proc_macro2::Ident,
    errors_ident: &proc_macro2::Ident,
) -> TokenStream {
    let mark_expr = quote! {
        (
            (#func_ident as fn(#node) -> bool) as usize,
            #node_address,
        )
    };
    shared_cycle_guarded_errors(
        &mark_expr,
        &quote! { #errors_ident(instance, __path, __errors) },
    )
}

fn cycle_guarded_ref_is_valid(
    node: &TokenStream,
    node_address: &TokenStream,
    func_ident: &proc_macro2::Ident,
) -> TokenStream {
    let mark_expr = quote! {
        (
            (#func_ident as fn(#node) -> bool) as usize,
            #node_address,
        )
    };
    shared_cycle_guarded_is_valid(&mark_expr, &quote! { #func_ident(instance) })
}

/// A `$ref` check: the target's error function runs `is_valid` first, so its caller needs no
/// guard. `$dynamicRef` and `$recursiveRef` may land on the root error function, which does not.
fn ref_check(is_valid: TokenStream, errors: TokenStream) -> CompiledExpr {
    CompiledExpr::with_errors(is_valid, errors).with_skips_valid(true)
}

pub(crate) fn compile<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    value: &Value,
) -> CompiledExpr {
    let err_instance = E::err_instance(format_ident!("instance"));
    let node = E::node_param(None);
    let node_address = E::node_address(format_ident!("instance"));
    let Some(reference) = value.as_str() else {
        return super::super::errors::invalid_schema_expected_string_keyword_expression("$ref");
    };

    let resolved = match resolve_ref(ctx, reference) {
        Ok(resolved) => resolved,
        Err(err) => {
            let message = format!("Failed to resolve `$ref` `{reference}`: {err}");
            return super::super::errors::invalid_schema_expression(&message);
        }
    };

    let is_direct_self_ref = ctx
        .compiling_stack
        .last()
        .is_some_and(|location| location == &resolved.location)
        && ctx
            .helper_root_depths
            .last()
            .is_some_and(|depth| ctx.schema_depth == depth + 1);
    if is_direct_self_ref {
        return CompiledExpr::always_true();
    }

    let func_name =
        get_or_create_is_valid_fn(ctx, &resolved.location, resolved.schema, resolved.base_uri);
    let func_ident = format_ident!("{}", func_name);
    let call_scope_bindings = if ctx.uses_dynamic_ref {
        collect_dynamic_anchor_bindings(ctx, ctx.current_base_uri.clone())
    } else {
        Vec::new()
    };

    let schema_path = ctx.schema_path_for_keyword("$ref");
    if ctx.uses_dynamic_ref && !call_scope_bindings.is_empty() {
        let call_scope_pushes: Vec<_> = call_scope_bindings
            .iter()
            .map(|binding| {
                let is_valid_ident = format_ident!("{}", binding.is_valid_name);
                push_dynamic_is_valid(&binding.anchor, &is_valid_ident)
            })
            .collect();
        let call_scope_errors_pushes: Vec<_> = call_scope_bindings
            .iter()
            .map(|binding| {
                let errors_ident = format_ident!("{}_errors", binding.is_valid_name);
                push_dynamic_errors(&binding.anchor, &errors_ident)
            })
            .collect();
        let call_scope_count = call_scope_bindings.len();
        let dynamic_pop = pop_dynamic_is_valid_n(call_scope_count);
        let dynamic_errors_pop = pop_dynamic_errors_n(call_scope_count);
        let is_valid = quote! {
            {
                #(#call_scope_pushes)*
                let __result = #func_ident(instance);
                #dynamic_pop
                __result
            }
        };
        // Use the error helper when available for accurate error paths.
        if ctx.is_valid_fns.get_errors_body(&func_name).is_some() {
            let errors_ident = format_ident!("{}_errors", func_name);
            ref_check(
                is_valid,
                quote! {
                    {
                        #(#call_scope_pushes)*
                        #(#call_scope_errors_pushes)*
                        let __stop = #errors_ident(instance, __path, __errors);
                        #dynamic_errors_pop
                        #dynamic_pop
                        if __stop {
                            return true;
                        }
                    }
                },
            )
        } else {
            CompiledExpr::from_bool_expr(is_valid, &schema_path, &err_instance)
        }
    } else {
        let plain_cycle = ctx.is_valid_fns.is_compiling(&resolved.location)
            && !ctx.uses_recursive_ref
            && !ctx.uses_dynamic_ref;
        if plain_cycle && ctx.closes_same_instance_cycle(&resolved.location) {
            ctx.uses_ref_cycle = true;
            let errors_ident = format_ident!("{}_errors", func_name);
            ref_check(
                cycle_guarded_ref_is_valid(&node, &node_address, &func_ident),
                cycle_guarded_ref_errors(&node, &node_address, &func_ident, &errors_ident),
            )
        } else if plain_cycle || ctx.is_valid_fns.get_errors_body(&func_name).is_some() {
            let errors_ident = format_ident!("{}_errors", func_name);
            ref_check(
                quote! { #func_ident(instance) },
                call_errors(&quote! { #errors_ident(instance, __path, __errors) }),
            )
        } else {
            CompiledExpr::from_bool_expr(
                quote! { #func_ident(instance) },
                &schema_path,
                &err_instance,
            )
        }
    }
}

pub(crate) fn compile_dynamic<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    value: &Value,
) -> CompiledExpr {
    let node_address = E::node_address(format_ident!("instance"));
    let Some(reference) = value.as_str() else {
        return super::super::errors::invalid_schema_expected_string_keyword_expression(
            "$dynamicRef",
        );
    };

    let fallback = compile(ctx, value);
    let Ok(resolved) = resolve_ref(ctx, reference) else {
        return fallback;
    };

    let Some(anchor_name) = dynamic_ref_anchor_name(reference, resolved.schema) else {
        return fallback;
    };

    ctx.uses_dynamic_ref = true;
    let fallback_is_valid = fallback.is_valid_token_stream();

    let dynamic_lookup = quote! {
        __JSONSCHEMA_DYNAMIC_STACK.with(|stack| {
            stack
                .borrow()
                .iter()
                .find(|(dynamic_anchor, _)| *dynamic_anchor == #anchor_name)
                .map(|(_, is_valid)| *is_valid)
        })
    };

    let guarded_is_valid = cycle_guarded_is_valid(&node_address, &quote! { target(instance) });
    let is_valid = quote! {
        {
            let __dynamic_target = #dynamic_lookup;
            if let Some(target) = __dynamic_target {
                #guarded_is_valid
            } else {
                #fallback_is_valid
            }
        }
    };

    let fallback_errors = match &fallback.errors {
        ErrorsBlock::Expr(expr) => expr.clone(),
        ErrorsBlock::AlwaysValid => quote! {},
    };
    let dynamic_lookup_errors = quote! {
        __JSONSCHEMA_DYNAMIC_ERRORS_STACK.with(|stack| {
            stack
                .borrow()
                .iter()
                .find(|(dynamic_anchor, _)| *dynamic_anchor == #anchor_name)
                .map(|(_, errors)| *errors)
        })
    };
    let guarded_errors = cycle_guarded_errors(
        &node_address,
        &quote! { target_errors(instance, __path, __errors) },
    );
    CompiledExpr::with_errors(
        is_valid,
        quote! {
            {
                let __dynamic_target_errors = #dynamic_lookup_errors;
                if let Some(target_errors) = __dynamic_target_errors {
                    #guarded_errors
                } else {
                    #fallback_errors
                }
            }
        },
    )
}

pub(crate) fn compile_recursive<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    value: &Value,
) -> CompiledExpr {
    let node_address = E::node_address(format_ident!("instance"));
    let target_has_recursive_anchor = value
        .as_str()
        .and_then(|reference| resolve_ref(ctx, reference).ok())
        .and_then(|resolved| {
            resolved
                .schema
                .as_object()
                .and_then(|obj| obj.get("$recursiveAnchor"))
                .and_then(Value::as_bool)
        })
        == Some(true);

    if !target_has_recursive_anchor {
        return compile(ctx, value);
    }

    ctx.uses_recursive_ref = true;
    let fallback = compile(ctx, value);
    let fallback_is_valid = fallback.is_valid_token_stream();

    let recursive_lookup = quote! {
        __JSONSCHEMA_RECURSIVE_STACK.with(|stack| {
            let stack = stack.borrow();
            let mut selected = None;
            for (is_valid, is_anchor) in stack.iter() {
                if *is_anchor {
                    selected = Some(*is_valid);
                    break;
                }
            }
            selected
        })
    };

    let guarded_is_valid = cycle_guarded_is_valid(&node_address, &quote! { target(instance) });
    let is_valid = quote! {
        {
            let __recursive_target = #recursive_lookup;
            if let Some(target) = __recursive_target {
                #guarded_is_valid
            } else {
                #fallback_is_valid
            }
        }
    };

    let fallback_errors = match &fallback.errors {
        ErrorsBlock::Expr(expr) => expr.clone(),
        ErrorsBlock::AlwaysValid => quote! {},
    };
    let recursive_lookup_errors = quote! {
        __JSONSCHEMA_RECURSIVE_ERRORS_STACK.with(|stack| {
            let stack = stack.borrow();
            let mut selected = None;
            for (errors, is_anchor) in stack.iter() {
                if *is_anchor {
                    selected = Some(*errors);
                    break;
                }
            }
            selected
        })
    };
    let guarded_errors = cycle_guarded_errors(
        &node_address,
        &quote! { target_errors(instance, __path, __errors) },
    );
    CompiledExpr::with_errors(
        is_valid,
        quote! {
            {
                let __recursive_target_errors = #recursive_lookup_errors;
                if let Some(target_errors) = __recursive_target_errors {
                    #guarded_errors
                } else {
                    #fallback_errors
                }
            }
        },
    )
}
