use crate::context::CompileContext;
use quote::quote;
use serde_json::Value;

use super::super::CompiledExpr;

/// Compile the "const" keyword.
pub(in super::super) fn compile(ctx: &CompileContext<'_>, value: &Value) -> CompiledExpr {
    let schema_path = ctx.schema_path_for_keyword("const");
    let const_json = serde_json::to_string(value).expect("Failed to serialize const value");

    let validate_block = quote! {
        {
            static CONST_EXPECTED: std::sync::LazyLock<serde_json::Value> =
                std::sync::LazyLock::new(|| {
                    serde_json::from_str(#const_json).expect("Failed to parse const value")
                });
            if !jsonschema::__private::cmp::equal(instance, &*CONST_EXPECTED) {
                return Some(jsonschema::__private::error::constant(
                    #schema_path, __path.clone(), instance, CONST_EXPECTED.clone(),
                ));
            }
        }
    };
    let is_valid_ts = match value {
        // Scalar constants can use direct checks without constructing serde_json::Value.
        Value::Null => crate::codegen::emit_serde::instance_is_null(),
        Value::Bool(expected) => {
            let as_bool = crate::codegen::emit_serde::instance_as_bool();
            quote! { #as_bool == Some(#expected) }
        }
        Value::String(expected) => {
            let as_str = crate::codegen::emit_serde::instance_as_str();
            quote! { #as_str == Some(#expected) }
        }
        Value::Number(expected) => {
            let num_json = expected.to_string();
            let number_arm = crate::codegen::emit_serde::match_number_arm(
                quote! { jsonschema::__private::cmp::equal_numbers(n, &*EXPECTED) },
            );
            quote! {
                {
                    static EXPECTED: std::sync::LazyLock<serde_json::Number> =
                        std::sync::LazyLock::new(|| {
                            serde_json::from_str(#num_json)
                                .expect("Failed to parse const number")
                        });
                    match instance {
                        #number_arm,
                        _ => false,
                    }
                }
            }
        }
        Value::Array(_) | Value::Object(_) => {
            quote! {
                {
                    static EXPECTED: std::sync::LazyLock<serde_json::Value> =
                        std::sync::LazyLock::new(|| {
                            serde_json::from_str(#const_json)
                                .expect("Failed to parse const value")
                        });
                    jsonschema::__private::cmp::equal(instance, &*EXPECTED)
                }
            }
        }
    };

    CompiledExpr::with_validate_blocks(is_valid_ts, validate_block)
}
