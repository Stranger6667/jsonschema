use super::{
    super::{CompileContext, CompiledExpr},
    additional_properties::{compile_first_unexpected_check, compile_wildcard_arm},
    object_pass::ClusterSubschemas,
};
use proc_macro2::TokenStream;
use quote::quote;
use serde_json::{Map, Value};

fn compile_known_keys_precheck(properties: &Map<String, Value>, schema_path: &str) -> CompiledExpr {
    let known_props: Vec<&str> = properties.keys().map(String::as_str).collect();
    let is_valid = if known_props.is_empty() {
        quote! { obj.is_empty() }
    } else {
        quote! {
            obj.keys().all(|key| {
                matches!(key.as_str(), #(#known_props)|*)
            })
        }
    };
    let validate = compile_first_unexpected_check(&known_props, schema_path);
    CompiledExpr::with_validate_blocks(is_valid, validate)
}

/// Compile `properties` together with the `additionalProperties` wildcard arm. This integrated
/// form emits a single `obj.iter().all(...)` pass with a match on each key.
pub(crate) fn compile(
    ctx: &mut CompileContext<'_>,
    properties: &Map<String, Value>,
    additional_properties: Option<&Value>,
    cluster: &ClusterSubschemas<'_>,
) -> CompiledExpr {
    let additional_properties_path = ctx.schema_path_for_keyword("additionalProperties");
    let use_known_keys_precheck = matches!(additional_properties, Some(Value::Bool(false)));

    let wildcard_arm_body = if use_known_keys_precheck {
        CompiledExpr::always_true()
    } else {
        compile_wildcard_arm(ctx, additional_properties, cluster.additional.as_ref())
    };
    let known_keys_precheck = if use_known_keys_precheck {
        compile_known_keys_precheck(properties, &additional_properties_path)
    } else {
        CompiledExpr::always_true()
    };

    let compiled_props = &cluster.properties;

    let mut is_valid_match_arms = Vec::new();
    let mut all_arms_trivially_true = true;
    for (name, compiled) in compiled_props {
        if !compiled.is_trivially_true() {
            all_arms_trivially_true = false;
        }
        is_valid_match_arms.push(quote! {
            #name => #compiled
        });
    }

    let iter_trivially_true = all_arms_trivially_true && wildcard_arm_body.is_trivially_true();

    // The single `obj.iter().all(...)` pass reused by `base_iter_check` below.
    let main_iter_is_valid: Option<TokenStream> = if iter_trivially_true {
        None
    } else {
        let key_as_str = crate::codegen::emit_serde::key_as_str(quote! { key });
        // With `additionalProperties: false` and no patternProperties, the wildcard arm
        // rejects unknown keys in this single pass, so `is_valid` needs no separate
        // key-membership scan.
        let is_valid_wildcard = if use_known_keys_precheck {
            quote! { false }
        } else {
            wildcard_arm_body.is_valid_token_stream()
        };
        let iter_body = quote! {
            let key_str = #key_as_str;
            match key_str {
                #(#is_valid_match_arms,)*
                _ => #is_valid_wildcard
            }
        };
        Some(crate::codegen::emit_serde::object_iter_all(
            quote! { obj },
            iter_body,
        ))
    };

    let base_iter_check: CompiledExpr = if let Some(main_is_valid) = main_iter_is_valid {
        // With `additionalProperties: false` and no patternProperties, the wildcard arm reports the
        // first unexpected key inline, so its error interleaves with covered-value errors in one pass.
        let wildcard_validate = if use_known_keys_precheck {
            quote! {
                return Some(jsonschema::__private::error::additional_properties(
                    #additional_properties_path, __path.into(), instance, vec![key.clone()],
                ));
            }
        } else {
            wildcard_arm_body.validate.as_token_stream()
        };
        let validate = build_validate_block(compiled_props, &wildcard_validate);
        let base_is_valid = quote! {
            {
                #main_is_valid
            }
        };
        CompiledExpr::with_validate_blocks(base_is_valid, validate)
    } else {
        CompiledExpr::always_true()
    };

    if base_iter_check.is_trivially_true() {
        if known_keys_precheck.is_trivially_true() {
            CompiledExpr::always_true()
        } else {
            // Trivial property values leave no covered-value errors to interleave with, so the
            // known-keys precheck alone reports the first unexpected key.
            let known_keys_is_valid = known_keys_precheck.is_valid_token_stream();
            let precheck_validate = known_keys_precheck.validate.as_token_stream();
            CompiledExpr::with_validate_blocks(
                quote! { { #known_keys_is_valid } },
                precheck_validate,
            )
        }
    } else {
        base_iter_check
    }
}

/// Build the `validate` block for the properties for-loop.
fn build_validate_block(
    compiled_props: &[(&str, CompiledExpr)],
    wildcard_validate: &TokenStream,
) -> TokenStream {
    let key_as_str = crate::codegen::emit_serde::key_as_str(quote! { key });

    let mut validate_arms = Vec::new();
    for (name, compiled) in compiled_props {
        let validate = compiled.validate.as_token_stream();
        validate_arms.push(quote! {
            #name => {
                let instance = value;
                let __path = &__path.push(#name);
                #validate
            }
        });
    }

    let wildcard_validate_arm = quote! {
        _ => { #wildcard_validate }
    };

    quote! {
        for (key, value) in obj.iter() {
            let key_str = #key_as_str;
            match key_str {
                #(#validate_arms,)*
                #wildcard_validate_arm
            }
        }
    }
}
