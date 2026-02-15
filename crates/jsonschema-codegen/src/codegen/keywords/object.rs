use super::super::{
    compile_additional_properties, compile_dependencies, compile_dependent_required,
    compile_dependent_schemas, compile_known_keys_precheck, compile_pattern_properties,
    compile_property_names, compile_schema, compile_unevaluated_properties, compile_wildcard_arm,
    invalid_schema_type_expression, is_trivially_true, parse_nonnegative_integer_keyword,
    supports_applicator_vocabulary, supports_dependent_required_keyword,
    supports_dependent_schemas_keyword, supports_property_names_keyword,
    supports_unevaluated_properties_keyword_for_context, supports_validation_vocabulary,
    CompileContext,
};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use serde_json::{Map, Value};

/// Compile all object-specific keywords.
pub(in super::super) fn compile(
    ctx: &mut CompileContext<'_>,
    schema: &Map<String, Value>,
) -> TokenStream {
    let mut items = Vec::new();
    let validation_vocab_enabled = supports_validation_vocabulary(ctx);
    let applicator_vocab_enabled = supports_applicator_vocabulary(ctx);
    let unevaluated_properties_enabled = supports_unevaluated_properties_keyword_for_context(ctx);

    let min_props = if validation_vocab_enabled {
        match schema.get("minProperties") {
            Some(value) => match parse_nonnegative_integer_keyword(ctx.draft, value) {
                Ok(parsed) => Some(parsed),
                Err(error) => {
                    items.push(error);
                    None
                }
            },
            None => None,
        }
    } else {
        None
    };
    let max_props = if validation_vocab_enabled {
        match schema.get("maxProperties") {
            Some(value) => match parse_nonnegative_integer_keyword(ctx.draft, value) {
                Ok(parsed) => Some(parsed),
                Err(error) => {
                    items.push(error);
                    None
                }
            },
            None => None,
        }
    } else {
        None
    };
    let object_len = ctx.config.backend.object_len(quote! { obj });
    match (min_props, max_props) {
        (Some(min), Some(max)) if min == max => {
            items.push(quote! { #object_len == #min as usize });
        }
        (Some(min), Some(max)) => {
            items.push(quote! { #object_len >= #min as usize });
            items.push(quote! { #object_len <= #max as usize });
        }
        (Some(min), None) => items.push(quote! { #object_len >= #min as usize }),
        (None, Some(max)) => items.push(quote! { #object_len <= #max as usize }),
        (None, None) => {}
    }

    let required_fields: Vec<&str> = if validation_vocab_enabled {
        match schema.get("required") {
            None => Vec::new(),
            Some(Value::Array(arr)) => {
                let mut required = Vec::with_capacity(arr.len());
                for item in arr {
                    if let Some(name) = item.as_str() {
                        required.push(name);
                    } else {
                        items.push(invalid_schema_type_expression(item, &["string"]));
                        break;
                    }
                }
                required
            }
            Some(other) => {
                items.push(invalid_schema_type_expression(other, &["array"]));
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    // Properties validation via iteration (will be handled at the end)
    let properties_map = if applicator_vocab_enabled {
        match schema.get("properties") {
            Some(Value::Object(map)) => Some(map),
            Some(other) => {
                items.push(invalid_schema_type_expression(other, &["object"]));
                None
            }
            None => None,
        }
    } else {
        None
    };

    // Partition required fields: those also in `properties` are tracked via bool
    // during iteration; those not in `properties` still need contains_key.
    let (required_in_props, required_only): (Vec<&str>, Vec<&str>) =
        if let Some(props) = properties_map {
            required_fields
                .iter()
                .copied()
                .partition(|name| props.contains_key(*name))
        } else {
            (Vec::new(), required_fields.clone())
        };

    for name in &required_only {
        items.push(ctx.config.backend.object_contains_key(quote! { obj }, name));
    }

    if applicator_vocab_enabled {
        if let Some(compiled) = schema
            .get("patternProperties")
            .and_then(|v| compile_pattern_properties(ctx, v))
        {
            items.push(compiled);
        }
    }

    if applicator_vocab_enabled {
        if let Some(compiled) = schema
            .get("dependencies")
            .and_then(|v| compile_dependencies(ctx, v))
        {
            items.push(compiled);
        }
    }

    // Draft 2019-09+: dependentRequired
    if validation_vocab_enabled && supports_dependent_required_keyword(ctx.draft) {
        if let Some(compiled) = schema
            .get("dependentRequired")
            .and_then(compile_dependent_required)
        {
            items.push(compiled);
        }
    }

    if applicator_vocab_enabled && supports_dependent_schemas_keyword(ctx.draft) {
        if let Some(compiled) = schema
            .get("dependentSchemas")
            .and_then(|v| compile_dependent_schemas(ctx, v))
        {
            items.push(compiled);
        }
    }

    // Draft 6+: propertyNames - all property names must validate against the schema
    if applicator_vocab_enabled && supports_property_names_keyword(ctx.draft) {
        if let Some(compiled) = schema
            .get("propertyNames")
            .map(|v| compile_property_names(ctx, v))
        {
            items.push(compiled);
        }
    }
    let ap = if applicator_vocab_enabled {
        schema.get("additionalProperties")
    } else {
        None
    };
    let pp = if applicator_vocab_enabled {
        schema.get("patternProperties")
    } else {
        None
    };

    if let Some(properties) = properties_map {
        // For strict objects without patternProperties, a key-only precheck can fail fast
        // before potentially expensive per-value validation.
        let use_known_keys_precheck = matches!(ap, Some(Value::Bool(false)))
            && pp
                .and_then(|value| value.as_object())
                .is_none_or(serde_json::Map::is_empty);

        // Merge additionalProperties coverage into the single properties iteration.
        // The wildcard arm body replaces `_ => true`, eliminating a separate key-coverage pass.
        let (wildcard_statics, wildcard_arm_body) = if use_known_keys_precheck {
            // Coverage is guaranteed by the precheck, so wildcard keys are unreachable.
            (Vec::new(), quote! { true })
        } else {
            compile_wildcard_arm(ctx, ap, pp)
        };
        let known_keys_precheck = if use_known_keys_precheck {
            compile_known_keys_precheck(properties)
        } else {
            quote! { true }
        };

        let mut match_arms = Vec::new();

        // Assign a unique bool variable per required-in-props field
        let tracked: Vec<(&str, proc_macro2::Ident)> = required_in_props
            .iter()
            .enumerate()
            .map(|(i, &name)| (name, format_ident!("__required_{}", i)))
            .collect();

        let mut all_arms_trivially_true = true;
        for (name, subschema) in properties {
            let compiled = compile_schema(ctx, subschema);
            if !is_trivially_true(&compiled) {
                all_arms_trivially_true = false;
            }
            if let Some((_, var)) = tracked.iter().find(|(n, _)| *n == name.as_str()) {
                match_arms.push(quote! {
                    #name => { #var = true; #compiled }
                });
            } else {
                match_arms.push(quote! {
                    #name => #compiled
                });
            }
        }

        // If the precheck covers key-set AND all property schemas + wildcard are trivially
        // true, skip the per-value iteration entirely.
        let iter_trivially_true = tracked.is_empty()
            && use_known_keys_precheck
            && all_arms_trivially_true
            && is_trivially_true(&wildcard_arm_body);

        let iter_check = if iter_trivially_true {
            quote! { true }
        } else {
            let key_as_str = ctx.config.backend.key_as_str(quote! { key });
            let iter_body = quote! {
                let key_str = #key_as_str;
                match key_str {
                    #(#match_arms,)*
                    _ => #wildcard_arm_body
                }
            };
            ctx.config
                .backend
                .object_iter_all(quote! { obj }, iter_body)
        };
        let properties_check = if tracked.is_empty() {
            if is_trivially_true(&known_keys_precheck) && is_trivially_true(&iter_check) {
                quote! { true }
            } else if is_trivially_true(&known_keys_precheck) {
                quote! {
                    {
                        #(#wildcard_statics)*
                        #iter_check
                    }
                }
            } else if is_trivially_true(&iter_check) {
                quote! {
                    {
                        #(#wildcard_statics)*
                        #known_keys_precheck
                    }
                }
            } else {
                quote! {
                    {
                        #(#wildcard_statics)*
                        #known_keys_precheck && #iter_check
                    }
                }
            }
        } else {
            let var_decls = tracked
                .iter()
                .map(|(_, var)| quote! { let mut #var = false; });
            let var_checks = tracked.iter().map(|(_, var)| quote! { #var });
            if is_trivially_true(&known_keys_precheck) {
                quote! {
                    {
                        #(#wildcard_statics)*
                        #(#var_decls)*
                        #iter_check && #(#var_checks)&&*
                    }
                }
            } else {
                quote! {
                    {
                        #(#wildcard_statics)*
                        #(#var_decls)*
                        #known_keys_precheck && #iter_check && #(#var_checks)&&*
                    }
                }
            }
        };
        items.push(properties_check);
    } else if applicator_vocab_enabled {
        // No properties: use the standalone additionalProperties check when present.
        // Note: we do NOT push a `true` placeholder here, which avoids a spurious `&& true`.
        if let Some(compiled) = compile_additional_properties(ctx, ap, schema.get("properties"), pp)
        {
            items.push(compiled);
        }
    }

    if unevaluated_properties_enabled {
        if let Some(compiled) = compile_unevaluated_properties(ctx, schema) {
            items.push(compiled);
        }
    }

    if items.is_empty() {
        quote! { true }
    } else {
        quote! { ( #(#items)&&* ) }
    }
}
