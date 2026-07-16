use quote::{format_ident, quote};
use referencing::Uri;
use serde_json::Value;
use std::{collections::HashSet, sync::Arc};

use crate::context::CompileContext;

use super::{
    compile_schema,
    draft::DraftExt,
    keywords::unevaluated::{compile_index_evaluated_expr, compile_key_evaluated_expr, GuardHoist},
    refs::resolve_ref,
    stack_emit::{
        pop_dynamic_collect_n, pop_dynamic_validate_n, pop_recursive_collect,
        pop_recursive_validate, push_dynamic_collect, push_dynamic_validate,
        push_recursive_collect, push_recursive_validate, stack_scoped_body, IS_VALID_FAMILY,
        ITEM_EVAL_FAMILY, KEY_EVAL_FAMILY,
    },
};

#[derive(Clone)]
pub(crate) struct DynamicAnchorBinding {
    pub(crate) anchor: String,
    pub(crate) is_valid_name: String,
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

fn collect_dynamic_anchor_names(schema: &Value, names: &mut HashSet<String>) {
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
    let Some(bindings) = ctx.with_dynamic_anchor_bindings_scope(cache_key.clone(), |ctx| {
        let resolver = ctx.config.registry.resolver((*schema_base_uri).clone());
        let resource = resolver
            .lookup("")
            .expect("the current base URI resolves to its own resource");
        let resource_schema = resource.contents();

        let mut names = HashSet::new();
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
                let is_valid_name = get_or_create_is_valid_fn(
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
                    is_valid_name,
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
    let bindings =
        bindings.expect("dynamic anchor bindings are produced whenever the scope is entered");
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
    if let Some(name) = ctx.key_eval_fns.get_name(location) {
        return name.clone();
    }

    let func_name = ctx.key_eval_fns.alloc_name(location);

    if let Value::Object(schema_obj) = schema {
        let body = ctx.with_key_eval_scope(location, |ctx| {
            let schema_value = Value::Object(schema_obj.clone());
            ctx.with_schema_env(&schema_value, schema_base_uri, |ctx| {
                let compiled = compile_key_evaluated_expr(ctx, schema_obj, true);
                let is_recursive_anchor = ctx.draft.supports_recursive_ref_keyword()
                    && schema_obj.get("$recursiveAnchor").and_then(Value::as_bool) == Some(true);
                let dynamic_bindings = if ctx.uses_dynamic_ref {
                    collect_dynamic_anchor_bindings(ctx, ctx.current_base_uri.clone())
                } else {
                    Vec::new()
                };
                stack_scoped_body(
                    &KEY_EVAL_FAMILY,
                    &format_ident!("{}", func_name),
                    is_recursive_anchor,
                    ctx.uses_recursive_ref,
                    ctx.uses_dynamic_ref,
                    &dynamic_bindings,
                    compiled,
                )
            })
        });

        ctx.key_eval_fns.set_body(&func_name, body);
    } else {
        ctx.key_eval_fns.set_body(&func_name, quote! { false });
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
    if let Some(name) = ctx.item_eval_fns.get_name(location) {
        return name.clone();
    }

    let func_name = ctx.item_eval_fns.alloc_name(location);

    if let Value::Object(schema_obj) = schema {
        let body = ctx.with_item_eval_scope(location, |ctx| {
            let schema_value = Value::Object(schema_obj.clone());
            ctx.with_schema_env(&schema_value, schema_base_uri, |ctx| {
                let mut hoist = GuardHoist::inline();
                let compiled = compile_index_evaluated_expr(ctx, schema_obj, &mut hoist);
                let is_recursive_anchor = ctx.draft.supports_recursive_ref_keyword()
                    && schema_obj.get("$recursiveAnchor").and_then(Value::as_bool) == Some(true);
                let dynamic_bindings = if ctx.uses_dynamic_ref {
                    collect_dynamic_anchor_bindings(ctx, ctx.current_base_uri.clone())
                } else {
                    Vec::new()
                };
                stack_scoped_body(
                    &ITEM_EVAL_FAMILY,
                    &format_ident!("{}", func_name),
                    is_recursive_anchor,
                    ctx.uses_recursive_ref,
                    ctx.uses_dynamic_ref,
                    &dynamic_bindings,
                    compiled,
                )
            })
        });

        ctx.item_eval_fns.set_body(&func_name, body);
    } else {
        ctx.item_eval_fns.set_body(&func_name, quote! { false });
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
    get_or_create_is_valid_fn_with(ctx, location, schema, schema_base_uri, compile_schema)
}

/// Like [`get_or_create_is_valid_fn`], with a custom body compiler.
pub(crate) fn get_or_create_is_valid_fn_with(
    ctx: &mut CompileContext<'_>,
    location: &str,
    schema: &Value,
    schema_base_uri: Arc<Uri<String>>,
    compile: impl Fn(&mut CompileContext<'_>, &Value) -> crate::codegen::CompiledExpr,
) -> String {
    if let Some(name) = ctx.is_valid_fns.get_name(location) {
        return name.clone();
    }

    let func_name = ctx.is_valid_fns.alloc_name(location);

    // Errors inside this helper carry the percent-decoded JSON Pointer fragment of
    // `location` ("/$defs/foo" from "base.json#/$defs/foo"); anchor fragments ("#foo") use "".
    let ref_schema_path: String =
        location
            .rsplit_once('#')
            .map_or_else(String::new, |(_, frag)| {
                if frag.starts_with('/') {
                    percent_encoding::percent_decode_str(frag)
                        .decode_utf8_lossy()
                        .into_owned()
                } else {
                    String::new()
                }
            });

    let body = ctx.with_is_valid_scope(location, |ctx| {
        ctx.with_schema_env(schema, schema_base_uri, |ctx| {
            // Set schema_path to the location fragment so errors embed the correct path.
            let compiled = ctx.with_schema_path_swap(ref_schema_path, |ctx| {
                ctx.with_helper_root_scope(|ctx| compile(ctx, schema))
            });
            let is_recursive_anchor = ctx.draft.supports_recursive_ref_keyword()
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
                // Pops mirror pushes in reverse family order on every path.
                let families = [
                    (&IS_VALID_FAMILY, &func_ident),
                    (&KEY_EVAL_FAMILY, &key_eval_ident),
                    (&ITEM_EVAL_FAMILY, &item_eval_ident),
                ];
                let recursive_push = if ctx.uses_recursive_ref {
                    let pushes = families
                        .iter()
                        .map(|(family, ident)| (family.push_recursive)(ident, is_recursive_anchor));
                    quote! { #(#pushes)* }
                } else {
                    quote! {}
                };
                let recursive_pop = if ctx.uses_recursive_ref {
                    let pops = families
                        .iter()
                        .rev()
                        .map(|(family, _)| (family.pop_recursive)());
                    quote! { #(#pops)* }
                } else {
                    quote! {}
                };
                let dynamic_binding_count = dynamic_bindings.len();
                let dynamic_push = if ctx.uses_dynamic_ref {
                    let pushes: Vec<_> = families
                        .iter()
                        .flat_map(|(family, _)| family.dynamic_pushes(&dynamic_bindings))
                        .collect();
                    quote! { #(#pushes)* }
                } else {
                    quote! {}
                };
                let dynamic_pop = if ctx.uses_dynamic_ref {
                    let pops = families
                        .iter()
                        .rev()
                        .map(|(family, _)| (family.pop_dynamic_n)(dynamic_binding_count));
                    quote! { #(#pops)* }
                } else {
                    quote! {}
                };
                // Store the validate body so $recursiveRef/$dynamicRef dispatch
                // in validate() context finds the right validate function.
                {
                    let validate_stmts = compiled.validate.as_token_stream();
                    let validate_ident = format_ident!("{}_validate", func_name);
                    let recursive_validate_push = if ctx.uses_recursive_ref {
                        push_recursive_validate(&validate_ident, is_recursive_anchor)
                    } else {
                        quote! {}
                    };
                    let recursive_validate_pop = if ctx.uses_recursive_ref {
                        pop_recursive_validate()
                    } else {
                        quote! {}
                    };
                    let dynamic_validate_pushes: Vec<_> = dynamic_bindings
                        .iter()
                        .map(|b| {
                            let binding_validate_ident =
                                format_ident!("{}_validate", b.is_valid_name);
                            push_dynamic_validate(&b.anchor, &binding_validate_ident)
                        })
                        .collect();
                    let dynamic_binding_count = dynamic_bindings.len();
                    let dynamic_validate_push = if ctx.uses_dynamic_ref {
                        quote! { #(#dynamic_validate_pushes)* }
                    } else {
                        quote! {}
                    };
                    let dynamic_validate_pop = if ctx.uses_dynamic_ref {
                        pop_dynamic_validate_n(dynamic_binding_count)
                    } else {
                        quote! {}
                    };
                    let validate_body = if recursive_validate_push.is_empty()
                        && dynamic_validate_push.is_empty()
                        && recursive_push.is_empty()
                        && dynamic_push.is_empty()
                    {
                        quote! { #validate_stmts None }
                    } else {
                        // Use an IIFE to capture early returns so the stack is always popped.
                        quote! {
                            #recursive_push
                            #recursive_validate_push
                            #dynamic_push
                            #dynamic_validate_push
                            let __r = (|| -> Option<__VE<'__i>> {
                                #validate_stmts
                                None
                            })();
                            #dynamic_validate_pop
                            #dynamic_pop
                            #recursive_validate_pop
                            #recursive_pop
                            if let Some(__e) = __r { return Some(__e); }
                            None
                        }
                    };
                    ctx.is_valid_fns
                        .set_validate_body(&func_name, validate_body);
                }
                {
                    let collect_stmts = compiled.collect.as_token_stream();
                    let collect_body = if recursive_push.is_empty() && dynamic_push.is_empty() {
                        quote! { #collect_stmts }
                    } else {
                        // Collect never returns early, so push/pop wrap the statements directly.
                        let recursive_collect_push = if ctx.uses_recursive_ref {
                            let collect_ident = format_ident!("{}_collect_errors", func_name);
                            push_recursive_collect(&collect_ident, is_recursive_anchor)
                        } else {
                            quote! {}
                        };
                        let recursive_collect_pop = if ctx.uses_recursive_ref {
                            pop_recursive_collect()
                        } else {
                            quote! {}
                        };
                        let dynamic_collect_pushes: Vec<_> = dynamic_bindings
                            .iter()
                            .map(|b| {
                                let binding_collect_ident =
                                    format_ident!("{}_collect_errors", b.is_valid_name);
                                push_dynamic_collect(&b.anchor, &binding_collect_ident)
                            })
                            .collect();
                        let dynamic_collect_push = if ctx.uses_dynamic_ref {
                            quote! { #(#dynamic_collect_pushes)* }
                        } else {
                            quote! {}
                        };
                        let dynamic_collect_pop = if ctx.uses_dynamic_ref {
                            pop_dynamic_collect_n(dynamic_bindings.len())
                        } else {
                            quote! {}
                        };
                        quote! {
                            #recursive_push
                            #recursive_collect_push
                            #dynamic_push
                            #dynamic_collect_push
                            #collect_stmts
                            #dynamic_collect_pop
                            #dynamic_pop
                            #recursive_collect_pop
                            #recursive_pop
                        }
                    };
                    ctx.is_valid_fns.set_collect_body(&func_name, collect_body);
                }
                if recursive_push.is_empty() && dynamic_push.is_empty() {
                    compiled.into_token_stream()
                } else {
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
                }
            } else {
                // Non-recursive, non-dynamic: store the complete validate function body.
                let validate_stmts = match &compiled.validate {
                    super::expr::ValidateBlock::Expr(v) => v.clone(),
                    super::expr::ValidateBlock::AlwaysValid => quote! {},
                };
                let validate_body = quote! { #validate_stmts None };
                ctx.is_valid_fns
                    .set_validate_body(&func_name, validate_body);
                ctx.is_valid_fns
                    .set_collect_body(&func_name, compiled.collect.as_token_stream());
                compiled.into_token_stream()
            }
        })
    });
    ctx.is_valid_fns.set_body(&func_name, body);

    func_name
}
