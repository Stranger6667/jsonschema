use crate::codegen::emit::ValueEmitter;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use referencing::{write_escaped_str, Uri};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use crate::context::{CompileContext, Dialect, ErrorsFn};

use super::{
    compile_schema,
    draft::DraftExt,
    keywords::unevaluated::{compile_index_evaluated_expr, compile_key_evaluated_expr, GuardHoist},
    refs::resolve_ref,
    stack_emit::{
        pop_dynamic_errors_n, pop_recursive_errors, push_dynamic_errors, push_recursive_errors,
        stack_scoped_body, IS_VALID_FAMILY, ITEM_EVAL_FAMILY, KEY_EVAL_FAMILY,
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

pub(crate) fn collect_dynamic_anchor_bindings<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
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
                if dynamic_ref_anchor_name(&reference, resolved.schema).is_none() {
                    continue;
                }
                let is_valid_name = get_or_create_is_valid_fn(
                    ctx,
                    &resolved.location,
                    resolved.resource_start,
                    resolved.schema,
                    resolved.base_uri.clone(),
                    resolved.dialect.clone(),
                );
                let key_eval_name = get_or_create_key_eval_fn(
                    ctx,
                    &resolved.location,
                    resolved.schema,
                    resolved.base_uri.clone(),
                    resolved.dialect.clone(),
                );
                let item_eval_name = get_or_create_item_eval_fn(
                    ctx,
                    &resolved.location,
                    resolved.schema,
                    resolved.base_uri,
                    resolved.dialect,
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
pub(crate) fn get_or_create_key_eval_fn<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    location: &str,
    schema: &Value,
    schema_base_uri: Arc<Uri<String>>,
    dialect: Dialect,
) -> String {
    if let Some(name) = ctx.key_eval_fns.get_name(location) {
        return name.clone();
    }

    let func_name = ctx.key_eval_fns.alloc_name(location);

    if let Value::Object(schema_obj) = schema {
        let body = ctx.with_key_eval_scope(location, |ctx| {
            ctx.with_schema_env(schema_base_uri, dialect, |ctx| {
                let compiled = compile_key_evaluated_expr(ctx, schema_obj, true, false, None);
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
pub(crate) fn get_or_create_item_eval_fn<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    location: &str,
    schema: &Value,
    schema_base_uri: Arc<Uri<String>>,
    dialect: Dialect,
) -> String {
    if let Some(name) = ctx.item_eval_fns.get_name(location) {
        return name.clone();
    }

    let func_name = ctx.item_eval_fns.alloc_name(location);

    if let Value::Object(schema_obj) = schema {
        let body = ctx.with_item_eval_scope(location, |ctx| {
            ctx.with_schema_env(schema_base_uri, dialect, |ctx| {
                let mut hoist = GuardHoist::inline();
                let compiled = compile_index_evaluated_expr(ctx, schema_obj, &mut hoist, None);
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

/// JSON Pointer of the schema at a resolved `location` within its resource: the percent-decoded
/// fragment ("/$defs/foo" from "base.json#/$defs/foo"), or where an anchor fragment ("#foo") sits.
pub(crate) fn location_pointer<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    location: &str,
    base_uri: &Uri<String>,
) -> String {
    location.rfind('#').map_or_else(String::new, |hash| {
        // `#foo`, borrowed from `location` to look the anchor up
        let reference = &location[hash..];
        let frag = &reference[1..];
        if frag.starts_with('/') {
            percent_encoding::percent_decode_str(frag)
                .decode_utf8_lossy()
                .into_owned()
        } else if frag.is_empty() {
            String::new()
        } else {
            named_target_pointer(ctx, base_uri, reference)
        }
    })
}

/// JSON Pointer of the schema `reference` (`#foo`) names within the resource at `base_uri`.
fn named_target_pointer<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    base_uri: &Uri<String>,
    reference: &str,
) -> String {
    let resolver = ctx.config.registry.resolver(base_uri.clone());
    let (Ok(resource), Ok(target)) = (resolver.lookup(""), resolver.lookup(reference)) else {
        // A guard helper's location is a serialized subschema, which names no anchor.
        return String::new();
    };
    anchor_pointer(ctx, resource.contents(), target.contents())
}

/// JSON Pointer of `target`, a schema an anchor names, within the resource rooted at `root`.
pub(crate) fn anchor_pointer<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    root: &Value,
    target: &Value,
) -> String {
    ctx.anchor_pointers
        .entry(std::ptr::from_ref(root) as usize)
        .or_insert_with(|| {
            let mut index = HashMap::new();
            index_anchors(root, &mut String::new(), &mut index);
            index
        })
        .get(&(std::ptr::from_ref(target) as usize))
        .cloned()
        .expect("An anchor lives in the resource it is looked up in")
}

/// Keywords that can name the schema holding them, across all drafts.
const ANCHOR_KEYWORDS: [&str; 4] = ["$anchor", "$dynamicAnchor", "$id", "id"];

fn index_anchors(value: &Value, pointer: &mut String, index: &mut HashMap<usize, String>) {
    let parent_len = pointer.len();
    match value {
        Value::Object(map) => {
            if ANCHOR_KEYWORDS
                .iter()
                .any(|keyword| map.contains_key(*keyword))
            {
                index.insert(std::ptr::from_ref(value) as usize, pointer.clone());
            }
            for (key, child) in map {
                pointer.push('/');
                write_escaped_str(pointer, key);
                index_anchors(child, pointer, index);
                pointer.truncate(parent_len);
            }
        }
        Value::Array(items) => {
            for (idx, child) in items.iter().enumerate() {
                pointer.push('/');
                pointer.push_str(&idx.to_string());
                index_anchors(child, pointer, index);
                pointer.truncate(parent_len);
            }
        }
        _ => {}
    }
}

/// Get or create a function for a reference location, written in `dialect`. Its pointer enters the
/// resource at `schema_base_uri` after `resource_start` bytes.
pub(crate) fn get_or_create_is_valid_fn<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    location: &str,
    resource_start: usize,
    schema: &Value,
    schema_base_uri: Arc<Uri<String>>,
    dialect: Dialect,
) -> String {
    get_or_create_is_valid_fn_with(
        ctx,
        location,
        resource_start,
        schema,
        schema_base_uri,
        dialect,
        compile_schema,
    )
}

/// The base the runtime locates keywords against, or `None` for the internal default one.
pub(crate) fn located_base(base: &Uri<String>) -> Option<String> {
    (base.scheme().as_str() != "json-schema").then(|| base.as_str().to_string())
}

/// The errors function `{func_name}_errors` of the subschema at `pointer` in the resource at
/// `base`, which starts `resource_start` bytes into `pointer`: `body` reports its errors relative
/// to it, between `enter` and `leave`, and the runtime places them below the reference that
/// reaches it.
fn errors_fn(
    func_name: &str,
    enter: &TokenStream,
    leave: &TokenStream,
    body: TokenStream,
    pointer: &str,
    base: Option<&str>,
    resource_start: usize,
) -> ErrorsFn {
    let body_ident = format_ident!("{}_errors_body", func_name);
    let base = base.map_or_else(|| quote! { None }, |base| quote! { Some(#base) });
    ErrorsFn {
        entry: quote! {
            let __start = __errors.mark();
            #enter
            let __stop = #body_ident(instance, __path, __errors);
            #leave
            __errors.reached(__start, #pointer, #base, #resource_start, __site);
            __stop
        },
        body,
    }
}

/// The code a `$recursiveRef` landing on `schema` runs, when `schema` is a resource root reached
/// through another `location`: the code of the root as a `$recursiveRef` in it names it.
fn recursive_anchor_name<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    location: &str,
    schema: &Value,
    is_recursive_anchor: bool,
) -> Option<String> {
    if !is_recursive_anchor {
        return None;
    }
    let root = resolve_ref(ctx, "#").expect("a resource's root resolves");
    if root.location == location || !std::ptr::eq(root.schema, schema) {
        return None;
    }
    Some(get_or_create_is_valid_fn(
        ctx,
        &root.location,
        root.resource_start,
        root.schema,
        root.base_uri,
        root.dialect,
    ))
}

/// Like [`get_or_create_is_valid_fn`], with a custom body compiler.
pub(crate) fn get_or_create_is_valid_fn_with<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    location: &str,
    resource_start: usize,
    schema: &Value,
    schema_base_uri: Arc<Uri<String>>,
    dialect: Dialect,
    compile: impl Fn(&mut CompileContext<'_, E>, &Value) -> crate::codegen::CompiledExpr,
) -> String {
    if let Some(name) = ctx.is_valid_fns.get_name(location).cloned() {
        if ctx.is_valid_fns.is_compiling(location) {
            ctx.reentered_targets.insert(location.to_string());
        }
        return name;
    }

    let func_name = ctx.is_valid_fns.alloc_name(location);
    ctx.compiled_within
        .insert(location.to_string(), ctx.compiling_stack.last().cloned());
    ctx.target_locations
        .entry(std::ptr::from_ref(schema).addr())
        .or_default()
        .push(location.to_string());

    // Errors inside this helper carry the target's JSON Pointer from the document its location
    // names.
    let ref_schema_path = location_pointer(ctx, location, &schema_base_uri);

    let pointer = ref_schema_path.clone();
    let base = located_base(&schema_base_uri);
    let body = ctx.with_is_valid_scope(location, |ctx| {
        ctx.with_schema_env(schema_base_uri, dialect, |ctx| {
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
                let recursive_name = if ctx.uses_recursive_ref {
                    recursive_anchor_name(ctx, location, schema, is_recursive_anchor)
                        .unwrap_or_else(|| func_name.clone())
                } else {
                    func_name.clone()
                };
                let recursive_ident = format_ident!("{}", recursive_name);
                let key_eval_name = get_or_create_key_eval_fn(
                    ctx,
                    location,
                    schema,
                    ctx.current_base_uri.clone(),
                    ctx.dialect(),
                );
                let key_eval_ident = format_ident!("{}", key_eval_name);
                let item_eval_name = get_or_create_item_eval_fn(
                    ctx,
                    location,
                    schema,
                    ctx.current_base_uri.clone(),
                    ctx.dialect(),
                );
                let item_eval_ident = format_ident!("{}", item_eval_name);
                // Pops mirror pushes in reverse family order on every path.
                let families = [
                    (&IS_VALID_FAMILY, &recursive_ident),
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
                {
                    let errors_stmts = compiled.errors.as_token_stream();
                    let (enter, leave) = if recursive_push.is_empty() && dynamic_push.is_empty() {
                        (quote! {}, quote! {})
                    } else {
                        let errors_ident = format_ident!("{}_errors", recursive_name);
                        let recursive_errors_push = if ctx.uses_recursive_ref {
                            push_recursive_errors(&errors_ident, is_recursive_anchor)
                        } else {
                            quote! {}
                        };
                        let recursive_errors_pop = if ctx.uses_recursive_ref {
                            pop_recursive_errors()
                        } else {
                            quote! {}
                        };
                        let dynamic_errors_pushes: Vec<_> = dynamic_bindings
                            .iter()
                            .map(|b| {
                                let binding_errors_ident =
                                    format_ident!("{}_errors", b.is_valid_name);
                                push_dynamic_errors(&b.anchor, &binding_errors_ident)
                            })
                            .collect();
                        let dynamic_errors_push = if ctx.uses_dynamic_ref {
                            quote! { #(#dynamic_errors_pushes)* }
                        } else {
                            quote! {}
                        };
                        let dynamic_errors_pop = if ctx.uses_dynamic_ref {
                            pop_dynamic_errors_n(dynamic_bindings.len())
                        } else {
                            quote! {}
                        };
                        (
                            quote! {
                                #recursive_push
                                #recursive_errors_push
                                #dynamic_push
                                #dynamic_errors_push
                            },
                            quote! {
                                #dynamic_errors_pop
                                #dynamic_pop
                                #recursive_errors_pop
                                #recursive_pop
                            },
                        )
                    };
                    ctx.is_valid_fns.set_errors_body(
                        &func_name,
                        errors_fn(
                            &func_name,
                            &enter,
                            &leave,
                            quote! { #errors_stmts false },
                            &pointer,
                            base.as_deref(),
                            resource_start,
                        ),
                    );
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
                let errors_stmts = compiled.errors.as_token_stream();
                ctx.is_valid_fns.set_errors_body(
                    &func_name,
                    errors_fn(
                        &func_name,
                        &quote! {},
                        &quote! {},
                        quote! { #errors_stmts false },
                        &pointer,
                        base.as_deref(),
                        resource_start,
                    ),
                );
                compiled.into_token_stream()
            }
        })
    });
    ctx.is_valid_fns.set_body(&func_name, body);

    func_name
}
