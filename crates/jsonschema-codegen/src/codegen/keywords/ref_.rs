use super::super::{
    expr::ValidateBlock,
    helpers::{
        collect_dynamic_anchor_bindings, dynamic_ref_anchor_name, get_or_create_is_valid_fn,
    },
    refs::resolve_ref,
    stack_emit::{
        pop_dynamic_validate_n, pop_dynamic_validate_v_n, push_dynamic_validate,
        push_dynamic_validate_v,
    },
    CompileContext, CompiledExpr,
};
use quote::{format_ident, quote};
use serde_json::Value;

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
            .map(|b| {
                let validate_ident = format_ident!("{}", b.validate_name);
                push_dynamic_validate(&b.anchor, &validate_ident)
            })
            .collect();
        let call_scope_v_pushes: Vec<_> = call_scope_bindings
            .iter()
            .map(|b| {
                let v_ident = format_ident!("{}_v", b.validate_name);
                push_dynamic_validate_v(&b.anchor, &v_ident)
            })
            .collect();
        let call_scope_count = call_scope_bindings.len();
        let dynamic_pop = pop_dynamic_validate_n(call_scope_count);
        let dynamic_v_pop = pop_dynamic_validate_v_n(call_scope_count);
        let is_valid_ts = quote! {
            {
                #(#call_scope_pushes)*
                let __result = #func_ident(instance);
                #dynamic_pop
                __result
            }
        };
        // Use _v helper when available for accurate error paths.
        let can_use_v = ctx.is_valid_helpers.get_validate_body(&func_name).is_some();
        if can_use_v {
            let v_ident = format_ident!("{}_v", func_name);
            CompiledExpr::with_validate_blocks(
                is_valid_ts,
                quote! {
                    {
                        #(#call_scope_pushes)*
                        #(#call_scope_v_pushes)*
                        let __r = #v_ident(instance, __path.clone());
                        #dynamic_v_pop
                        #dynamic_pop
                        if let Some(__e) = __r { return Some(__e); }
                    }
                },
            )
        } else {
            CompiledExpr::from_bool_expr(is_valid_ts, &schema_path)
        }
    } else {
        // Use the _v helper when available to report errors at the referenced
        // schema's location.  Also handles cycle-breaking refs: when the target
        // helper is still being compiled (is_compiling) in a non-recursive,
        // non-dynamic context, its _v function will be generated once compilation
        // finishes, so it's safe to emit calls to it now.
        let can_use_helpers = ctx.is_valid_helpers.get_validate_body(&func_name).is_some()
            || (ctx.is_valid_helpers.is_compiling(&resolved.location)
                && !ctx.uses_recursive_ref
                && !ctx.uses_dynamic_ref);
        if can_use_helpers {
            let v_ident = format_ident!("{}_v", func_name);
            CompiledExpr::with_validate_blocks(
                quote! { #func_ident(instance) },
                quote! {
                    if let Some(__err) = #v_ident(instance, __path.clone()) {
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
    let fallback_ts = fallback.is_valid_ts();

    let dynamic_lookup = quote! {
        __JSONSCHEMA_DYNAMIC_STACK.with(|stack| {
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

    let is_valid_ts = quote! {
        {
            let __dynamic_target = #dynamic_lookup;
            if let Some(target) = __dynamic_target {
                target(instance)
            } else {
                #fallback_ts
            }
        }
    };

    {
        let fallback_validate = match &fallback.validate {
            ValidateBlock::Expr(ts) => ts.clone(),
            ValidateBlock::AlwaysValid => quote! {},
        };
        // Look up the _v stack for accurate error paths.
        let dynamic_lookup_v = quote! {
            __JSONSCHEMA_DYNAMIC_VALIDATE_STACK.with(|stack| {
                let stack = stack.borrow();
                let mut selected = None;
                for (dynamic_anchor, validate_v) in stack.iter().rev() {
                    if *dynamic_anchor == #anchor_name {
                        selected = Some(*validate_v);
                    }
                }
                selected
            })
        };
        CompiledExpr::with_validate_blocks(
            is_valid_ts,
            quote! {
                {
                    let __dynamic_target_v = #dynamic_lookup_v;
                    if let Some(target_v) = __dynamic_target_v {
                        if let Some(__err) = target_v(instance, __path.clone()) {
                            return Some(__err);
                        }
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
    let fallback_ts = fallback.is_valid_ts();

    let recursive_lookup = quote! {
        __JSONSCHEMA_RECURSIVE_STACK.with(|stack| {
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

    let is_valid_ts = quote! {
        {
            let __recursive_target = #recursive_lookup;
            if let Some(target) = __recursive_target {
                target(instance)
            } else {
                #fallback_ts
            }
        }
    };

    {
        let fallback_validate = match &fallback.validate {
            ValidateBlock::Expr(ts) => ts.clone(),
            ValidateBlock::AlwaysValid => quote! {},
        };
        // Use the _v stack for accurate error paths in validate() context.
        let recursive_lookup_v = quote! {
            __JSONSCHEMA_RECURSIVE_VALIDATE_STACK.with(|stack| {
                let stack = stack.borrow();
                let mut selected = None;
                for (validate_v, is_anchor) in stack.iter().rev() {
                    if *is_anchor {
                        selected = Some(*validate_v);
                    } else {
                        break;
                    }
                }
                selected
            })
        };
        CompiledExpr::with_validate_blocks(
            is_valid_ts,
            quote! {
                {
                    let __recursive_target_v = #recursive_lookup_v;
                    if let Some(target_v) = __recursive_target_v {
                        if let Some(__err) = target_v(instance, __path.clone()) {
                            return Some(__err);
                        }
                    } else {
                        #fallback_validate
                    }
                }
            },
        )
    }
}
