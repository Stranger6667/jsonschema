use proc_macro2::TokenStream;
use quote::quote;
use serde_json::{Map, Value};

use crate::context::CompileContext;

use super::{
    errors::{
        invalid_schema_non_empty_array_expression, invalid_schema_type_expression,
        invalid_schema_unexpected_type_expression,
    },
    format_emits_assertion, keywords, supports_applicator_vocabulary, supports_contains_keyword,
    supports_content_validation_keywords, supports_dependent_required_keyword,
    supports_dependent_schemas_keyword, supports_prefix_items_keyword,
    supports_property_names_keyword, supports_unevaluated_items_keyword_for_context,
    supports_unevaluated_properties_keyword_for_context, supports_validation_vocabulary,
};

fn type_keyword_includes(type_value: &Value, name: &str) -> bool {
    match type_value {
        Value::String(s) => s == name,
        Value::Array(arr) => arr.iter().any(|v| v.as_str() == Some(name)),
        _ => false,
    }
}

pub(super) fn compile_typed(
    ctx: &mut CompileContext<'_>,
    schema: &Map<String, Value>,
    has_type_constraint: bool,
) -> Option<TokenStream> {
    let validation_vocab_enabled = supports_validation_vocabulary(ctx);
    let applicator_vocab_enabled = supports_applicator_vocabulary(ctx);
    let unevaluated_items_enabled = supports_unevaluated_items_keyword_for_context(ctx);
    let unevaluated_properties_enabled = supports_unevaluated_properties_keyword_for_context(ctx);

    let has_string = (validation_vocab_enabled
        && (schema.contains_key("minLength")
            || schema.contains_key("maxLength")
            || schema.contains_key("pattern")))
        || schema
            .get("format")
            .is_some_and(|value| format_emits_assertion(ctx, value))
        || (supports_content_validation_keywords(ctx.draft)
            && (schema.contains_key("contentEncoding") || schema.contains_key("contentMediaType")));

    let has_number = validation_vocab_enabled
        && (schema.contains_key("minimum")
            || schema.contains_key("maximum")
            || schema.contains_key("exclusiveMinimum")
            || schema.contains_key("exclusiveMaximum")
            || schema.contains_key("multipleOf"));

    let has_array = (validation_vocab_enabled
        && (schema.contains_key("minItems")
            || schema.contains_key("maxItems")
            || schema.contains_key("uniqueItems")))
        || (applicator_vocab_enabled
            && (schema.contains_key("items")
                || schema.contains_key("additionalItems")
                || (supports_contains_keyword(ctx.draft) && schema.contains_key("contains"))
                || (supports_prefix_items_keyword(ctx.draft)
                    && schema.contains_key("prefixItems"))))
        || (unevaluated_items_enabled && schema.contains_key("unevaluatedItems"));

    let has_object = (validation_vocab_enabled
        && (schema.contains_key("required")
            || schema.contains_key("minProperties")
            || schema.contains_key("maxProperties")
            || (supports_dependent_required_keyword(ctx.draft)
                && schema.contains_key("dependentRequired"))))
        || (applicator_vocab_enabled
            && (schema.contains_key("properties")
                || schema.contains_key("patternProperties")
                || schema.contains_key("additionalProperties")
                || schema.contains_key("dependencies")
                || (supports_dependent_schemas_keyword(ctx.draft)
                    && schema.contains_key("dependentSchemas"))
                || (supports_property_names_keyword(ctx.draft)
                    && schema.contains_key("propertyNames"))))
        || (unevaluated_properties_enabled && schema.contains_key("unevaluatedProperties"));

    if !has_string && !has_number && !has_array && !has_object {
        return None;
    }

    let backend = &ctx.config.backend;
    let mut match_arms = Vec::new();
    let integer_guard = backend.integer_number_guard(ctx.draft);

    if has_string {
        let for_string = keywords::string::compile(ctx, schema);
        match_arms.push(backend.match_string_arm(for_string));
    }

    if has_number {
        let for_number = keywords::number::compile(ctx, schema);
        if has_type_constraint {
            if let Some(type_val) = schema.get("type") {
                let allows_number = type_keyword_includes(type_val, "number");
                let allows_integer = type_keyword_includes(type_val, "integer");
                if allows_number {
                    match_arms.push(backend.match_number_arm(for_number));
                } else if allows_integer {
                    match_arms.push(backend.match_integer_arm(integer_guard.clone(), for_number));
                }
            }
        } else {
            match_arms.push(backend.match_number_arm(for_number));
        }
    }

    if has_array {
        let for_array = keywords::array::compile(ctx, schema);
        match_arms.push(backend.match_array_arm(for_array));
    }

    if has_object {
        let for_object = keywords::object::compile(ctx, schema);
        match_arms.push(backend.match_object_arm(for_object));
    }

    if has_type_constraint {
        if let Some(type_val) = schema.get("type") {
            let mut additional_types = Vec::new();

            let has_string_fallback = type_keyword_includes(type_val, "string") && !has_string;
            let has_number_fallback = type_keyword_includes(type_val, "number") && !has_number;
            let has_integer_fallback =
                type_keyword_includes(type_val, "integer") && !has_number && !has_number_fallback;
            let has_array_fallback = type_keyword_includes(type_val, "array") && !has_array;
            let has_object_fallback = type_keyword_includes(type_val, "object") && !has_object;
            let has_boolean_fallback = type_keyword_includes(type_val, "boolean");
            let has_null_fallback = type_keyword_includes(type_val, "null");

            if has_string_fallback {
                additional_types.push(backend.pattern_string());
            }
            if has_number_fallback {
                additional_types.push(backend.pattern_number());
            }
            if has_integer_fallback {
                additional_types.push(backend.pattern_integer(integer_guard.clone()));
            }
            if has_array_fallback {
                additional_types.push(backend.pattern_array());
            }
            if has_object_fallback {
                additional_types.push(backend.pattern_object());
            }
            if has_boolean_fallback {
                additional_types.push(backend.pattern_boolean());
            }
            if has_null_fallback {
                additional_types.push(backend.pattern_null());
            }

            if !additional_types.is_empty() {
                match_arms.push(quote! { #(#additional_types)|* => true });
            }
        }
        match_arms.push(quote! { _ => false });
    } else {
        match_arms.push(quote! { _ => true });
    }

    Some(quote! {
        match instance {
            #(#match_arms),*
        }
    })
}

pub(super) fn compile_type(ctx: &CompileContext<'_>, value: &Value) -> TokenStream {
    fn is_known_type_name(name: &str) -> bool {
        matches!(
            name,
            "string" | "number" | "integer" | "boolean" | "null" | "array" | "object"
        )
    }

    let backend = &ctx.config.backend;

    match value {
        Value::String(ty) => {
            if is_known_type_name(ty) {
                generate_type_check(ctx, ty.as_str())
            } else {
                invalid_schema_unexpected_type_expression()
            }
        }
        Value::Array(types) => {
            let mut type_names = Vec::with_capacity(types.len());
            for item in types {
                let Some(type_name) = item.as_str() else {
                    return invalid_schema_type_expression(item, &["string"]);
                };
                if !is_known_type_name(type_name) {
                    return invalid_schema_unexpected_type_expression();
                }
                type_names.push(type_name);
            }
            if type_names.is_empty() {
                return invalid_schema_non_empty_array_expression();
            }
            if type_names.len() == 1 {
                return generate_type_check(ctx, type_names[0]);
            }

            let has_integer = type_names.contains(&"integer");
            let has_number = type_names.contains(&"number");

            if has_integer || has_number {
                let number_arm = if has_number {
                    let pattern = backend.pattern_number();
                    quote! { #pattern => true }
                } else {
                    let pattern = backend.pattern_number_binding();
                    let int_check = backend.integer_number_guard(ctx.draft);
                    quote! { #pattern => #int_check }
                };
                let mut arms = vec![number_arm];
                for ty in &type_names {
                    let arm = match *ty {
                        "string" => {
                            let pattern = backend.pattern_string();
                            quote! { #pattern => true }
                        }
                        "boolean" => {
                            let pattern = backend.pattern_boolean();
                            quote! { #pattern => true }
                        }
                        "null" => {
                            let pattern = backend.pattern_null();
                            quote! { #pattern => true }
                        }
                        "array" => {
                            let pattern = backend.pattern_array();
                            quote! { #pattern => true }
                        }
                        "object" => {
                            let pattern = backend.pattern_object();
                            quote! { #pattern => true }
                        }
                        _ => continue,
                    };
                    arms.push(arm);
                }
                arms.push(quote! { _ => false });
                quote! { match instance { #(#arms),* } }
            } else {
                let patterns: Vec<TokenStream> = type_names
                    .iter()
                    .filter_map(|ty| match *ty {
                        "string" => Some(backend.pattern_string()),
                        "boolean" => Some(backend.pattern_boolean()),
                        "null" => Some(backend.pattern_null()),
                        "array" => Some(backend.pattern_array()),
                        "object" => Some(backend.pattern_object()),
                        _ => None,
                    })
                    .collect();
                quote! { matches!(instance, #(#patterns)|*) }
            }
        }
        _ => invalid_schema_type_expression(value, &["string", "array"]),
    }
}

fn generate_type_check(ctx: &CompileContext<'_>, value: &str) -> TokenStream {
    let backend = &ctx.config.backend;

    match value {
        "string" => backend.instance_is_string(),
        "number" => backend.instance_is_number(),
        "integer" => backend.instance_is_integer(ctx.draft),
        "boolean" => backend.instance_is_boolean(),
        "null" => backend.instance_is_null(),
        "array" => backend.instance_is_array(),
        "object" => backend.instance_is_object(),
        _ => invalid_schema_unexpected_type_expression(),
    }
}
