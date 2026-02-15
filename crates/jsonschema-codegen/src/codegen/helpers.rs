use quote::{format_ident, quote};
use referencing::Uri;
use serde_json::Value;
use std::sync::Arc;

use crate::context::CompileContext;

use super::{
    compile_index_evaluated_expr, compile_key_evaluated_expr, compile_schema,
    refs::resolve_ref,
    stack_emit::{
        pop_dynamic_item_eval_n, pop_dynamic_key_eval_n, pop_dynamic_validate_n,
        pop_recursive_item_eval, pop_recursive_key_eval, pop_recursive_validate,
        push_dynamic_item_eval, push_dynamic_key_eval, push_dynamic_validate,
        push_recursive_item_eval, push_recursive_key_eval, push_recursive_validate,
    },
    supports_dynamic_ref_keyword, supports_recursive_ref_keyword,
};

pub(in crate::codegen) fn dynamic_ref_anchor_name(
    reference: &str,
    resolved_schema: &Value,
) -> Option<String> {
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

pub(in crate::codegen) fn collect_dynamic_anchor_bindings(
    ctx: &mut CompileContext<'_>,
    schema_base_uri: Arc<Uri<String>>,
) -> Vec<(String, String, String, String)> {
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
                let validate_name = get_or_create_function(
                    ctx,
                    &resolved.location,
                    &resolved.schema,
                    resolved.base_uri.clone(),
                );
                let key_eval_name = get_or_create_eval_function(
                    ctx,
                    &resolved.location,
                    &resolved.schema,
                    resolved.base_uri.clone(),
                );
                let item_eval_name = get_or_create_item_eval_function(
                    ctx,
                    &resolved.location,
                    &resolved.schema,
                    resolved.base_uri,
                );
                bindings.push((anchor, validate_name, key_eval_name, item_eval_name));
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
pub(in crate::codegen) fn get_or_create_eval_function(
    ctx: &mut CompileContext<'_>,
    location: &str,
    schema: &Value,
    schema_base_uri: Arc<Uri<String>>,
) -> String {
    if let Some(name) = ctx.location_to_eval_function.get(location) {
        return name.clone();
    }

    let func_name = format!("eval_ref_{}", ctx.eval_counter);
    ctx.eval_counter += 1;
    ctx.location_to_eval_function
        .insert(location.to_string(), func_name.clone());

    if let Value::Object(schema_obj) = schema {
        let body = ctx.with_eval_compilation_scope(location, |ctx| {
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
                        .map(|(anchor, _, key_eval_name, _)| {
                            let key_eval_ident = format_ident!("{}", key_eval_name);
                            push_dynamic_key_eval(anchor, &key_eval_ident)
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

        ctx.eval_bodies.insert(func_name.clone(), body);
    } else {
        ctx.eval_bodies.insert(func_name.clone(), quote! { false });
    }

    func_name
}

/// Get or create a helper that determines whether an array index is already
/// evaluated for a referenced schema.
pub(in crate::codegen) fn get_or_create_item_eval_function(
    ctx: &mut CompileContext<'_>,
    location: &str,
    schema: &Value,
    schema_base_uri: Arc<Uri<String>>,
) -> String {
    if let Some(name) = ctx.location_to_item_eval_function.get(location) {
        return name.clone();
    }

    let func_name = format!("eval_items_ref_{}", ctx.item_eval_counter);
    ctx.item_eval_counter += 1;
    ctx.location_to_item_eval_function
        .insert(location.to_string(), func_name.clone());

    if let Value::Object(schema_obj) = schema {
        let body = ctx.with_item_eval_compilation_scope(location, |ctx| {
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
                        .map(|(anchor, _, _, item_eval_name)| {
                            let item_eval_ident = format_ident!("{}", item_eval_name);
                            push_dynamic_item_eval(anchor, &item_eval_ident)
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

        ctx.item_eval_bodies.insert(func_name.clone(), body);
    } else {
        ctx.item_eval_bodies
            .insert(func_name.clone(), quote! { false });
    }

    func_name
}

/// Get or create a function for a reference location.
pub(in crate::codegen) fn get_or_create_function(
    ctx: &mut CompileContext<'_>,
    location: &str,
    schema: &Value,
    schema_base_uri: Arc<Uri<String>>,
) -> String {
    // Check if function already exists
    if let Some(name) = ctx.location_to_function.get(location) {
        return name.clone();
    }

    // Create new function
    let func_name = format!("validate_ref_{}", ctx.ref_counter);
    ctx.ref_counter += 1;

    // Store function info
    ctx.location_to_function
        .insert(location.to_string(), func_name.clone());

    // Check for recursion
    if ctx.seen.contains(location) {
        // Recursive - create stub that will be filled later
        // For now, just return true to avoid infinite recursion during compilation
        ctx.is_valid_bodies
            .insert(func_name.clone(), quote! { true });
    } else {
        let body = ctx.with_ref_compilation_scope(location, |ctx| {
            ctx.with_schema_env(schema, schema_base_uri, |ctx| {
                let compiled = ctx.with_helper_root_scope(|ctx| compile_schema(ctx, schema));
                let is_recursive_anchor = supports_recursive_ref_keyword(ctx.draft)
                    && schema.get("$recursiveAnchor").and_then(Value::as_bool) == Some(true);
                let dynamic_bindings = if ctx.uses_dynamic_ref {
                    collect_dynamic_anchor_bindings(ctx, ctx.current_base_uri.clone())
                } else {
                    Vec::new()
                };
                if ctx.uses_recursive_ref || ctx.uses_dynamic_ref {
                    let func_ident = format_ident!("{}", func_name);
                    let key_eval_name = get_or_create_eval_function(
                        ctx,
                        location,
                        schema,
                        ctx.current_base_uri.clone(),
                    );
                    let key_eval_ident = format_ident!("{}", key_eval_name);
                    let item_eval_name = get_or_create_item_eval_function(
                        ctx,
                        location,
                        schema,
                        ctx.current_base_uri.clone(),
                    );
                    let item_eval_ident = format_ident!("{}", item_eval_name);
                    let recursive_push = if ctx.uses_recursive_ref {
                        let push_validate =
                            push_recursive_validate(&func_ident, is_recursive_anchor);
                        let push_key =
                            push_recursive_key_eval(&key_eval_ident, is_recursive_anchor);
                        let push_item =
                            push_recursive_item_eval(&item_eval_ident, is_recursive_anchor);
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
                        .map(|(anchor, validate_name, _, _)| {
                            let validate_ident = format_ident!("{}", validate_name);
                            push_dynamic_validate(anchor, &validate_ident)
                        })
                        .collect();
                    let dynamic_key_pushes: Vec<_> = dynamic_bindings
                        .iter()
                        .map(|(anchor, _, key_eval_name, _)| {
                            let key_eval_ident = format_ident!("{}", key_eval_name);
                            push_dynamic_key_eval(anchor, &key_eval_ident)
                        })
                        .collect();
                    let dynamic_item_pushes: Vec<_> = dynamic_bindings
                        .iter()
                        .map(|(anchor, _, _, item_eval_name)| {
                            let item_eval_ident = format_ident!("{}", item_eval_name);
                            push_dynamic_item_eval(anchor, &item_eval_ident)
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

        ctx.is_valid_bodies.insert(func_name.clone(), body);
    }

    func_name
}
