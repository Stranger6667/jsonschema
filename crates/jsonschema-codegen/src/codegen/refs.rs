use super::{
    errors::invalid_schema_expected_string_keyword_expression,
    helpers::{collect_dynamic_anchor_bindings, dynamic_ref_anchor_name, get_or_create_function},
    invalid_schema_expression,
    stack_emit::{pop_dynamic_validate_n, push_dynamic_validate},
    CompileContext,
};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use referencing::Uri;
use serde_json::Value;
use std::{borrow::Cow, sync::Arc};

pub(in crate::codegen) struct ResolvedRef {
    pub(in crate::codegen) schema: Value,
    pub(in crate::codegen) location: String,
    /// Base URI of the resolved schema (for resolving nested references)
    pub(in crate::codegen) base_uri: Arc<Uri<String>>,
}

/// Resolve a short top-level `$ref` chain for branch-shape analysis.
pub(super) fn resolve_top_level_ref_for_one_of_analysis<'b>(
    ctx: &mut CompileContext<'_>,
    schema: &'b Value,
) -> Cow<'b, Value> {
    let mut current = Cow::Borrowed(schema);
    for _ in 0..8 {
        let Value::Object(obj) = current.as_ref() else {
            break;
        };
        let Some(reference) = obj.get("$ref").and_then(Value::as_str) else {
            break;
        };
        let Ok(resolved) = resolve_ref(ctx, reference) else {
            break;
        };
        current = Cow::Owned(resolved.schema);
    }
    current
}

/// Compile a $ref keyword.
pub(super) fn compile_ref(ctx: &mut CompileContext<'_>, value: &Value) -> TokenStream {
    let Some(reference) = value.as_str() else {
        return invalid_schema_expected_string_keyword_expression("$ref");
    };

    let resolved = match resolve_ref(ctx, reference) {
        Ok(resolved) => resolved,
        Err(err) => {
            let message = format!("Failed to resolve `$ref` `{reference}`: {err}");
            return invalid_schema_expression(&message);
        }
    };

    // Break only direct self-recursion at helper root. Recursive uses through
    // nested applicators (e.g. `items`) must still call back into the helper.
    let is_direct_self_ref = ctx
        .compiling_stack
        .last()
        .is_some_and(|location| location == &resolved.location)
        && ctx
            .helper_root_depths
            .last()
            .is_some_and(|depth| ctx.schema_depth == depth + 1);
    if is_direct_self_ref {
        return quote! { true };
    }

    // Get or create function for this location, passing the resolved base URI
    let func_name =
        get_or_create_function(ctx, &resolved.location, &resolved.schema, resolved.base_uri);
    let func_ident = format_ident!("{}", func_name);
    let call_scope_bindings = if ctx.uses_dynamic_ref {
        collect_dynamic_anchor_bindings(ctx, ctx.current_base_uri.clone())
    } else {
        Vec::new()
    };

    if ctx.uses_dynamic_ref && !call_scope_bindings.is_empty() {
        let call_scope_pushes: Vec<_> = call_scope_bindings
            .iter()
            .map(|(anchor, validate_name, _, _)| {
                let validate_ident = format_ident!("{}", validate_name);
                push_dynamic_validate(anchor, &validate_ident)
            })
            .collect();
        let call_scope_count = call_scope_bindings.len();
        let dynamic_pop = pop_dynamic_validate_n(call_scope_count);
        quote! {
            {
                #(#call_scope_pushes)*
                let __result = #func_ident(instance);
                #dynamic_pop
                __result
            }
        }
    } else {
        // Helpers are free functions inside the private __impl module
        quote! { #func_ident(instance) }
    }
}

/// Compile a $dynamicRef keyword (draft 2020-12).
pub(super) fn compile_dynamic_ref(ctx: &mut CompileContext<'_>, value: &Value) -> TokenStream {
    let Some(reference) = value.as_str() else {
        return invalid_schema_expected_string_keyword_expression("$dynamicRef");
    };

    let fallback = compile_ref(ctx, value);
    let Ok(resolved) = resolve_ref(ctx, reference) else {
        return fallback;
    };

    let Some(anchor_name) = dynamic_ref_anchor_name(reference, &resolved.schema) else {
        return fallback;
    };

    ctx.uses_dynamic_ref = true;
    quote! {
        {
            let __dynamic_target = __JSONSCHEMA_DYNAMIC_STACK.with(|stack| {
                let stack = stack.borrow();
                let mut selected = None;
                for (dynamic_anchor, validate) in stack.iter().rev() {
                    if *dynamic_anchor == #anchor_name {
                        selected = Some(*validate);
                    }
                }
                selected
            });
            if let Some(target) = __dynamic_target {
                target(instance)
            } else {
                #fallback
            }
        }
    }
}

/// Compile a $recursiveRef keyword (draft 2019-09).
pub(super) fn compile_recursive_ref(ctx: &mut CompileContext<'_>, value: &Value) -> TokenStream {
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
        return compile_ref(ctx, value);
    }

    ctx.uses_recursive_ref = true;
    let fallback = compile_ref(ctx, value);
    quote! {
        {
            let __recursive_target = __JSONSCHEMA_RECURSIVE_STACK.with(|stack| {
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
            });
            if let Some(target) = __recursive_target {
                target(instance)
            } else {
                #fallback
            }
        }
    }
}

/// Resolve a reference using the Registry.
pub(in crate::codegen) fn resolve_ref(
    ctx: &mut CompileContext<'_>,
    reference: &str,
) -> Result<ResolvedRef, String> {
    // Use current_base_uri (always set, starts as config.base_uri)
    let base_uri = ctx.current_base_uri.clone();

    let resolver = ctx.config.registry.resolver((*base_uri).clone());
    let resolved = resolver
        .lookup(reference)
        .map_err(|e| format!("Failed to resolve {reference}: {e}"))?;

    // Get the resolved schema's base URI for resolving nested references
    let resolved_base_uri = resolved.resolver().base_uri().clone();

    // Build a canonical location key that preserves the fragment part to avoid
    // conflating references to different targets within the same resource.
    let location_key = if reference.starts_with('#') {
        format!("{base_uri}{reference}")
    } else if let Some((_, fragment)) = reference.rsplit_once('#') {
        if fragment.is_empty() {
            resolved_base_uri.to_string()
        } else {
            format!("{resolved_base_uri}#{fragment}")
        }
    } else {
        resolved_base_uri.to_string()
    };
    let (contents, _, _) = resolved.into_inner();

    Ok(ResolvedRef {
        schema: contents.clone(),
        location: location_key,
        base_uri: resolved_base_uri,
    })
}
