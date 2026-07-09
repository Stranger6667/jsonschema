use proc_macro2::TokenStream;
use quote::quote;
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
fn simple_type_pattern(ty: &str) -> Option<TokenStream> {
    Some(match ty {
        "string" => crate::codegen::emit_serde::pattern_string(),
        "boolean" => crate::codegen::emit_serde::pattern_boolean(),
        "null" => crate::codegen::emit_serde::pattern_null(),
        "array" => crate::codegen::emit_serde::pattern_array(),
        "object" => crate::codegen::emit_serde::pattern_object(),
        _ => return None,
    })
}

fn wrap_type_check(is_valid: &TokenStream, error_expr: &TokenStream) -> CompiledExpr {
    CompiledExpr::from_check_and_error(is_valid.clone(), error_expr.clone())
}

pub(crate) fn compile(ctx: &CompileContext<'_>, value: &Value) -> CompiledExpr {
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
                let number_arm = if has_number {
                    let pattern = crate::codegen::emit_serde::pattern_number();
                    quote! { #pattern => true }
                } else {
                    let pattern = crate::codegen::emit_serde::pattern_number_binding();
                    let integer_check = crate::codegen::emit_serde::integer_number_guard(ctx.draft);
                    quote! { #pattern => #integer_check }
                };
                let mut arms = vec![number_arm];
                for &ty in &type_names {
                    if let Some(pattern) = simple_type_pattern(ty) {
                        arms.push(quote! { #pattern => true });
                    }
                }
                arms.push(quote! { _ => false });
                wrap_type_check(
                    &quote! { match instance { #(#arms),* } },
                    &build_type_error_expr(value, &schema_path),
                )
            } else {
                let patterns: Vec<TokenStream> = type_names
                    .iter()
                    .filter_map(|&ty| simple_type_pattern(ty))
                    .collect();
                wrap_type_check(
                    &quote! { matches!(instance, #(#patterns)|*) },
                    &build_type_error_expr(value, &schema_path),
                )
            }
        }
        _ => invalid_schema_type_expression(value, &["string", "array"]),
    }
}

fn generate_type_check(ctx: &CompileContext<'_>, value: &str, schema_path: &str) -> CompiledExpr {
    let is_valid = match value {
        "string" => crate::codegen::emit_serde::instance_is_string(),
        "number" => crate::codegen::emit_serde::instance_is_number(),
        "integer" => crate::codegen::emit_serde::instance_is_integer(ctx.draft),
        "boolean" => crate::codegen::emit_serde::instance_is_boolean(),
        "null" => crate::codegen::emit_serde::instance_is_null(),
        "array" => crate::codegen::emit_serde::instance_is_array(),
        "object" => crate::codegen::emit_serde::instance_is_object(),
        _ => return invalid_schema_unexpected_type_expression(),
    };
    let error_expr = build_type_error_expr(&Value::String(value.to_string()), schema_path);
    wrap_type_check(&is_valid, &error_expr)
}
