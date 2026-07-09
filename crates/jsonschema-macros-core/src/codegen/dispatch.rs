use proc_macro2::TokenStream;
use quote::quote;
use serde_json::{Map, Value};

use crate::context::CompileContext;

use super::{
    draft::DraftExt,
    expr::{CollectBlock, IsValidExpr, ValidateBlock},
    keywords,
    keywords::format::format_emits_assertion,
    CompiledExpr,
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
) -> Option<CompiledExpr> {
    let validation_vocab_enabled = ctx.supports_validation_vocabulary();
    let applicator_vocab_enabled = ctx.supports_applicator_vocabulary();
    let unevaluated_items_enabled = ctx.supports_unevaluated_items();
    let unevaluated_properties_enabled = ctx.supports_unevaluated_properties();

    let has_string = (validation_vocab_enabled
        && (schema.contains_key("minLength")
            || schema.contains_key("maxLength")
            || schema.contains_key("pattern")))
        || schema
            .get("format")
            .is_some_and(|value| format_emits_assertion(ctx, value))
        || (ctx.draft.supports_content_validation_keywords()
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
                || (ctx.draft.supports_contains_keyword() && schema.contains_key("contains"))
                || (ctx.draft.supports_prefix_items_keyword()
                    && schema.contains_key("prefixItems"))))
        || (unevaluated_items_enabled && schema.contains_key("unevaluatedItems"));

    let has_object = (validation_vocab_enabled
        && (schema.contains_key("required")
            || schema.contains_key("minProperties")
            || schema.contains_key("maxProperties")
            || (ctx.draft.supports_dependent_required_keyword()
                && schema.contains_key("dependentRequired"))))
        || (applicator_vocab_enabled
            && (schema.contains_key("properties")
                || schema.contains_key("patternProperties")
                || schema.contains_key("additionalProperties")
                || schema.contains_key("dependencies")
                || (ctx.draft.supports_dependent_schemas_keyword()
                    && schema.contains_key("dependentSchemas"))
                || (ctx.draft.supports_property_names_keyword()
                    && schema.contains_key("propertyNames"))))
        || (unevaluated_properties_enabled && schema.contains_key("unevaluatedProperties"));

    if !has_string && !has_number && !has_array && !has_object {
        return None;
    }

    let mut is_valid_arms = Vec::new();
    let mut validate_arms = Vec::new();
    let mut collect_arms = Vec::new();
    let integer_guard = crate::codegen::emit_serde::integer_number_guard(ctx.draft);

    if has_string {
        let for_string = keywords::string::compile(ctx, schema);
        if has_type_constraint || !for_string.is_trivially_true() {
            is_valid_arms.push(crate::codegen::emit_serde::match_string_arm(
                for_string.is_valid_token_stream(),
            ));
            if has_type_constraint || matches!(&for_string.validate, ValidateBlock::Expr(_)) {
                validate_arms.push(crate::codegen::emit_serde::match_string_arm(
                    for_string.validate.as_token_stream(),
                ));
                collect_arms.push(crate::codegen::emit_serde::match_string_arm(
                    for_string.collect.as_token_stream(),
                ));
            }
        }
    }

    if has_number {
        let for_number = keywords::number::compile(ctx, schema);
        if has_type_constraint {
            let type_val = schema
                .get("type")
                .expect("has_type_constraint implies a `type` keyword");
            let allows_number = type_keyword_includes(type_val, "number");
            let allows_integer = type_keyword_includes(type_val, "integer");
            if allows_number {
                is_valid_arms.push(crate::codegen::emit_serde::match_number_arm(
                    for_number.is_valid_token_stream(),
                ));
                validate_arms.push(crate::codegen::emit_serde::match_number_arm(
                    for_number.validate.as_token_stream(),
                ));
                collect_arms.push(crate::codegen::emit_serde::match_number_arm(
                    for_number.collect.as_token_stream(),
                ));
            } else if allows_integer {
                is_valid_arms.push(crate::codegen::emit_serde::match_integer_arm(
                    integer_guard.clone(),
                    for_number.is_valid_token_stream(),
                ));
                validate_arms.push(crate::codegen::emit_serde::match_integer_arm(
                    integer_guard.clone(),
                    for_number.validate.as_token_stream(),
                ));
                // Runtime reports both the type error and the numeric errors for a non-integer number
                // under `type: integer`, so collect runs the numeric checks for every number and pushes
                // the type error inline when it is not an integer.
                let type_schema_path = ctx.schema_path_for_keyword("type");
                let type_error_expr = build_type_error_expr(type_val, &type_schema_path);
                let guard = integer_guard.clone();
                let numeric_collect = for_number.collect.as_token_stream();
                collect_arms.push(crate::codegen::emit_serde::match_number_arm(quote! {
                    if !(#guard) {
                        __errors.push(#type_error_expr);
                    }
                    #numeric_collect
                }));
            } else if for_number.is_compile_error() {
                is_valid_arms.push(crate::codegen::emit_serde::match_number_arm(
                    for_number.is_valid_token_stream(),
                ));
                validate_arms.push(crate::codegen::emit_serde::match_number_arm(
                    for_number.validate.as_token_stream(),
                ));
                collect_arms.push(crate::codegen::emit_serde::match_number_arm(
                    for_number.collect.as_token_stream(),
                ));
            }
        } else if !for_number.is_trivially_true() {
            is_valid_arms.push(crate::codegen::emit_serde::match_number_arm(
                for_number.is_valid_token_stream(),
            ));
            if matches!(&for_number.validate, ValidateBlock::Expr(_)) {
                validate_arms.push(crate::codegen::emit_serde::match_number_arm(
                    for_number.validate.as_token_stream(),
                ));
                collect_arms.push(crate::codegen::emit_serde::match_number_arm(
                    for_number.collect.as_token_stream(),
                ));
            }
        }
    }

    if has_array {
        let for_array = keywords::array::compile(ctx, schema);
        if has_type_constraint || !for_array.is_trivially_true() {
            is_valid_arms.push(crate::codegen::emit_serde::match_array_arm(
                for_array.is_valid_token_stream(),
            ));
            if has_type_constraint || matches!(&for_array.validate, ValidateBlock::Expr(_)) {
                validate_arms.push(crate::codegen::emit_serde::match_array_arm(
                    for_array.validate.as_token_stream(),
                ));
                collect_arms.push(crate::codegen::emit_serde::match_array_arm(
                    for_array.collect.as_token_stream(),
                ));
            }
        }
    }

    if has_object {
        let for_object = keywords::object::compile(ctx, schema);
        if has_type_constraint || !for_object.is_trivially_true() {
            is_valid_arms.push(crate::codegen::emit_serde::match_object_arm(
                for_object.is_valid_token_stream(),
            ));
            if has_type_constraint || matches!(&for_object.validate, ValidateBlock::Expr(_)) {
                validate_arms.push(crate::codegen::emit_serde::match_object_arm(
                    for_object.validate.as_token_stream(),
                ));
                collect_arms.push(crate::codegen::emit_serde::match_object_arm(
                    for_object.collect.as_token_stream(),
                ));
            }
        }
    }

    if has_type_constraint {
        let type_val = schema
            .get("type")
            .expect("has_type_constraint implies a `type` keyword");
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
            additional_types.push(crate::codegen::emit_serde::pattern_string());
        }
        if has_number_fallback {
            additional_types.push(crate::codegen::emit_serde::pattern_number());
        }
        if has_integer_fallback {
            additional_types.push(crate::codegen::emit_serde::pattern_integer(
                integer_guard.clone(),
            ));
        }
        if has_array_fallback {
            additional_types.push(crate::codegen::emit_serde::pattern_array());
        }
        if has_object_fallback {
            additional_types.push(crate::codegen::emit_serde::pattern_object());
        }
        if has_boolean_fallback {
            additional_types.push(crate::codegen::emit_serde::pattern_boolean());
        }
        if has_null_fallback {
            additional_types.push(crate::codegen::emit_serde::pattern_null());
        }

        if !additional_types.is_empty() {
            is_valid_arms.push(quote! { #(#additional_types)|* => true });
            for pattern in &additional_types {
                validate_arms.push(quote! { #pattern => {} });
                collect_arms.push(quote! { #pattern => {} });
            }
        }
        is_valid_arms.push(quote! { _ => false });

        let type_schema_path = ctx.schema_path_for_keyword("type");
        let type_value = schema
            .get("type")
            .expect("has_type_constraint implies a `type` keyword");
        let type_error_expr = build_type_error_expr(type_value, &type_schema_path);
        validate_arms.push(quote! { _ => { return Some(#type_error_expr); } });
        collect_arms.push(quote! { _ => { __errors.push(#type_error_expr); } });
    } else {
        if is_valid_arms.is_empty() {
            return None;
        }
        is_valid_arms.push(quote! { _ => true });
        validate_arms.push(quote! { _ => {} });
        collect_arms.push(quote! { _ => {} });
    }

    let is_valid_ts = quote! {
        match instance {
            #(#is_valid_arms),*
        }
    };
    let validate_ts = quote! { match instance { #(#validate_arms),* } };
    let collect_ts = quote! { match instance { #(#collect_arms),* } };
    Some(CompiledExpr {
        is_valid: IsValidExpr::Expr(is_valid_ts),
        validate: ValidateBlock::Expr(validate_ts),
        collect: CollectBlock::Expr(collect_ts),
        compile_error: false,
    })
}

pub(super) fn build_type_error_expr(type_val: &Value, type_schema_path: &str) -> TokenStream {
    match type_val {
        Value::String(ty) => {
            let json_type = type_name_to_json_type_token(ty.as_str());
            quote! {
                jsonschema::__private::error::single_type(
                    #type_schema_path, __path.into(), instance, #json_type,
                )
            }
        }
        Value::Array(types) => {
            // Build the set as a const chain: JsonTypeSet::empty().insert(A).insert(B)...
            let chain = types.iter().filter_map(|v| v.as_str()).fold(
                quote! { jsonschema::JsonTypeSet::empty() },
                |acc, ty| {
                    let json_type = type_name_to_json_type_token(ty);
                    quote! { #acc.insert(#json_type) }
                },
            );
            quote! {
                jsonschema::__private::error::multiple_types(
                    #type_schema_path, __path.into(), instance, #chain,
                )
            }
        }
        _ => {
            quote! {
                jsonschema::__private::error::false_schema(
                    #type_schema_path, __path.into(), instance,
                )
            }
        }
    }
}

fn type_name_to_json_type_token(name: &str) -> TokenStream {
    match name {
        "number" => quote! { jsonschema::JsonType::Number },
        "integer" => quote! { jsonschema::JsonType::Integer },
        "boolean" => quote! { jsonschema::JsonType::Boolean },
        "null" => quote! { jsonschema::JsonType::Null },
        "array" => quote! { jsonschema::JsonType::Array },
        "object" => quote! { jsonschema::JsonType::Object },
        _ => quote! { jsonschema::JsonType::String },
    }
}
