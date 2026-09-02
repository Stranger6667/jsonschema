use super::{
    super::{compile_schema, expr::ValidateBlock, CompileContext, CompiledExpr},
    pattern_coverage::build_pattern_coverage,
};
use crate::codegen::emit::ValueEmitter;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use serde_json::Value;

// The wildcard arm for keys that are not defined properties. It runs only without patternProperties
// (patterns route through `object_pass`), so a key reaching it is simply not a defined property, and
// `additionalProperties: false` is instead rejected by the known-keys precheck.
pub(super) fn compile_wildcard_arm<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    additional_properties: Option<&Value>,
    compiled: Option<&CompiledExpr>,
) -> CompiledExpr {
    match additional_properties {
        None | Some(Value::Bool(true)) => CompiledExpr::always_true(),
        Some(schema) => {
            let fallback;
            let schema_check = if let Some(check) = compiled {
                check
            } else {
                fallback = ctx.with_schema_path_segment("additionalProperties", |ctx| {
                    ctx.with_instance_scope(|ctx| compile_schema(ctx, schema))
                });
                &fallback
            };
            if schema_check.is_trivially_true() {
                CompiledExpr::always_true()
            } else {
                let schema_is_valid = schema_check.is_valid_token_stream();
                // Bind `instance = value` and extend `__path` so the sub-schema validation sees the
                // property value/path (the match arm in `build_validate_block` does not rebind them).
                match &schema_check.validate {
                    ValidateBlock::Expr(expr) => {
                        let collect = schema_check.collect.as_token_stream();
                        CompiledExpr::with_validate_and_collect_blocks(
                            quote! { { #schema_is_valid } },
                            quote! {
                                let instance = value;
                                let __path = &__path.push(key_str);
                                #expr
                            },
                            quote! {
                                let instance = value;
                                let __path = &__path.push(key_str);
                                #collect
                            },
                        )
                    }
                    ValidateBlock::AlwaysValid => CompiledExpr::always_true(),
                }
            }
        }
    }
}

pub(crate) fn compile<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    additional_properties: Option<&Value>,
    pattern_properties: Option<&Value>,
    compiled: Option<&CompiledExpr>,
) -> Option<CompiledExpr> {
    let err_instance = E::err_instance(format_ident!("instance"));
    let additional_properties = additional_properties?;

    // `patternProperties` combined with a false or schema `additionalProperties` is fused into a
    // single instance-order pass by `object_pass`, so this path never covers keys by pattern; the
    // only role of `pattern_properties` here is surfacing an invalid pattern regex.
    if let Err(err) = build_pattern_coverage(ctx, pattern_properties) {
        return Some(err);
    }

    let schema_path = ctx.schema_path_for_keyword("additionalProperties");
    match additional_properties {
        Value::Bool(false) => Some(CompiledExpr::from_bool_expr(
            E::object_is_empty(format_ident!("obj")),
            &schema_path,
            &err_instance,
        )),
        Value::Bool(true) => None,
        schema => {
            let fallback;
            let schema_check = if let Some(check) = compiled {
                check
            } else {
                fallback = ctx.with_schema_path_segment("additionalProperties", |ctx| {
                    ctx.with_instance_scope(|ctx| compile_schema(ctx, schema))
                });
                &fallback
            };
            if schema_check.is_trivially_true() {
                return None;
            }
            let schema_is_valid = schema_check.is_valid_token_stream();
            match &schema_check.validate {
                ValidateBlock::Expr(expr) => {
                    let child_collect = schema_check.collect.as_token_stream();
                    let entries = E::object_iter_entries(format_ident!("obj"));
                    let key_as_str = E::key_as_str(format_ident!("key"));
                    let values_iter = E::object_values_iter(format_ident!("obj"));
                    Some(CompiledExpr::with_validate_and_collect_blocks(
                        quote! { #values_iter.all(|instance| #schema_is_valid) },
                        quote! {
                            for (key, value) in #entries {
                                let instance = value;
                                let __path = &__path.push(#key_as_str);
                                #expr
                            }
                        },
                        quote! {
                            for (key, value) in #entries {
                                let instance = value;
                                let __path = &__path.push(#key_as_str);
                                #child_collect
                            }
                        },
                    ))
                }
                ValidateBlock::AlwaysValid => None,
            }
        }
    }
}

/// Build a `validate` block for `additionalProperties: false`: return an `AdditionalProperties`
/// error for the first key not covered by `properties` (`known_props`), matching the runtime's
/// fail-fast reporting.
pub(super) fn compile_first_unexpected_check<E: ValueEmitter>(
    known_properties: &[&str],
    schema_path: &str,
) -> TokenStream {
    let covered = if known_properties.is_empty() {
        quote! { false }
    } else {
        quote! { matches!(key_str, #(#known_properties)|*) }
    };
    let keys_iter = E::object_keys_iter(format_ident!("obj"));
    let key_as_str = E::key_as_str(format_ident!("key"));
    let err_instance = E::err_instance(format_ident!("instance"));
    let owned_key = E::key_to_owned(format_ident!("key"));
    quote! {
        for key in #keys_iter {
            let key_str = #key_as_str;
            if !(#covered) {
                return Some(__err::additional_properties(
                    #schema_path, __path.into(), #err_instance, vec![#owned_key],
                ));
            }
        }
    }
}
