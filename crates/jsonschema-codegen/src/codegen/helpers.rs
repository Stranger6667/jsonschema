use quote::{format_ident, quote};
use referencing::Uri;
use serde_json::Value;
use std::sync::Arc;

use crate::context::CompileContext;

use super::{
    compile_schema,
    draft::{supports_dynamic_ref_keyword, supports_recursive_ref_keyword},
    keywords::unevaluated::{compile_index_evaluated_expr, compile_key_evaluated_expr},
    refs::resolve_ref,
    stack_emit::{
        pop_dynamic_item_eval_n, pop_dynamic_iter_errors_e_n, pop_dynamic_key_eval_n,
        pop_dynamic_validate_n, pop_dynamic_validate_v_n, pop_recursive_item_eval,
        pop_recursive_iter_errors_e, pop_recursive_key_eval, pop_recursive_validate,
        pop_recursive_validate_v, push_dynamic_item_eval, push_dynamic_iter_errors_e,
        push_dynamic_key_eval, push_dynamic_validate, push_dynamic_validate_v,
        push_recursive_item_eval, push_recursive_iter_errors_e, push_recursive_key_eval,
        push_recursive_validate, push_recursive_validate_v,
    },
};

#[derive(Clone)]
pub(crate) struct DynamicAnchorBinding {
    pub(crate) anchor: String,
    pub(crate) validate_name: String,
    pub(crate) key_eval_name: String,
    pub(crate) item_eval_name: String,
}

pub(crate) fn dynamic_ref_anchor_name(reference: &str, resolved_schema: &Value) -> Option<String> {
    let (_, fragment) = reference.rsplit_once('#')?;
    if fragment.is_empty() || fragment.starts_with('/') {
        return None;
    }
    let dynamic_anchor = resolved_schema
        .as_object()
        .and_then(|obj| obj.get("$dynamicAnchor"))
        .and_then(Value::as_str)?;
    (dynamic_anchor == fragment).then(|| dynamic_anchor.to_string())
}

fn collect_dynamic_anchor_names(schema: &Value, names: &mut std::collections::HashSet<String>) {
    match schema {
        Value::Object(obj) => {
            if let Some(anchor) = obj.get("$dynamicAnchor").and_then(Value::as_str) {
                names.insert(anchor.to_string());
            }
            for value in obj.values() {
                collect_dynamic_anchor_names(value, names);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_dynamic_anchor_names(item, names);
            }
        }
        _ => {}
    }
}

pub(crate) fn collect_dynamic_anchor_bindings(
    ctx: &mut CompileContext<'_>,
    schema_base_uri: Arc<Uri<String>>,
) -> Vec<DynamicAnchorBinding> {
    let cache_key = schema_base_uri.to_string();
    if let Some(cached) = ctx.dynamic_anchor_bindings_cache.get(&cache_key) {
        return cached.clone();
    }
    if !supports_dynamic_ref_keyword(ctx.draft) {
        return Vec::new();
    }

    let Some(bindings) = ctx.with_dynamic_anchor_bindings_scope(cache_key.clone(), |ctx| {
        let resolver = ctx.config.registry.resolver((*schema_base_uri).clone());
        let Ok(resource) = resolver.lookup("") else {
            return None;
        };
        let resource_schema = resource.contents();

        let mut names = std::collections::HashSet::new();
        collect_dynamic_anchor_names(resource_schema, &mut names);
        let mut names: Vec<_> = names.into_iter().collect();
        names.sort();

        let bindings = ctx.with_base_uri_scope(schema_base_uri, |ctx| {
            let mut bindings = Vec::new();
            for anchor in names {
                let reference = format!("#{anchor}");
                let Ok(resolved) = resolve_ref(ctx, &reference) else {
                    continue;
                };
                if dynamic_ref_anchor_name(&reference, &resolved.schema).is_none() {
                    continue;
                }
                let validate_name = get_or_create_is_valid_fn(
                    ctx,
                    &resolved.location,
                    &resolved.schema,
                    resolved.base_uri.clone(),
                );
                let key_eval_name = get_or_create_key_eval_fn(
                    ctx,
                    &resolved.location,
                    &resolved.schema,
                    resolved.base_uri.clone(),
                );
                let item_eval_name = get_or_create_item_eval_fn(
                    ctx,
                    &resolved.location,
                    &resolved.schema,
                    resolved.base_uri,
                );
                bindings.push(DynamicAnchorBinding {
                    anchor,
                    validate_name,
                    key_eval_name,
                    item_eval_name,
                });
            }
            bindings
        });

        Some(bindings)
    }) else {
        return Vec::new();
    };
    let Some(bindings) = bindings else {
        return Vec::new();
    };
    ctx.dynamic_anchor_bindings_cache
        .insert(cache_key, bindings.clone());
    bindings
}

/// Get or create a helper function that determines whether a property key is
/// already evaluated for the referenced schema.
pub(crate) fn get_or_create_key_eval_fn(
    ctx: &mut CompileContext<'_>,
    location: &str,
    schema: &Value,
    schema_base_uri: Arc<Uri<String>>,
) -> String {
    if let Some(name) = ctx.key_eval_helpers.get_name(location) {
        return name.clone();
    }

    let func_name = ctx.key_eval_helpers.alloc_name(location);

    if let Value::Object(schema_obj) = schema {
        let body = ctx.with_key_eval_scope(location, |ctx| {
            let schema_value = Value::Object(schema_obj.clone());
            ctx.with_schema_env(&schema_value, schema_base_uri, |ctx| {
                let compiled = compile_key_evaluated_expr(ctx, schema_obj);
                let is_recursive_anchor = supports_recursive_ref_keyword(ctx.draft)
                    && schema_obj.get("$recursiveAnchor").and_then(Value::as_bool) == Some(true);
                let dynamic_bindings = if ctx.uses_dynamic_ref {
                    collect_dynamic_anchor_bindings(ctx, ctx.current_base_uri.clone())
                } else {
                    Vec::new()
                };
                if ctx.uses_recursive_ref || ctx.uses_dynamic_ref {
                    let func_ident = format_ident!("{}", func_name);
                    let recursive_push = if ctx.uses_recursive_ref {
                        push_recursive_key_eval(&func_ident, is_recursive_anchor)
                    } else {
                        quote! {}
                    };
                    let recursive_pop = if ctx.uses_recursive_ref {
                        pop_recursive_key_eval()
                    } else {
                        quote! {}
                    };
                    let dynamic_key_pushes: Vec<_> = dynamic_bindings
                        .iter()
                        .map(|b| {
                            let key_eval_ident = format_ident!("{}", b.key_eval_name);
                            push_dynamic_key_eval(&b.anchor, &key_eval_ident)
                        })
                        .collect();
                    let dynamic_binding_count = dynamic_bindings.len();
                    let dynamic_push = if ctx.uses_dynamic_ref {
                        quote! {
                            #(#dynamic_key_pushes)*
                        }
                    } else {
                        quote! {}
                    };
                    let dynamic_pop = if ctx.uses_dynamic_ref {
                        pop_dynamic_key_eval_n(dynamic_binding_count)
                    } else {
                        quote! {}
                    };
                    quote! {
                        {
                            #recursive_push
                            #dynamic_push
                            let __result = { #compiled };
                            #dynamic_pop
                            #recursive_pop
                            __result
                        }
                    }
                } else {
                    compiled
                }
            })
        });

        ctx.key_eval_helpers.set_body(&func_name, body);
    } else {
        ctx.key_eval_helpers.set_body(&func_name, quote! { false });
    }

    func_name
}

/// Get or create a helper that determines whether an array index is already
/// evaluated for a referenced schema.
pub(crate) fn get_or_create_item_eval_fn(
    ctx: &mut CompileContext<'_>,
    location: &str,
    schema: &Value,
    schema_base_uri: Arc<Uri<String>>,
) -> String {
    if let Some(name) = ctx.item_eval_helpers.get_name(location) {
        return name.clone();
    }

    let func_name = ctx.item_eval_helpers.alloc_name(location);

    if let Value::Object(schema_obj) = schema {
        let body = ctx.with_item_eval_scope(location, |ctx| {
            let schema_value = Value::Object(schema_obj.clone());
            ctx.with_schema_env(&schema_value, schema_base_uri, |ctx| {
                let compiled = compile_index_evaluated_expr(ctx, schema_obj);
                let is_recursive_anchor = supports_recursive_ref_keyword(ctx.draft)
                    && schema_obj.get("$recursiveAnchor").and_then(Value::as_bool) == Some(true);
                let dynamic_bindings = if ctx.uses_dynamic_ref {
                    collect_dynamic_anchor_bindings(ctx, ctx.current_base_uri.clone())
                } else {
                    Vec::new()
                };
                if ctx.uses_recursive_ref || ctx.uses_dynamic_ref {
                    let func_ident = format_ident!("{}", func_name);
                    let recursive_push = if ctx.uses_recursive_ref {
                        push_recursive_item_eval(&func_ident, is_recursive_anchor)
                    } else {
                        quote! {}
                    };
                    let recursive_pop = if ctx.uses_recursive_ref {
                        pop_recursive_item_eval()
                    } else {
                        quote! {}
                    };
                    let dynamic_item_pushes: Vec<_> = dynamic_bindings
                        .iter()
                        .map(|b| {
                            let item_eval_ident = format_ident!("{}", b.item_eval_name);
                            push_dynamic_item_eval(&b.anchor, &item_eval_ident)
                        })
                        .collect();
                    let dynamic_binding_count = dynamic_bindings.len();
                    let dynamic_push = if ctx.uses_dynamic_ref {
                        quote! {
                            #(#dynamic_item_pushes)*
                        }
                    } else {
                        quote! {}
                    };
                    let dynamic_pop = if ctx.uses_dynamic_ref {
                        pop_dynamic_item_eval_n(dynamic_binding_count)
                    } else {
                        quote! {}
                    };
                    quote! {
                        {
                            #recursive_push
                            #dynamic_push
                            let __result = { #compiled };
                            #dynamic_pop
                            #recursive_pop
                            __result
                        }
                    }
                } else {
                    compiled
                }
            })
        });

        ctx.item_eval_helpers.set_body(&func_name, body);
    } else {
        ctx.item_eval_helpers.set_body(&func_name, quote! { false });
    }

    func_name
}

/// Get or create a function for a reference location.
pub(crate) fn get_or_create_is_valid_fn(
    ctx: &mut CompileContext<'_>,
    location: &str,
    schema: &Value,
    schema_base_uri: Arc<Uri<String>>,
) -> String {
    if let Some(name) = ctx.is_valid_helpers.get_name(location) {
        return name.clone();
    }

    let func_name = ctx.is_valid_helpers.alloc_name(location);
    // Note: alloc_name registers the location→name mapping, so any recursive call
    // to this function for the same location will hit the get_name early-return above
    // before reaching here — making a separate cycle guard unnecessary.

    // The schema_path for errors inside this helper should be the JSON-pointer
    // fragment of the resolved location (e.g. "/$defs/foo" from "base.json#/$defs/foo").
    // For anchor-based fragments (e.g. "#foo"), the path is relative to the schema
    // root, so we use "" as the base path.
    let ref_schema_path =
        location.rsplit_once('#').map_or(
            "",
            |(_, frag)| if frag.starts_with('/') { frag } else { "" },
        );

    let body = ctx.with_is_valid_scope(location, |ctx| {
        ctx.with_schema_env(schema, schema_base_uri, |ctx| {
            // Set schema_path to the location fragment so errors embed the correct path.
            let saved_path = std::mem::replace(&mut ctx.schema_path, ref_schema_path.to_string());
            let compiled = ctx.with_helper_root_scope(|ctx| compile_schema(ctx, schema));
            ctx.schema_path = saved_path;
            let is_recursive_anchor = supports_recursive_ref_keyword(ctx.draft)
                && schema.get("$recursiveAnchor").and_then(Value::as_bool) == Some(true);
            let dynamic_bindings = if ctx.uses_dynamic_ref {
                collect_dynamic_anchor_bindings(ctx, ctx.current_base_uri.clone())
            } else {
                Vec::new()
            };
            if ctx.uses_recursive_ref || ctx.uses_dynamic_ref {
                let func_ident = format_ident!("{}", func_name);
                let key_eval_name =
                    get_or_create_key_eval_fn(ctx, location, schema, ctx.current_base_uri.clone());
                let key_eval_ident = format_ident!("{}", key_eval_name);
                let item_eval_name =
                    get_or_create_item_eval_fn(ctx, location, schema, ctx.current_base_uri.clone());
                let item_eval_ident = format_ident!("{}", item_eval_name);
                let recursive_push = if ctx.uses_recursive_ref {
                    let push_validate = push_recursive_validate(&func_ident, is_recursive_anchor);
                    let push_key = push_recursive_key_eval(&key_eval_ident, is_recursive_anchor);
                    let push_item = push_recursive_item_eval(&item_eval_ident, is_recursive_anchor);
                    quote! {
                        #push_validate
                        #push_key
                        #push_item
                    }
                } else {
                    quote! {}
                };
                let recursive_pop = if ctx.uses_recursive_ref {
                    let pop_item = pop_recursive_item_eval();
                    let pop_key = pop_recursive_key_eval();
                    let pop_validate = pop_recursive_validate();
                    quote! {
                        #pop_item
                        #pop_key
                        #pop_validate
                    }
                } else {
                    quote! {}
                };
                let dynamic_validate_pushes: Vec<_> = dynamic_bindings
                    .iter()
                    .map(|b| {
                        let validate_ident = format_ident!("{}", b.validate_name);
                        push_dynamic_validate(&b.anchor, &validate_ident)
                    })
                    .collect();
                let dynamic_key_pushes: Vec<_> = dynamic_bindings
                    .iter()
                    .map(|b| {
                        let key_eval_ident = format_ident!("{}", b.key_eval_name);
                        push_dynamic_key_eval(&b.anchor, &key_eval_ident)
                    })
                    .collect();
                let dynamic_item_pushes: Vec<_> = dynamic_bindings
                    .iter()
                    .map(|b| {
                        let item_eval_ident = format_ident!("{}", b.item_eval_name);
                        push_dynamic_item_eval(&b.anchor, &item_eval_ident)
                    })
                    .collect();
                let dynamic_binding_count = dynamic_bindings.len();
                let dynamic_push = if ctx.uses_dynamic_ref {
                    quote! {
                        #(#dynamic_validate_pushes)*
                        #(#dynamic_key_pushes)*
                        #(#dynamic_item_pushes)*
                    }
                } else {
                    quote! {}
                };
                let dynamic_pop = if ctx.uses_dynamic_ref {
                    let pop_items = pop_dynamic_item_eval_n(dynamic_binding_count);
                    let pop_keys = pop_dynamic_key_eval_n(dynamic_binding_count);
                    let pop_validate = pop_dynamic_validate_n(dynamic_binding_count);
                    quote! {
                        #pop_items
                        #pop_keys
                        #pop_validate
                    }
                } else {
                    quote! {}
                };
                // Store validate/iter_errors bodies to enable accurate error paths
                // for recursive/dynamic helpers.
                {
                    let validate_stmts = compiled.validate.as_ts();
                    let iter_errors_stmts = compiled.iter_errors.as_ts();
                    let v_ident = format_ident!("{}_v", func_name);
                    let e_ident = format_ident!("{}_e", func_name);
                    // Push _v function to __JSONSCHEMA_RECURSIVE_VALIDATE_STACK so that
                    // $recursiveRef dispatch in validate() context finds the right _v fn.
                    let push_rec_v = if ctx.uses_recursive_ref {
                        push_recursive_validate_v(&v_ident, is_recursive_anchor)
                    } else {
                        quote! {}
                    };
                    let pop_rec_v = if ctx.uses_recursive_ref {
                        pop_recursive_validate_v()
                    } else {
                        quote! {}
                    };
                    let push_rec_e = if ctx.uses_recursive_ref {
                        push_recursive_iter_errors_e(&e_ident, is_recursive_anchor)
                    } else {
                        quote! {}
                    };
                    let pop_rec_e = if ctx.uses_recursive_ref {
                        pop_recursive_iter_errors_e()
                    } else {
                        quote! {}
                    };
                    // Push _v/_e function pointers to dynamic validate/iter_errors stacks.
                    let dynamic_v_pushes: Vec<_> = dynamic_bindings
                        .iter()
                        .map(|b| {
                            let v = format_ident!("{}_v", b.validate_name);
                            push_dynamic_validate_v(&b.anchor, &v)
                        })
                        .collect();
                    let dynamic_e_pushes: Vec<_> = dynamic_bindings
                        .iter()
                        .map(|b| {
                            let e = format_ident!("{}_e", b.validate_name);
                            push_dynamic_iter_errors_e(&b.anchor, &e)
                        })
                        .collect();
                    let dynamic_binding_count = dynamic_bindings.len();
                    let push_dyn_v = if ctx.uses_dynamic_ref {
                        quote! { #(#dynamic_v_pushes)* }
                    } else {
                        quote! {}
                    };
                    let pop_dyn_v = if ctx.uses_dynamic_ref {
                        pop_dynamic_validate_v_n(dynamic_binding_count)
                    } else {
                        quote! {}
                    };
                    let push_dyn_e = if ctx.uses_dynamic_ref {
                        quote! { #(#dynamic_e_pushes)* }
                    } else {
                        quote! {}
                    };
                    let pop_dyn_e = if ctx.uses_dynamic_ref {
                        pop_dynamic_iter_errors_e_n(dynamic_binding_count)
                    } else {
                        quote! {}
                    };
                    // Use an IIFE to capture early returns so the stack is always popped.
                    let validate_body = quote! {
                        #recursive_push
                        #push_rec_v
                        #dynamic_push
                        #push_dyn_v
                        let __r = (|| -> Option<jsonschema::ValidationError<'__i>> {
                            #validate_stmts
                            None
                        })();
                        #pop_dyn_v
                        #dynamic_pop
                        #pop_rec_v
                        #recursive_pop
                        if let Some(__e) = __r { return Some(__e); }
                        None
                    };
                    let iter_errors_body = quote! {
                        #recursive_push
                        #push_rec_e
                        #dynamic_push
                        #push_dyn_e
                        { #iter_errors_stmts }
                        #pop_dyn_e
                        #dynamic_pop
                        #pop_rec_e
                        #recursive_pop
                    };
                    ctx.is_valid_helpers.set_validate_iter_bodies(
                        &func_name,
                        validate_body,
                        iter_errors_body,
                    );
                }
                quote! {
                    {
                        #recursive_push
                        #dynamic_push
                        let __result = { #compiled };
                        #dynamic_pop
                        #recursive_pop
                        __result
                    }
                }
            } else {
                // Non-recursive, non-dynamic: store the complete validate_v function body.
                let validate_stmts = match &compiled.validate {
                    super::expr::ValidateBlock::Expr(v) => v.clone(),
                    super::expr::ValidateBlock::AlwaysValid => quote! {},
                };
                let iter_errors_body = match &compiled.iter_errors {
                    super::expr::ValidateBlock::Expr(ie) => ie.clone(),
                    super::expr::ValidateBlock::AlwaysValid => quote! {},
                };
                let validate_body = quote! { #validate_stmts None };
                ctx.is_valid_helpers.set_validate_iter_bodies(
                    &func_name,
                    validate_body,
                    iter_errors_body,
                );
                compiled.into_token_stream()
            }
        })
    });
    ctx.is_valid_helpers.set_body(&func_name, body);

    func_name
}
