use crate::codegen::emit::ValueEmitter;
use proc_macro2::TokenStream;
use quote::{format_ident, quote, ToTokens};
use serde_json::{Map, Value};

use crate::context::CompileContext;

use super::{
    draft::DraftExt,
    expr::{call_errors, ErrorsBlock, IsValidExpr, RefErrors},
    keywords,
    keywords::format::format_emits_assertion,
    CompiledExpr,
};

enum SoleType {
    /// `is_valid`, errors, and the referenced schema checking every item when that is the only
    /// error source.
    Array(TokenStream, TokenStream, Option<(String, &'static str)>),
    Object(TokenStream, TokenStream),
}

fn type_keyword_includes(type_value: &Value, name: &str) -> bool {
    match type_value {
        Value::String(s) => s == name,
        Value::Array(arr) => arr.iter().any(|v| v.as_str() == Some(name)),
        _ => false,
    }
}

/// The checks of keywords for one instance type. Without `report_type_error`, they leave the
/// `type` error to the caller; with `split_unevaluated`, the `unevaluated*` errors go to a second
/// type dispatch.
pub(super) fn compile_typed<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    schema: &Map<String, Value>,
    has_type_constraint: bool,
    report_type_error: bool,
    split_unevaluated: bool,
) -> Option<(CompiledExpr, Option<CompiledExpr>)> {
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

    let mut is_valid_fallback = quote! { true };
    let mut type_schema_path = None;
    let mut typed_arms = 0usize;
    let mut sole: Option<SoleType> = None;
    let mut is_valid_arms = Vec::new();
    let mut errors_arms = Vec::new();
    let mut skips_valid = true;
    let integer_guard = E::integer_number_guard(ctx.draft);
    let type_value = schema.get("type").filter(|_| has_type_constraint);
    let type_error = type_value.map(|type_value| {
        if report_type_error {
            let type_error =
                build_type_error_expr::<E>(type_value, &ctx.schema_path_for_keyword("type"));
            quote! { if __errors.push(#type_error) { return true; } }
        } else {
            TokenStream::new()
        }
    });
    let mut unevaluated_array = None;
    let mut unevaluated_object = None;
    let mut unevaluated_skips_valid = true;
    // Whether any arm reports an error, which may be none once `unevaluated*` errors are split out.
    let mut reports_errors = false;
    // The type error, when `type` leaves out every one of `names`: such instances fail, and the
    // runtime reports the type error before the errors of their keywords.
    let excluded = |names: &[&str]| {
        type_error.clone().filter(|_| {
            type_value.is_some_and(|type_value| {
                !names
                    .iter()
                    .any(|name| type_keyword_includes(type_value, name))
            })
        })
    };

    if has_string {
        let for_string = keywords::string::compile(ctx, schema);
        skips_valid &= for_string.skips_valid;
        reports_errors |= matches!(for_string.errors, ErrorsBlock::Expr(_));
        if let Some(type_error) = excluded(&["string"]) {
            typed_arms += 1;
            let errors = for_string.errors.as_token_stream();
            is_valid_arms.push(E::match_string_arm(quote! { false }));
            errors_arms.push(E::match_string_arm(quote! { #type_error #errors }));
        } else if has_type_constraint || !for_string.is_trivially_true() {
            typed_arms += 1;
            is_valid_arms.push(E::match_string_arm(for_string.is_valid_token_stream()));
            errors_arms.push(E::match_string_arm(for_string.errors.as_token_stream()));
        }
    }

    if has_number {
        let for_number = keywords::number::compile(ctx, schema);
        skips_valid &= for_number.skips_valid;
        reports_errors |= matches!(for_number.errors, ErrorsBlock::Expr(_));
        if has_type_constraint {
            let type_val = schema
                .get("type")
                .expect("has_type_constraint implies a `type` keyword");
            let allows_number = type_keyword_includes(type_val, "number");
            let allows_integer = type_keyword_includes(type_val, "integer");
            if allows_number {
                typed_arms += 1;
                is_valid_arms.push(E::match_number_arm(for_number.is_valid_token_stream()));
                errors_arms.push(E::match_number_arm(for_number.errors.as_token_stream()));
            } else if allows_integer {
                typed_arms += 1;
                is_valid_arms.push(E::match_integer_arm(
                    integer_guard.clone(),
                    for_number.is_valid_token_stream(),
                ));
                // Runtime reports both the type error and the numeric errors for a non-integer number
                // under `type: integer`, so the numeric checks run for every number and the type
                // error is reported inline when it is not an integer.
                let numeric_errors = for_number.errors.as_token_stream();
                if report_type_error {
                    let type_schema_path = ctx.schema_path_for_keyword("type");
                    let type_error_expr = build_type_error_expr::<E>(type_val, &type_schema_path);
                    let guard = integer_guard.clone();
                    errors_arms.push(E::match_number_arm(quote! {
                        if !(#guard) {
                            if __errors.push(#type_error_expr) { return true; }
                        }
                        #numeric_errors
                    }));
                } else {
                    errors_arms.push(E::match_number_arm(numeric_errors));
                }
            } else if for_number.is_compile_error() {
                typed_arms += 1;
                is_valid_arms.push(E::match_number_arm(for_number.is_valid_token_stream()));
                errors_arms.push(E::match_number_arm(for_number.errors.as_token_stream()));
            } else {
                typed_arms += 1;
                let errors = for_number.errors.as_token_stream();
                is_valid_arms.push(E::match_number_arm(quote! { false }));
                errors_arms.push(E::match_number_arm(quote! { #type_error #errors }));
            }
        } else if !for_number.is_trivially_true() {
            typed_arms += 1;
            is_valid_arms.push(E::match_number_arm(for_number.is_valid_token_stream()));
            errors_arms.push(E::match_number_arm(for_number.errors.as_token_stream()));
        }
    }

    if has_array {
        let (mut for_array, unevaluated) = keywords::array::compile(ctx, schema, split_unevaluated);
        skips_valid &= for_array.skips_valid;
        reports_errors |= matches!(for_array.errors, ErrorsBlock::Expr(_));
        if let Some(unevaluated) = unevaluated {
            unevaluated_skips_valid &= unevaluated.skips_valid;
            unevaluated_array = Some(unevaluated.errors.as_token_stream());
        }
        if let Some(type_error) = excluded(&["array"]) {
            typed_arms += 1;
            let errors = for_array.errors.as_token_stream();
            is_valid_arms.push(E::match_array_arm(quote! { false }));
            errors_arms.push(E::match_array_arm(quote! { #type_error #errors }));
        } else if has_type_constraint || !for_array.is_trivially_true() {
            typed_arms += 1;
            sole = Some(SoleType::Array(
                for_array.is_valid_token_stream(),
                for_array.errors.as_token_stream(),
                match for_array.ref_errors.take() {
                    Some(RefErrors::EachItem(name, keyword)) => Some((name, keyword)),
                    _ => None,
                },
            ));
            is_valid_arms.push(E::match_array_arm(for_array.is_valid_token_stream()));
            if has_type_constraint || matches!(&for_array.errors, ErrorsBlock::Expr(_)) {
                errors_arms.push(E::match_array_arm(for_array.errors.as_token_stream()));
            }
        }
    }

    if has_object {
        let (for_object, unevaluated) = keywords::object::compile(ctx, schema, split_unevaluated);
        skips_valid &= for_object.skips_valid;
        reports_errors |= matches!(for_object.errors, ErrorsBlock::Expr(_));
        if let Some(unevaluated) = unevaluated {
            unevaluated_skips_valid &= unevaluated.skips_valid;
            unevaluated_object = Some(unevaluated.errors.as_token_stream());
        }
        if let Some(type_error) = excluded(&["object"]) {
            typed_arms += 1;
            let errors = for_object.errors.as_token_stream();
            is_valid_arms.push(E::match_object_arm(quote! { false }));
            errors_arms.push(E::match_object_arm(quote! { #type_error #errors }));
        } else if has_type_constraint || !for_object.is_trivially_true() {
            typed_arms += 1;
            sole = Some(SoleType::Object(
                for_object.is_valid_token_stream(),
                for_object.errors.as_token_stream(),
            ));
            is_valid_arms.push(E::match_object_arm(for_object.is_valid_token_stream()));
            if has_type_constraint || matches!(&for_object.errors, ErrorsBlock::Expr(_)) {
                errors_arms.push(E::match_object_arm(for_object.errors.as_token_stream()));
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
            additional_types.push(E::pattern_string());
        }
        if has_number_fallback {
            additional_types.push(E::pattern_number());
        }
        if has_integer_fallback {
            additional_types.push(E::pattern_integer(integer_guard.clone()));
        }
        if has_array_fallback {
            additional_types.push(E::pattern_array());
        }
        if has_object_fallback {
            additional_types.push(E::pattern_object());
        }
        if has_boolean_fallback {
            additional_types.push(E::pattern_boolean());
        }
        if has_null_fallback {
            additional_types.push(E::pattern_null());
        }

        if !additional_types.is_empty() {
            typed_arms += 1;
            // Bare (non-block) arm, so it carries its own trailing comma: `type_match`
            // concatenates arms with no separator, and `_ => false` always follows it.
            is_valid_arms.push(quote! { #(#additional_types)|* => true, });
            for pattern in &additional_types {
                errors_arms.push(quote! { #pattern => {} });
            }
        }
        is_valid_fallback = quote! { false };
        type_schema_path = report_type_error.then(|| ctx.schema_path_for_keyword("type"));
    } else if is_valid_arms.is_empty() {
        return None;
    }
    let errors_fallback = || {
        if let (Some(type_value), Some(type_schema_path)) = (schema.get("type"), &type_schema_path)
        {
            let type_error_expr = build_type_error_expr::<E>(type_value, type_schema_path);
            quote! { { if __errors.push(#type_error_expr) { return true; } } }
        } else {
            quote! { {} }
        }
    };

    let (is_valid_ts, errors_ts) = match sole.filter(|_| typed_arms == 1) {
        Some(SoleType::Array(is_valid, errors, each_item_ref)) => {
            let errors = match (schema.get("type"), each_item_ref, &type_schema_path) {
                // Arrays of one referenced schema differ only in their type error's location, so
                // they report through one helper taking that location.
                (
                    Some(type_value @ Value::String(ty)),
                    Some((ref_name, keyword)),
                    Some(type_schema_path),
                ) if ty == "array" => {
                    let ref_path = format!("{}/{keyword}", ctx.schema_path_for_keyword("items"));
                    let helper = ctx.array_errors_helper(ref_name.clone(), || {
                        let type_error =
                            build_type_error_expr::<E>(type_value, format_ident!("__type_path"));
                        let errors_ident = format_ident!("{}_errors", ref_name);
                        let call = quote! { #errors_ident(instance, __path, __errors, __ref_path) };
                        let iter_expr = E::array_iter(format_ident!("arr"));
                        E::sole_array_match(
                            quote! {
                                for (idx, item) in #iter_expr.enumerate() {
                                    let instance = item;
                                    let __path = &__path.push(idx);
                                    if #call {
                                        return true;
                                    }
                                }
                            },
                            quote! { { if __errors.push(#type_error) { return true; } } },
                        )
                    });
                    call_errors(&quote! {
                        #helper(instance, __path, __errors, #type_schema_path, #ref_path)
                    })
                }
                _ => E::sole_array_match(errors, errors_fallback()),
            };
            (E::sole_array_match(is_valid, is_valid_fallback), errors)
        }
        Some(SoleType::Object(is_valid, errors)) => (
            E::sole_object_match(is_valid, is_valid_fallback),
            E::sole_object_match(errors, errors_fallback()),
        ),
        _ => {
            let errors_fallback = errors_fallback();
            is_valid_arms.push(quote! { _ => #is_valid_fallback });
            errors_arms.push(quote! { _ => #errors_fallback });
            (
                E::type_match(format_ident!("instance"), is_valid_arms),
                E::type_match(format_ident!("instance"), errors_arms),
            )
        }
    };
    let unevaluated_errors = match (unevaluated_array, unevaluated_object) {
        (None, None) => None,
        (Some(array), None) => Some(E::sole_array_match(array, quote! { {} })),
        (None, Some(object)) => Some(E::sole_object_match(object, quote! { {} })),
        (Some(array), Some(object)) => Some(E::type_match(
            format_ident!("instance"),
            vec![
                E::match_array_arm(array),
                E::match_object_arm(object),
                quote! { _ => {} },
            ],
        )),
    };
    let unevaluated = unevaluated_errors.map(|errors| CompiledExpr {
        is_valid: IsValidExpr::AlwaysTrue,
        errors: ErrorsBlock::Expr(errors),
        compile_error: false,
        skips_valid: unevaluated_skips_valid,
        ref_errors: None,
    });
    Some((
        CompiledExpr {
            is_valid: IsValidExpr::Expr(is_valid_ts),
            errors: if reports_errors || type_schema_path.is_some() {
                ErrorsBlock::Expr(errors_ts)
            } else {
                ErrorsBlock::AlwaysValid
            },
            compile_error: false,
            skips_valid,
            ref_errors: None,
        },
        unevaluated,
    ))
}

pub(super) fn build_type_error_expr<E: ValueEmitter>(
    type_val: &Value,
    type_schema_path: impl ToTokens,
) -> TokenStream {
    let err_instance = E::err_instance(format_ident!("instance"));
    match type_val {
        Value::String(ty) => {
            let json_type = type_name_to_json_type_token(ty.as_str());
            quote! {
                __err::single_type(
                    #type_schema_path, __path.into(), #err_instance, #json_type,
                )
            }
        }
        Value::Array(types) => {
            // Build the set as a const chain: JsonTypeSet::empty().insert(A).insert(B)...
            let chain = types.iter().filter_map(|v| v.as_str()).fold(
                quote! { __JTS::empty() },
                |acc, ty| {
                    let json_type = type_name_to_json_type_token(ty);
                    quote! { #acc.insert(#json_type) }
                },
            );
            quote! {
                __err::multiple_types(
                    #type_schema_path, __path.into(), #err_instance, #chain,
                )
            }
        }
        _ => {
            quote! {
                __err::false_schema(
                    #type_schema_path, __path.into(), #err_instance,
                )
            }
        }
    }
}

fn type_name_to_json_type_token(name: &str) -> TokenStream {
    match name {
        "number" => quote! { __JT::Number },
        "integer" => quote! { __JT::Integer },
        "boolean" => quote! { __JT::Boolean },
        "null" => quote! { __JT::Null },
        "array" => quote! { __JT::Array },
        "object" => quote! { __JT::Object },
        _ => quote! { __JT::String },
    }
}
