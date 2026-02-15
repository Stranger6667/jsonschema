use proc_macro2::TokenStream;
use quote::quote;
use serde_json::{Map, Value};

use crate::context::CompileContext;

use super::{
    expr::{IsValidExpr, ValidateBlock},
    keywords,
    keywords::format::format_emits_assertion,
    supports_applicator_vocabulary, supports_contains_keyword,
    supports_content_validation_keywords, supports_dependent_required_keyword,
    supports_dependent_schemas_keyword, supports_prefix_items_keyword,
    supports_property_names_keyword, supports_unevaluated_items_keyword_for_context,
    supports_unevaluated_properties_keyword_for_context, supports_validation_vocabulary,
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
    let mut is_valid_arms = Vec::new();
    let mut validate_arms = Vec::new();
    let mut iter_errors_arms = Vec::new();
    let integer_guard = backend.integer_number_guard(ctx.draft);
    // When `type: integer` with numeric keywords (e.g. multipleOf), a Number that fails the
    // integer guard (e.g. 1e+308 in Draft4 where only i64/u64 qualify) should still have its
    // numeric constraints checked in iter_errors mode — the dynamic validator runs each keyword
    // validator independently. We defer building the iter_errors Number arm until we have the
    // type_error_expr available, then emit: run numeric checks for ALL numbers, then add type
    // error for non-integers.
    let mut integer_ie_override: Option<(TokenStream, TokenStream)> = None;

    if has_string {
        let for_string = keywords::string::compile(ctx, schema);
        is_valid_arms.push(backend.match_string_arm(for_string.is_valid_ts()));
        let v = for_string.validate.as_ts();
        let ie = for_string.iter_errors.as_ts();
        if has_type_constraint || matches!(&for_string.validate, ValidateBlock::Expr(_)) {
            validate_arms.push(backend.match_string_arm(v));
            iter_errors_arms.push(backend.match_string_arm(ie));
        }
    }

    if has_number {
        let for_number = keywords::number::compile(ctx, schema);
        if has_type_constraint {
            if let Some(type_val) = schema.get("type") {
                let allows_number = type_keyword_includes(type_val, "number");
                let allows_integer = type_keyword_includes(type_val, "integer");
                if allows_number {
                    is_valid_arms.push(backend.match_number_arm(for_number.is_valid_ts()));
                    let v = for_number.validate.as_ts();
                    let ie = for_number.iter_errors.as_ts();
                    validate_arms.push(backend.match_number_arm(v));
                    iter_errors_arms.push(backend.match_number_arm(ie));
                } else if allows_integer {
                    is_valid_arms.push(
                        backend.match_integer_arm(integer_guard.clone(), for_number.is_valid_ts()),
                    );
                    let v = for_number.validate.as_ts();
                    let ie = for_number.iter_errors.as_ts();
                    validate_arms.push(backend.match_integer_arm(integer_guard.clone(), v));
                    // Defer iter_errors arm: we need type_error_expr (built later)
                    // to emit both numeric and type errors for numbers that fail the
                    // integer guard (e.g. 1e+308 in Draft4).
                    integer_ie_override = Some((ie, integer_guard.clone()));
                }
            }
        } else {
            is_valid_arms.push(backend.match_number_arm(for_number.is_valid_ts()));
            let v = for_number.validate.as_ts();
            let ie = for_number.iter_errors.as_ts();
            if matches!(&for_number.validate, ValidateBlock::Expr(_)) {
                validate_arms.push(backend.match_number_arm(v));
                iter_errors_arms.push(backend.match_number_arm(ie));
            }
        }
    }

    if has_array {
        let for_array = keywords::array::compile(ctx, schema);
        is_valid_arms.push(backend.match_array_arm(for_array.is_valid_ts()));
        let v = for_array.validate.as_ts();
        let ie = for_array.iter_errors.as_ts();
        if has_type_constraint || matches!(&for_array.validate, ValidateBlock::Expr(_)) {
            validate_arms.push(backend.match_array_arm(v));
            iter_errors_arms.push(backend.match_array_arm(ie));
        }
    }

    if has_object {
        let for_object = keywords::object::compile(ctx, schema);
        is_valid_arms.push(backend.match_object_arm(for_object.is_valid_ts()));
        let v = for_object.validate.as_ts();
        let ie = for_object.iter_errors.as_ts();
        if has_type_constraint || matches!(&for_object.validate, ValidateBlock::Expr(_)) {
            validate_arms.push(backend.match_object_arm(v));
            iter_errors_arms.push(backend.match_object_arm(ie));
        }
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
                is_valid_arms.push(quote! { #(#additional_types)|* => true });
            }
        }
        is_valid_arms.push(quote! { _ => false });

        let type_schema_path = ctx.schema_path_for_keyword("type");
        let type_error_expr = build_type_error_expr(
            schema.get("type").unwrap(),
            &type_schema_path,
            &integer_guard,
        );
        // Build the deferred integer iter_errors arm now that we have type_error_expr.
        // For numbers that fail the integer guard (e.g. 1e+308 in Draft4), we run the
        // numeric checks anyway and then emit a type error — matching the dynamic validator
        // which evaluates each keyword independently in iter_errors mode.
        if let Some((ie, guard)) = integer_ie_override {
            iter_errors_arms.push(quote! {
                serde_json::Value::Number(n) => {
                    // Emit type error first (matching dynamic validator order), then numeric
                    // checks for any number value regardless of the integer guard.
                    if !(#guard) {
                        __errors.push(#type_error_expr);
                    }
                    #ie
                }
            });
        }
        validate_arms.push(quote! { _ => { return Some(#type_error_expr); } });
        iter_errors_arms.push(quote! { _ => { __errors.push(#type_error_expr); } });
    } else {
        is_valid_arms.push(quote! { _ => true });
        validate_arms.push(quote! { _ => {} });
        iter_errors_arms.push(quote! { _ => {} });
    }

    let is_valid_ts = quote! {
        match instance {
            #(#is_valid_arms),*
        }
    };
    let validate_ts = quote! { match instance { #(#validate_arms),* } };
    let iter_errors_ts = quote! { match instance { #(#iter_errors_arms),* } };
    Some(CompiledExpr {
        is_valid: IsValidExpr::Expr(is_valid_ts),
        validate: ValidateBlock::Expr(validate_ts),
        iter_errors: ValidateBlock::Expr(iter_errors_ts),
    })
}

fn build_type_error_expr(
    type_val: &Value,
    type_schema_path: &str,
    _integer_guard: &TokenStream,
) -> TokenStream {
    match type_val {
        Value::String(ty) => {
            let json_type = type_name_to_json_type_token(ty.as_str());
            quote! {
                jsonschema::keywords_helpers::error::single_type(
                    #type_schema_path, __path.clone(), instance, #json_type,
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
                jsonschema::keywords_helpers::error::multiple_types(
                    #type_schema_path, __path.clone(), instance, #chain,
                )
            }
        }
        _ => {
            quote! {
                jsonschema::keywords_helpers::error::false_schema(
                    #type_schema_path, __path.clone(), instance,
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
