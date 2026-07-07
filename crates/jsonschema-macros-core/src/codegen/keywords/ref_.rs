use super::super::{
    expr::ValidateBlock,
    helpers::{
        collect_dynamic_anchor_bindings, dynamic_ref_anchor_name, get_or_create_is_valid_fn,
    },
    refs::resolve_ref,
    stack_emit::{
        pop_dynamic_is_valid_n, pop_dynamic_validate_n, push_dynamic_is_valid,
        push_dynamic_validate,
    },
    CompileContext, CompiledExpr,
};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use serde_json::Value;

/// Break a self-referential `$dynamicRef`/`$recursiveRef` `is_valid` cycle: a re-entry on
/// the same (helper fn pointer, instance pointer) is valid, so the outer call's checks decide.
fn cycle_guarded_is_valid(call: &TokenStream) -> TokenStream {
    quote! {
        {
            let __mark = (target as usize, std::ptr::from_ref(instance) as usize);
            if __JSONSCHEMA_VALIDATE_MARK.with(|__marks| __marks.borrow().contains(&__mark)) {
                true
            } else {
                __JSONSCHEMA_VALIDATE_MARK.with(|__marks| __marks.borrow_mut().push(__mark));
                let __valid = #call;
                __JSONSCHEMA_VALIDATE_MARK.with(|__marks| __marks.borrow_mut().pop());
                __valid
            }
        }
    }
}

/// Same cycle guard for the error-collecting (`validate`) dispatch; a re-entry reports no error.
fn cycle_guarded_validate(call: &TokenStream) -> TokenStream {
    quote! {
        {
            let __mark = (target_validate as usize, std::ptr::from_ref(instance) as usize);
            if !__JSONSCHEMA_VALIDATE_MARK.with(|__marks| __marks.borrow().contains(&__mark)) {
                __JSONSCHEMA_VALIDATE_MARK.with(|__marks| __marks.borrow_mut().push(__mark));
                let __r = #call;
                __JSONSCHEMA_VALIDATE_MARK.with(|__marks| __marks.borrow_mut().pop());
                if let Some(__err) = __r {
                    return Some(__err);
                }
            }
        }
    }
}

fn cycle_guarded_ref_is_valid(func_ident: &proc_macro2::Ident) -> TokenStream {
    quote! {
        {
            let __mark = (
                (#func_ident as fn(&serde_json::Value) -> bool) as usize,
                std::ptr::from_ref(instance) as usize,
            );
            if __JSONSCHEMA_VALIDATE_MARK.with(|__marks| __marks.borrow().contains(&__mark)) {
                true
            } else {
                __JSONSCHEMA_VALIDATE_MARK.with(|__marks| __marks.borrow_mut().push(__mark));
                let __valid = #func_ident(instance);
                __JSONSCHEMA_VALIDATE_MARK.with(|__marks| __marks.borrow_mut().pop());
                __valid
            }
        }
    }
}

fn cycle_guarded_ref_validate(
    func_ident: &proc_macro2::Ident,
    validate_ident: &proc_macro2::Ident,
) -> TokenStream {
    quote! {
        {
            let __mark = (
                (#func_ident as fn(&serde_json::Value) -> bool) as usize,
                std::ptr::from_ref(instance) as usize,
            );
            if !__JSONSCHEMA_VALIDATE_MARK.with(|__marks| __marks.borrow().contains(&__mark)) {
                __JSONSCHEMA_VALIDATE_MARK.with(|__marks| __marks.borrow_mut().push(__mark));
                let __r = #validate_ident(instance, __path);
                __JSONSCHEMA_VALIDATE_MARK.with(|__marks| __marks.borrow_mut().pop());
                if let Some(__err) = __r {
                    return Some(__err);
                }
            }
        }
    }
}

pub(crate) fn compile(ctx: &mut CompileContext<'_>, value: &Value) -> CompiledExpr {
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
        get_or_create_is_valid_fn(ctx, &resolved.location, &resolved.schema, resolved.base_uri);
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
        let call_scope_validate_pushes: Vec<_> = call_scope_bindings
            .iter()
            .map(|binding| {
                let validate_ident = format_ident!("{}_validate", binding.is_valid_name);
                push_dynamic_validate(&binding.anchor, &validate_ident)
            })
            .collect();
        let call_scope_count = call_scope_bindings.len();
        let dynamic_pop = pop_dynamic_is_valid_n(call_scope_count);
        let dynamic_validate_pop = pop_dynamic_validate_n(call_scope_count);
        let is_valid = quote! {
            {
                #(#call_scope_pushes)*
                let __result = #func_ident(instance);
                #dynamic_pop
                __result
            }
        };
        // Use the validate helper when available for accurate error paths.
        let can_use_validate = ctx.is_valid_fns.get_validate_body(&func_name).is_some();
        if can_use_validate {
            let validate_ident = format_ident!("{}_validate", func_name);
            CompiledExpr::with_validate_blocks(
                is_valid,
                quote! {
                    {
                        #(#call_scope_pushes)*
                        #(#call_scope_validate_pushes)*
                        let __r = #validate_ident(instance, __path);
                        #dynamic_validate_pop
                        #dynamic_pop
                        if let Some(__e) = __r { return Some(__e); }
                    }
                },
            )
        } else {
            CompiledExpr::from_bool_expr(is_valid, &schema_path)
        }
    } else {
        let plain_cycle = ctx.is_valid_fns.is_compiling(&resolved.location)
            && !ctx.uses_recursive_ref
            && !ctx.uses_dynamic_ref;
        if plain_cycle && ctx.closes_same_instance_cycle(&resolved.location) {
            ctx.uses_ref_cycle = true;
            let validate_ident = format_ident!("{}_validate", func_name);
            CompiledExpr::with_validate_blocks(
                cycle_guarded_ref_is_valid(&func_ident),
                cycle_guarded_ref_validate(&func_ident, &validate_ident),
            )
        } else if plain_cycle || ctx.is_valid_fns.get_validate_body(&func_name).is_some() {
            let validate_ident = format_ident!("{}_validate", func_name);
            CompiledExpr::with_validate_blocks(
                quote! { #func_ident(instance) },
                quote! {
                    if let Some(__err) = #validate_ident(instance, __path) {
                        return Some(__err);
                    }
                },
            )
        } else {
            CompiledExpr::from_bool_expr(quote! { #func_ident(instance) }, &schema_path)
        }
    }
}

pub(crate) fn compile_dynamic(ctx: &mut CompileContext<'_>, value: &Value) -> CompiledExpr {
    let Some(reference) = value.as_str() else {
        return super::super::errors::invalid_schema_expected_string_keyword_expression(
            "$dynamicRef",
        );
    };

    let fallback = compile(ctx, value);
    let Ok(resolved) = resolve_ref(ctx, reference) else {
        return fallback;
    };

    let Some(anchor_name) = dynamic_ref_anchor_name(reference, &resolved.schema) else {
        return fallback;
    };

    ctx.uses_dynamic_ref = true;
    let fallback_is_valid = fallback.is_valid_token_stream();

    let dynamic_lookup = quote! {
        __JSONSCHEMA_DYNAMIC_STACK.with(|stack| {
            let stack = stack.borrow();
            let mut selected = None;
            for (dynamic_anchor, is_valid) in stack.iter().rev() {
                if *dynamic_anchor == #anchor_name {
                    selected = Some(*is_valid);
                }
            }
            selected
        })
    };

    let guarded_is_valid = cycle_guarded_is_valid(&quote! { target(instance) });
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

    {
        let fallback_validate = match &fallback.validate {
            ValidateBlock::Expr(expr) => expr.clone(),
            ValidateBlock::AlwaysValid => quote! {},
        };
        // Look up the validate stack for accurate error paths.
        let dynamic_lookup_validate = quote! {
            __JSONSCHEMA_DYNAMIC_VALIDATE_STACK.with(|stack| {
                let stack = stack.borrow();
                let mut selected = None;
                for (dynamic_anchor, validate) in stack.iter().rev() {
                    if *dynamic_anchor == #anchor_name {
                        selected = Some(*validate);
                    }
                }
                selected
            })
        };
        let guarded_validate =
            cycle_guarded_validate(&quote! { target_validate(instance, __path) });
        CompiledExpr::with_validate_blocks(
            is_valid,
            quote! {
                {
                    let __dynamic_target_validate = #dynamic_lookup_validate;
                    if let Some(target_validate) = __dynamic_target_validate {
                        #guarded_validate
                    } else {
                        #fallback_validate
                    }
                }
            },
        )
    }
}

pub(crate) fn compile_recursive(ctx: &mut CompileContext<'_>, value: &Value) -> CompiledExpr {
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
            for (is_valid, is_anchor) in stack.iter().rev() {
                if *is_anchor {
                    selected = Some(*is_valid);
                } else {
                    break;
                }
            }
            selected
        })
    };

    let guarded_is_valid = cycle_guarded_is_valid(&quote! { target(instance) });
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

    {
        let fallback_validate = match &fallback.validate {
            ValidateBlock::Expr(expr) => expr.clone(),
            ValidateBlock::AlwaysValid => quote! {},
        };
        // Use the validate stack for accurate error paths in validate() context.
        let recursive_lookup_validate = quote! {
            __JSONSCHEMA_RECURSIVE_VALIDATE_STACK.with(|stack| {
                let stack = stack.borrow();
                let mut selected = None;
                for (validate, is_anchor) in stack.iter().rev() {
                    if *is_anchor {
                        selected = Some(*validate);
                    } else {
                        break;
                    }
                }
                selected
            })
        };
        let guarded_validate =
            cycle_guarded_validate(&quote! { target_validate(instance, __path) });
        CompiledExpr::with_validate_blocks(
            is_valid,
            quote! {
                {
                    let __recursive_target_validate = #recursive_lookup_validate;
                    if let Some(target_validate) = __recursive_target_validate {
                        #guarded_validate
                    } else {
                        #fallback_validate
                    }
                }
            },
        )
    }
}
