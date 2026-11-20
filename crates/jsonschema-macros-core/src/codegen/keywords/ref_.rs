use super::super::{
    expr::{call_errors, ErrorsBlock, RefErrors},
    helpers::{
        collect_dynamic_anchor_bindings, dynamic_ref_anchor_name, get_or_create_is_valid_fn,
        DynamicAnchorBinding,
    },
    refs::resolve_ref,
    stack_emit::{
        pop_dynamic_errors_n, pop_dynamic_is_valid_n, push_dynamic_errors, push_dynamic_is_valid,
    },
    CompileContext, CompiledExpr,
};
use crate::{codegen::emit::ValueEmitter, context::RefResultsKey};
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

/// The cycle guard mark of a `$ref` target on the instance, by its `is_valid` function in both
/// dispatches. The dynamic scope stays out of it: a re-entry stops in any scope, as the runtime's
/// guard on the target does.
fn ref_mark(
    node: &TokenStream,
    node_address: &TokenStream,
    func_ident: &proc_macro2::Ident,
) -> TokenStream {
    quote! {
        (
            (#func_ident as fn(#node) -> bool) as usize,
            #node_address,
        )
    }
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
    compile_at(ctx, value, "$ref")
}

/// The call validating `instance` against the target `func_ident` names, at `location`.
///
/// Keywords reaching one instance through a cycle at every level would otherwise validate it once
/// per path to it, so a cycle's results are kept for the run. Under a dynamic scope a result
/// depends on what the scope binds, so each is kept for the scope it was computed in.
fn ref_call<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    func_ident: &proc_macro2::Ident,
    cycle: bool,
    location: &str,
) -> TokenStream {
    if !cycle || !ctx.revisited_ref_targets.contains(location) {
        return quote! { #func_ident(instance) };
    }
    let json = E::json_representation();
    match &mut ctx.ref_results_key {
        RefResultsKey::Instance => quote! { __ref_memo::is_valid::<#json>(#func_ident, instance) },
        RefResultsKey::InstanceAndScope { calls } => {
            *calls += 1;
            quote! {
                __ref_memo::is_valid_in_scope::<#json>(#func_ident, instance, __dynamic_scope)
            }
        }
    }
}

/// The code putting the `$dynamicAnchor` bindings of the resource a reference leaves on the
/// dynamic-scope stacks while its target runs, and taking them off after.
struct CallScope {
    pushes: Vec<TokenStream>,
    errors_pushes: Vec<TokenStream>,
    pop: TokenStream,
    errors_pop: TokenStream,
}

impl CallScope {
    fn new(bindings: &[DynamicAnchorBinding]) -> Self {
        CallScope {
            pushes: bindings
                .iter()
                .map(|binding| {
                    let is_valid_ident = format_ident!("{}", binding.is_valid_name);
                    push_dynamic_is_valid(&binding.anchor, &is_valid_ident)
                })
                .collect(),
            errors_pushes: bindings
                .iter()
                .map(|binding| {
                    let errors_ident = format_ident!("{}_errors", binding.is_valid_name);
                    push_dynamic_errors(&binding.anchor, &errors_ident)
                })
                .collect(),
            pop: pop_dynamic_is_valid_n(bindings.len()),
            errors_pop: pop_dynamic_errors_n(bindings.len()),
        }
    }
}

/// A static reference, through the `keyword` that holds it.
fn compile_at<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    value: &Value,
    keyword: &'static str,
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

    let func_name = get_or_create_is_valid_fn(
        ctx,
        &resolved.location,
        resolved.resource_start,
        resolved.schema,
        resolved.base_uri,
        resolved.dialect,
    );
    let func_ident = format_ident!("{}", func_name);
    let call_scope_bindings = if ctx.uses_dynamic_ref {
        collect_dynamic_anchor_bindings(ctx, ctx.current_base_uri.clone())
    } else {
        Vec::new()
    };

    let schema_path = ctx.schema_path_for_keyword(keyword);
    let site = quote! { #schema_path };
    // A target still compiling closes a cycle, and gets its error function once it compiles.
    let cycle = ctx.is_valid_fns.is_compiling(&resolved.location);
    let same_instance_cycle = cycle && ctx.closes_same_instance_cycle(&resolved.location);
    if same_instance_cycle {
        ctx.uses_ref_cycle = true;
    }
    let call = ref_call(ctx, &func_ident, cycle, &resolved.location);
    if ctx.uses_dynamic_ref && !call_scope_bindings.is_empty() {
        let CallScope {
            pushes: call_scope_pushes,
            errors_pushes: call_scope_errors_pushes,
            pop: dynamic_pop,
            errors_pop: dynamic_errors_pop,
        } = CallScope::new(&call_scope_bindings);
        let is_valid = quote! {
            {
                #(#call_scope_pushes)*
                let __result = #call;
                #dynamic_pop
                __result
            }
        };
        if cycle || ctx.is_valid_fns.get_errors_body(&func_name).is_some() {
            let errors_ident = format_ident!("{}_errors", func_name);
            let scoped_errors = quote! {
                #(#call_scope_pushes)*
                #(#call_scope_errors_pushes)*
                let __stop = #errors_ident(instance, __path, __errors, #site);
                #dynamic_errors_pop
                #dynamic_pop
            };
            if same_instance_cycle {
                let mark = ref_mark(&node, &node_address, &func_ident);
                ref_check(
                    shared_cycle_guarded_is_valid(&mark, &is_valid),
                    shared_cycle_guarded_errors(&mark, &quote! { { #scoped_errors __stop } }),
                )
            } else {
                ref_check(
                    is_valid,
                    quote! {
                        {
                            #scoped_errors
                            if __stop {
                                return true;
                            }
                        }
                    },
                )
            }
        } else {
            CompiledExpr::from_bool_expr(is_valid, &schema_path, &err_instance)
        }
    } else {
        let errors_ident = format_ident!("{}_errors", func_name);
        let is_valid = call;
        if same_instance_cycle {
            let mark = ref_mark(&node, &node_address, &func_ident);
            // The guard comes before the memo, as in the runtime: a target re-entered on an
            // instance is valid there whatever its memo holds.
            ref_check(
                shared_cycle_guarded_is_valid(&mark, &is_valid),
                shared_cycle_guarded_errors(
                    &mark,
                    &quote! { #errors_ident(instance, __path, __errors, #site) },
                ),
            )
        } else if cycle || ctx.is_valid_fns.get_errors_body(&func_name).is_some() {
            ref_check(
                is_valid,
                call_errors(&quote! { #errors_ident(instance, __path, __errors, #site) }),
            )
            .with_ref_errors(RefErrors::Call(func_name, keyword))
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
    let Some(reference) = value.as_str() else {
        return super::super::errors::invalid_schema_expected_string_keyword_expression(
            "$dynamicRef",
        );
    };

    let fallback = compile_at(ctx, value, "$dynamicRef");
    let Ok(resolved) = resolve_ref(ctx, reference) else {
        return fallback;
    };

    let Some(anchor_name) = dynamic_ref_anchor_name(reference, resolved.schema) else {
        return fallback;
    };

    ctx.uses_dynamic_ref = true;
    let caller = collect_dynamic_anchor_bindings(ctx, ctx.current_base_uri.clone());
    through_dynamic_scope(
        ctx,
        &resolved.location,
        &fallback,
        &BoundEntry {
            is_valid_stack: format_ident!("__JSONSCHEMA_DYNAMIC_STACK"),
            errors_stack: format_ident!("__JSONSCHEMA_DYNAMIC_ERRORS_STACK"),
            pattern: quote! { (dynamic_anchor, target) },
            selects: quote! { *dynamic_anchor == #anchor_name },
        },
        &caller,
        "$dynamicRef",
    )
}

pub(crate) fn compile_recursive<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    value: &Value,
) -> CompiledExpr {
    let anchored_target = value
        .as_str()
        .and_then(|reference| resolve_ref(ctx, reference).ok())
        .filter(|resolved| {
            resolved
                .schema
                .as_object()
                .and_then(|obj| obj.get("$recursiveAnchor"))
                .and_then(Value::as_bool)
                == Some(true)
        });

    let Some(resolved) = anchored_target else {
        return compile_at(ctx, value, "$recursiveRef");
    };

    ctx.uses_recursive_ref = true;
    let fallback = compile_at(ctx, value, "$recursiveRef");
    through_dynamic_scope(
        ctx,
        &resolved.location,
        &fallback,
        &BoundEntry {
            is_valid_stack: format_ident!("__JSONSCHEMA_RECURSIVE_STACK"),
            errors_stack: format_ident!("__JSONSCHEMA_RECURSIVE_ERRORS_STACK"),
            pattern: quote! { (target, is_anchor) },
            selects: quote! { *is_anchor },
        },
        &[],
        "$recursiveRef",
    )
}

/// The entry of the dynamic-scope stacks a reference resolves to: the first one `pattern`
/// destructures into `target` and its key, and `selects` picks.
struct BoundEntry {
    is_valid_stack: proc_macro2::Ident,
    errors_stack: proc_macro2::Ident,
    pattern: TokenStream,
    selects: TokenStream,
}

impl BoundEntry {
    /// The code the scope binds on `stack`, or `None` where it binds none or `static_target`.
    fn lookup(
        &self,
        stack: &proc_macro2::Ident,
        static_target: &proc_macro2::Ident,
    ) -> TokenStream {
        let pattern = &self.pattern;
        let selects = &self.selects;
        quote! {
            #stack.with(|stack| {
                let stack = stack.borrow();
                let mut selected = None;
                for #pattern in stack.iter() {
                    if #selects {
                        if *target as usize != #static_target as usize {
                            selected = Some(*target);
                        }
                        break;
                    }
                }
                selected
            })
        }
    }
}

/// A reference resolving through the dynamic scope to the code `bound` finds there.
///
/// Bound to its static target, at `location`, it is the static reference `fallback`: the runtime
/// compiles both under one name, so they share a cycle guard and the first entry stays unguarded.
/// Bound elsewhere, it enters that code under a guard of its own, with the `caller` bindings of the
/// resource it leaves on the stacks.
fn through_dynamic_scope<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    location: &str,
    fallback: &CompiledExpr,
    bound: &BoundEntry,
    caller: &[DynamicAnchorBinding],
    keyword: &'static str,
) -> CompiledExpr {
    let node_address = E::node_address(format_ident!("instance"));
    let static_target = format_ident!(
        "{}",
        ctx.is_valid_fns
            .get_name(location)
            .expect("the static target compiles before its dynamic references")
    );
    ctx.looked_up_stacks
        .insert(bound.is_valid_stack.to_string());
    let lookup = bound.lookup(&bound.is_valid_stack, &static_target);
    let fallback_is_valid = fallback.is_valid_token_stream();
    let site = ctx.schema_path_for_keyword(keyword);
    let (landed_is_valid, landed_errors) = if caller.is_empty() {
        (
            quote! { target(instance) },
            quote! { target_errors(instance, __path, __errors, #site) },
        )
    } else {
        let CallScope {
            pushes,
            errors_pushes,
            pop,
            errors_pop,
        } = CallScope::new(caller);
        (
            quote! {{
                #(#pushes)*
                let __valid = target(instance);
                #pop
                __valid
            }},
            quote! {{
                #(#pushes)*
                #(#errors_pushes)*
                let __stop = target_errors(instance, __path, __errors, #site);
                #errors_pop
                #pop
                __stop
            }},
        )
    };
    let guarded_is_valid = cycle_guarded_is_valid(&node_address, &landed_is_valid);
    let is_valid = quote! {
        {
            let __dynamic_target = #lookup;
            if let Some(target) = __dynamic_target {
                #guarded_is_valid
            } else {
                #fallback_is_valid
            }
        }
    };

    let lookup_errors = bound.lookup(
        &bound.errors_stack,
        &format_ident!("{}_errors", static_target),
    );
    let fallback_errors = match &fallback.errors {
        ErrorsBlock::Expr(expr) => expr.clone(),
        ErrorsBlock::AlwaysValid => quote! {},
    };
    let guarded_errors = cycle_guarded_errors(&node_address, &landed_errors);
    CompiledExpr::with_errors(
        is_valid,
        quote! {
            {
                let __dynamic_target_errors = #lookup_errors;
                if let Some(target_errors) = __dynamic_target_errors {
                    #guarded_errors
                } else {
                    #fallback_errors
                }
            }
        },
    )
}
