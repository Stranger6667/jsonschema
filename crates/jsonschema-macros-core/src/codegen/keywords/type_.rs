use crate::codegen::emit::ValueEmitter;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use serde_json::Value;

use super::super::{
    dispatch::build_type_error_expr,
    errors::{
        invalid_schema_non_empty_array_expression, invalid_schema_type_expression,
        invalid_schema_unexpected_type_expression,
    },
    CompileContext, CompiledExpr,
};

/// Match pattern for a type whose check is a bare serde variant pattern.
/// `None` for "number"/"integer", whose checks carry an integer sub-guard.
fn simple_type_pattern<E: ValueEmitter>(ty: &str) -> Option<TokenStream> {
    Some(match ty {
        "string" => E::pattern_string(),
        "boolean" => E::pattern_boolean(),
        "null" => E::pattern_null(),
        "array" => E::pattern_array(),
        "object" => E::pattern_object(),
        _ => return None,
    })
}

fn wrap_type_check(is_valid: &TokenStream, error_expr: &TokenStream) -> CompiledExpr {
    CompiledExpr::from_check_and_error(is_valid.clone(), error_expr.clone())
}

pub(crate) fn compile<E: ValueEmitter>(ctx: &CompileContext<'_, E>, value: &Value) -> CompiledExpr {
    fn is_known_type_name(name: &str) -> bool {
        matches!(
            name,
            "string" | "number" | "integer" | "boolean" | "null" | "array" | "object"
        )
    }

    let schema_path = ctx.schema_path_for_keyword("type");

    match value {
        Value::String(ty) => {
            if is_known_type_name(ty) {
                generate_type_check(ctx, ty.as_str(), &schema_path)
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
            if let &[type_name] = type_names.as_slice() {
                return generate_type_check(ctx, type_name, &schema_path);
            }

            let has_integer = type_names.contains(&"integer");
            let has_number = type_names.contains(&"number");

            if has_integer || has_number {
                // Bare (non-block) arms, so each carries its own trailing comma: `type_match`
                // concatenates arms with no separator.
                let number_arm = if has_number {
                    let pattern = E::pattern_number();
                    quote! { #pattern => true, }
                } else {
                    let pattern = E::pattern_number_binding();
                    let integer_check = E::integer_number_guard(ctx.draft);
                    quote! { #pattern => #integer_check, }
                };
                let mut arms = vec![number_arm];
                for &ty in &type_names {
                    if let Some(pattern) = simple_type_pattern::<E>(ty) {
                        arms.push(quote! { #pattern => true, });
                    }
                }
                arms.push(quote! { _ => false, });
                wrap_type_check(
                    &E::type_match(format_ident!("instance"), arms),
                    &build_type_error_expr::<E>(value, &schema_path),
                )
            } else {
                let patterns: Vec<TokenStream> = type_names
                    .iter()
                    .filter_map(|&ty| simple_type_pattern::<E>(ty))
                    .collect();
                wrap_type_check(
                    &E::type_matches(patterns),
                    &build_type_error_expr::<E>(value, &schema_path),
                )
            }
        }
        _ => invalid_schema_type_expression(value, &["string", "array"]),
    }
}

fn generate_type_check<E: ValueEmitter>(
    ctx: &CompileContext<'_, E>,
    value: &str,
    schema_path: &str,
) -> CompiledExpr {
    let is_valid = match value {
        "string" => E::instance_is_string(),
        "number" => E::instance_is_number(),
        "integer" => E::instance_is_integer(ctx.draft),
        "boolean" => E::instance_is_boolean(),
        "null" => E::instance_is_null(),
        "array" => E::instance_is_array(),
        "object" => E::instance_is_object(),
        _ => return invalid_schema_unexpected_type_expression(),
    };
    let error_expr = build_type_error_expr::<E>(&Value::String(value.to_string()), schema_path);
    wrap_type_check(&is_valid, &error_expr)
}
