use super::{
    super::{CompileContext, CompiledExpr},
    additional_properties::compile_wildcard_arm,
    object_pass::ClusterSubschemas,
    required,
};
use crate::codegen::emit::ValueEmitter;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use serde_json::{Map, Value};

/// `is_valid` for an object whose keys must all be declared `properties`.
fn known_keys_is_valid<E: ValueEmitter>(properties: &Map<String, Value>) -> TokenStream {
    let known_props: Vec<&str> = properties.keys().map(String::as_str).collect();
    if known_props.is_empty() {
        E::object_is_empty(format_ident!("obj"))
    } else {
        let keys_iter = E::object_keys_iter(format_ident!("obj"));
        let key_as_str = E::key_as_str(format_ident!("key"));
        quote! {
            #keys_iter.all(|key| {
                matches!(#key_as_str, #(#known_props)|*)
            })
        }
    }
}

/// Compile `properties` together with the `additionalProperties` wildcard arm. This integrated
/// form emits a single `obj.iter().all(...)` pass with a match on each key.
///
/// `required` names are checked ahead of the property values, as the runtime does.
pub(crate) fn compile<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    properties: &Map<String, Value>,
    additional_properties: Option<&Value>,
    cluster: &ClusterSubschemas<'_>,
    required: &[&str],
) -> CompiledExpr {
    let additional_properties_path = ctx.schema_path_for_keyword("additionalProperties");
    let use_known_keys_precheck = matches!(additional_properties, Some(Value::Bool(false)));

    let wildcard_arm_body = if use_known_keys_precheck {
        CompiledExpr::always_true()
    } else {
        compile_wildcard_arm(ctx, additional_properties, cluster.additional.as_ref())
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

    let errors = || {
        build_errors_block::<E>(
            compiled_props,
            use_known_keys_precheck,
            &wildcard_arm_body,
            &additional_properties_path,
        )
    };
    let compiled = if !iter_trivially_true {
        let key_as_str = E::key_as_str(format_ident!("key"));
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
        let main_is_valid = E::object_iter_all(format_ident!("obj"), iter_body);
        CompiledExpr::with_errors(quote! { { #main_is_valid } }, errors())
    } else if use_known_keys_precheck {
        // Trivial property values leave no covered-value errors to interleave with, so the
        // known-keys check alone decides validity.
        let known_keys_is_valid = known_keys_is_valid::<E>(properties);
        CompiledExpr::with_errors(quote! { { #known_keys_is_valid } }, errors())
    } else {
        CompiledExpr::always_true()
    };
    if required.is_empty() {
        return compiled;
    }
    let required_checks: Vec<CompiledExpr> = required
        .iter()
        .map(|name| required::compile_single(ctx, name))
        .collect();
    CompiledExpr::combine_and(required_checks.into_iter().chain([compiled]))
}

/// Build the errors block. It iterates the instance's keys, like `is_valid`, with
/// `additionalProperties: false` aggregating unexpected keys into one trailing error.
fn build_errors_block<E: ValueEmitter>(
    compiled_props: &[(&str, CompiledExpr)],
    use_known_keys_precheck: bool,
    wildcard_arm_body: &CompiledExpr,
    additional_properties_path: &str,
) -> TokenStream {
    let key_as_str = E::key_as_str(format_ident!("key"));
    let entries = E::object_iter_entries(format_ident!("obj"));
    let match_arms = compiled_props.iter().map(|(name, compiled)| {
        if compiled.is_trivially_true() {
            return quote! { #name => {} };
        }
        let child = compiled.errors.as_token_stream();
        let errors = compiled.guard_errors(&quote! {
            let __path = &__path.push(#name);
            #child
        });
        quote! {
            #name => {
                let instance = value;
                #errors
            }
        }
    });
    if use_known_keys_precheck {
        let err_instance = E::err_instance(format_ident!("instance"));
        let owned_key = E::key_to_owned(format_ident!("key"));
        quote! {
            let mut __unexpected: Vec<String> = Vec::new();
            for (key, value) in #entries {
                let key_str = #key_as_str;
                match key_str {
                    #(#match_arms,)*
                    _ => {
                        // `validate` reports the first unexpected key at once, like the runtime.
                        if __errors.first_only() {
                            return __errors.push(__err::additional_properties(
                                #additional_properties_path, __path.into(), #err_instance, vec![#owned_key],
                            ));
                        }
                        __unexpected.push(#owned_key);
                    }
                }
            }
            if !__unexpected.is_empty() {
                if __errors.push(__err::additional_properties(
                    #additional_properties_path, __path.into(), #err_instance, __unexpected,
                )) { return true; }
            }
        }
    } else {
        let wildcard_errors = wildcard_arm_body.errors.as_token_stream();
        quote! {
            for (key, value) in #entries {
                let key_str = #key_as_str;
                match key_str {
                    #(#match_arms,)*
                    _ => { #wildcard_errors }
                }
            }
        }
    }
}
