use proc_macro2::TokenStream;
use quote::quote;
use serde_json::Value;

use super::super::{
    errors::{
        invalid_schema_non_empty_array_expression, invalid_schema_type_expression,
        invalid_schema_unexpected_type_expression,
    },
    CompileContext, CompiledExpr,
};

pub(crate) fn compile(ctx: &CompileContext<'_>, value: &Value) -> CompiledExpr {
    fn is_known_type_name(name: &str) -> bool {
        matches!(
            name,
            "string" | "number" | "integer" | "boolean" | "null" | "array" | "object"
        )
    }

    let backend = &ctx.config.backend;
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
                CompiledExpr::from_bool_expr(quote! { match instance { #(#arms),* } }, &schema_path)
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
                CompiledExpr::from_bool_expr(
                    quote! { matches!(instance, #(#patterns)|*) },
                    &schema_path,
                )
            }
        }
        _ => invalid_schema_type_expression(value, &["string", "array"]),
    }
}

fn generate_type_check(ctx: &CompileContext<'_>, value: &str, schema_path: &str) -> CompiledExpr {
    let backend = &ctx.config.backend;
    match value {
        "string" => CompiledExpr::from_bool_expr(backend.instance_is_string(), schema_path),
        "number" => CompiledExpr::from_bool_expr(backend.instance_is_number(), schema_path),
        "integer" => {
            CompiledExpr::from_bool_expr(backend.instance_is_integer(ctx.draft), schema_path)
        }
        "boolean" => CompiledExpr::from_bool_expr(backend.instance_is_boolean(), schema_path),
        "null" => CompiledExpr::from_bool_expr(backend.instance_is_null(), schema_path),
        "array" => CompiledExpr::from_bool_expr(backend.instance_is_array(), schema_path),
        "object" => CompiledExpr::from_bool_expr(backend.instance_is_object(), schema_path),
        _ => invalid_schema_unexpected_type_expression(),
    }
}
