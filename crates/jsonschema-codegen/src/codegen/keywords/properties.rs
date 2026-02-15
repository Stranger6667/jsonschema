use super::{
    super::{compile_schema, CompileContext, CompiledExpr},
    additional_properties::compile_wildcard_arm,
};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use serde_json::{Map, Value};

fn compile_known_keys_precheck(properties: &Map<String, Value>, schema_path: &str) -> CompiledExpr {
    let known_props: Vec<&str> = properties.keys().map(String::as_str).collect();
    if known_props.is_empty() {
        CompiledExpr::from_bool_expr(quote! { obj.is_empty() }, schema_path)
    } else {
        CompiledExpr::from_bool_expr(
            quote! {
                obj.keys().all(|key| {
                    matches!(key.as_str(), #(#known_props)|*)
                })
            },
            schema_path,
        )
    }
}

/// Compile `properties` together with the `additionalProperties` wildcard arm and
/// required-field tracking.  This integrated form emits a single `obj.iter().all(...)`
/// pass with a match on each key.
pub(crate) fn compile(
    ctx: &mut CompileContext<'_>,
    properties: &Map<String, Value>,
    additional_properties: Option<&Value>,
    pattern_properties: Option<&Value>,
    required_in_props: &[&str],
) -> CompiledExpr {
    let schema_path = ctx.schema_path_for_keyword("properties");
    let ap_schema_path = ctx.schema_path_for_keyword("additionalProperties");
    let use_known_keys_precheck = matches!(additional_properties, Some(Value::Bool(false)))
        && pattern_properties
            .and_then(|value| value.as_object())
            .is_none_or(serde_json::Map::is_empty);

    let (wildcard_statics, wildcard_arm_body) = if use_known_keys_precheck {
        (Vec::new(), CompiledExpr::always_true())
    } else {
        compile_wildcard_arm(ctx, additional_properties, pattern_properties)
    };
    let known_keys_precheck = if use_known_keys_precheck {
        compile_known_keys_precheck(properties, &ap_schema_path)
    } else {
        CompiledExpr::always_true()
    };

    // Compile each property's sub-schema with the correct schema path context.
    let tracked: Vec<(&str, proc_macro2::Ident)> = required_in_props
        .iter()
        .enumerate()
        .map(|(i, &name)| (name, format_ident!("__required_{}", i)))
        .collect();

    let mut compiled_props: Vec<(&str, CompiledExpr)> = Vec::new();
    for (name, subschema) in properties {
        let compiled = ctx.with_schema_path_segment("properties", |ctx| {
            ctx.with_schema_path_segment(name, |ctx| compile_schema(ctx, subschema))
        });
        compiled_props.push((name.as_str(), compiled));
    }

    // ---- is_valid match arms ----
    let mut is_valid_match_arms = Vec::new();
    let mut all_arms_trivially_true = true;
    for (name, compiled) in &compiled_props {
        if !compiled.is_trivially_true() {
            all_arms_trivially_true = false;
        }
        if let Some((_, var)) = tracked.iter().find(|(n, _)| *n == *name) {
            is_valid_match_arms.push(quote! {
                #name => { #var = true; #compiled }
            });
        } else {
            is_valid_match_arms.push(quote! {
                #name => #compiled
            });
        }
    }

    let iter_trivially_true = tracked.is_empty()
        && use_known_keys_precheck
        && all_arms_trivially_true
        && wildcard_arm_body.is_trivially_true();

    let iter_check: CompiledExpr = if iter_trivially_true {
        CompiledExpr::always_true()
    } else {
        let key_as_str = ctx.config.backend.key_as_str(quote! { key });
        let iter_body = quote! {
            let key_str = #key_as_str;
            match key_str {
                #(#is_valid_match_arms,)*
                _ => #wildcard_arm_body
            }
        };
        let is_valid_ts = ctx
            .config
            .backend
            .object_iter_all(quote! { obj }, iter_body);
        CompiledExpr::from_bool_expr(is_valid_ts, &schema_path)
    };

    // ---- validate/iter_errors blocks ----
    let (validate_ts, iter_errors_ts) = if iter_trivially_true {
        (None, None)
    } else {
        let (v, ie) = build_validate_blocks(
            ctx,
            &compiled_props,
            &tracked,
            &wildcard_arm_body,
            &wildcard_statics,
        );
        (Some(v), Some(ie))
    };

    // ---- Combine ----
    let base_iter_check: CompiledExpr = if iter_trivially_true {
        CompiledExpr::always_true()
    } else {
        let (v, ie) = (validate_ts.unwrap(), iter_errors_ts.unwrap());
        let key_as_str = ctx.config.backend.key_as_str(quote! { key });
        let iter_body = quote! {
            let key_str = #key_as_str;
            match key_str {
                #(#is_valid_match_arms,)*
                _ => #wildcard_arm_body
            }
        };
        let is_valid_ts_main = ctx
            .config
            .backend
            .object_iter_all(quote! { obj }, iter_body);
        let base_is_valid = if use_known_keys_precheck {
            let kp_ts = known_keys_precheck.is_valid_ts();
            quote! {
                {
                    #(#wildcard_statics)*
                    #kp_ts && #is_valid_ts_main
                }
            }
        } else {
            quote! {
                {
                    #(#wildcard_statics)*
                    #is_valid_ts_main
                }
            }
        };
        if tracked.is_empty() {
            CompiledExpr::with_validate_blocks(base_is_valid, v, ie)
        } else {
            let var_decls = tracked
                .iter()
                .map(|(_, var)| quote! { let mut #var = false; });
            let var_checks = tracked.iter().map(|(_, var)| quote! { #var });
            CompiledExpr::with_validate_blocks(
                quote! {
                    {
                        #(#wildcard_statics)*
                        #(#var_decls)*
                        let __iter_result = #is_valid_ts_main;
                        __iter_result && #(#var_checks)&&*
                    }
                },
                v,
                ie,
            )
        }
    };

    if tracked.is_empty() {
        if known_keys_precheck.is_trivially_true() && base_iter_check.is_trivially_true() {
            CompiledExpr::always_true()
        } else if known_keys_precheck.is_trivially_true() {
            base_iter_check
        } else if base_iter_check.is_trivially_true() {
            let kp_ts = known_keys_precheck.is_valid_ts();
            CompiledExpr::from_bool_expr(
                quote! { { #(#wildcard_statics)* #kp_ts } },
                &ap_schema_path,
            )
        } else {
            let kp_ts = known_keys_precheck.is_valid_ts();
            let ic_ts = base_iter_check.is_valid_ts();
            let v = base_iter_check.validate.as_ts();
            let ie = base_iter_check.iter_errors.as_ts();
            CompiledExpr::with_validate_blocks(
                quote! { { #(#wildcard_statics)* #kp_ts && #ic_ts } },
                quote! {
                    if let Some(obj) = instance.as_object() {
                        if !(#kp_ts) {
                            return Some(jsonschema::keywords_helpers::error::false_schema(
                                #ap_schema_path, __path.clone(), instance,
                            ));
                        }
                    }
                    #v
                },
                quote! {
                    if let Some(obj) = instance.as_object() {
                        if !(#kp_ts) {
                            __errors.push(jsonschema::keywords_helpers::error::false_schema(
                                #ap_schema_path, __path.clone(), instance,
                            ));
                            return;
                        }
                    }
                    #ie
                },
            )
        }
    } else {
        let var_decls = tracked
            .iter()
            .map(|(_, var)| quote! { let mut #var = false; });
        let var_checks = tracked.iter().map(|(_, var)| quote! { #var });
        let iter_check_ts = iter_check.is_valid_ts();
        if known_keys_precheck.is_trivially_true() {
            let v = base_iter_check.validate.as_ts();
            let ie = base_iter_check.iter_errors.as_ts();
            CompiledExpr::with_validate_blocks(
                quote! {
                    {
                        #(#wildcard_statics)*
                        #(#var_decls)*
                        #iter_check_ts && #(#var_checks)&&*
                    }
                },
                v,
                ie,
            )
        } else {
            let kp_ts = known_keys_precheck.is_valid_ts();
            // known_keys_precheck validates that no extra keys exist (additionalProperties: false).
            // We must emit that check in validate/iter_errors BEFORE the required-field check so
            // that additionalProperties errors are reported before missing-required errors, matching
            // the dynamic validator's error ordering.
            let kp_v = known_keys_precheck.validate.as_ts();
            let kp_ie = known_keys_precheck.iter_errors.as_ts();
            let v = base_iter_check.validate.as_ts();
            let ie = base_iter_check.iter_errors.as_ts();
            CompiledExpr::with_validate_blocks(
                quote! {
                    {
                        #(#wildcard_statics)*
                        #(#var_decls)*
                        #kp_ts && #iter_check_ts && #(#var_checks)&&*
                    }
                },
                quote! { #kp_v #v },
                quote! { #kp_ie #ie },
            )
        }
    }
}

/// Build `validate` and `iter_errors` blocks for the properties for-loop.
#[allow(clippy::too_many_arguments)]
fn build_validate_blocks(
    ctx: &CompileContext<'_>,
    compiled_props: &[(&str, CompiledExpr)],
    tracked: &[(&str, proc_macro2::Ident)],
    wildcard_arm_body: &CompiledExpr,
    wildcard_statics: &[TokenStream],
) -> (TokenStream, TokenStream) {
    let key_as_str = ctx.config.backend.key_as_str(quote! { key });

    let mut validate_arms = Vec::new();
    let mut iter_errors_arms = Vec::new();

    for (name, compiled) in compiled_props {
        let v = compiled.validate.as_ts();
        let ie = compiled.iter_errors.as_ts();

        // Track required if needed
        let track_required = tracked
            .iter()
            .find(|(n, _)| *n == *name)
            .map(|(_, var)| quote! { #var = true; });

        validate_arms.push(quote! {
            #name => {
                #track_required
                let instance = value;
                let __path = __path.join(#name);
                #v
            }
        });
        iter_errors_arms.push(quote! {
            #name => {
                #track_required
                let instance = value;
                let __path = __path.join(#name);
                #ie
            }
        });
    }

    // Wildcard arm
    let wildcard_v = wildcard_arm_body.validate.as_ts();
    let wildcard_ie = wildcard_arm_body.iter_errors.as_ts();

    let wildcard_validate_arm = quote! {
        _ => { #wildcard_v }
    };
    let wildcard_iter_errors_arm = quote! {
        _ => { #wildcard_ie }
    };

    // Required var declarations and post-loop checks
    let var_decls: Vec<_> = tracked
        .iter()
        .map(|(_, var)| quote! { let mut #var = false; })
        .collect();

    let required_schema_path = ctx.schema_path_for_keyword("required");
    let required_validate_checks: Vec<_> = tracked
        .iter()
        .map(|(name, var)| {
            let sp = required_schema_path.as_str();
            quote! {
                if !#var {
                    return Some(jsonschema::keywords_helpers::error::required(
                        #sp, __path.clone(), instance, #name,
                    ));
                }
            }
        })
        .collect();
    let required_ie_checks: Vec<_> = tracked
        .iter()
        .map(|(name, var)| {
            let sp = required_schema_path.as_str();
            quote! {
                if !#var {
                    __errors.push(jsonschema::keywords_helpers::error::required(
                        #sp, __path.clone(), instance, #name,
                    ));
                }
            }
        })
        .collect();

    let validate_ts = quote! {
        #(#wildcard_statics)*
        #(#var_decls)*
        if let Some(obj) = instance.as_object() {
            for (key, value) in obj.iter() {
                let key_str = #key_as_str;
                match key_str {
                    #(#validate_arms,)*
                    #wildcard_validate_arm
                }
            }
        }
        #(#required_validate_checks)*
    };

    let iter_errors_ts = quote! {
        #(#wildcard_statics)*
        #(#var_decls)*
        if let Some(obj) = instance.as_object() {
            for (key, value) in obj.iter() {
                let key_str = #key_as_str;
                match key_str {
                    #(#iter_errors_arms,)*
                    #wildcard_iter_errors_arm
                }
            }
        }
        #(#required_ie_checks)*
    };

    (validate_ts, iter_errors_ts)
}
