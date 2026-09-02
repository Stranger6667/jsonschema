use crate::{codegen::emit::ValueEmitter, context::CompileContext};
use quote::{format_ident, quote};
use serde_json::Value;

use super::super::CompiledExpr;

/// Compile the "const" keyword.
pub(in super::super) fn compile<E: ValueEmitter>(
    ctx: &CompileContext<'_, E>,
    value: &Value,
) -> CompiledExpr {
    let err_instance = E::err_instance(format_ident!("instance"));
    let schema_path = ctx.schema_path_for_keyword("const");
    let const_json = serde_json::to_string(value).expect("Failed to serialize const value");

    let is_valid = match value {
        // Scalar constants can use direct checks without constructing serde_json::Value.
        Value::Null => E::instance_is_null(),
        Value::Bool(expected) => {
            let as_bool = E::instance_as_bool();
            quote! { #as_bool == Some(#expected) }
        }
        Value::String(expected) => {
            let as_str = E::instance_as_str();
            quote! { #as_str == Some(#expected) }
        }
        Value::Number(expected) => {
            let num_json = expected.to_string();
            let number_arm = E::match_number_arm(
                quote! { jsonschema::__private::cmp::equal_numbers(n, &*EXPECTED) },
            );
            let number_match = E::type_match(
                format_ident!("instance"),
                vec![number_arm, quote! { _ => false, }],
            );
            quote! {
                {
                    static EXPECTED: __Lazy<serde_json::Number> =
                        __Lazy::new(|| {
                            serde_json::from_str(#num_json)
                                .expect("Failed to parse const number")
                        });
                    #number_match
                }
            }
        }
        Value::Array(_) | Value::Object(_) => {
            let equals = E::instance_equals_value(quote! { &*EXPECTED });
            quote! {
                {
                    static EXPECTED: __Lazy<serde_json::Value> =
                        __Lazy::new(|| {
                            serde_json::from_str(#const_json)
                                .expect("Failed to parse const value")
                        });
                    #equals
                }
            }
        }
    };

    // `validate` reuses the same scalar-optimized check and only constructs the expected
    // `serde_json::Value` on the error path.
    let validate_block = quote! {
        if !(#is_valid) {
            static CONST_EXPECTED: __Lazy<serde_json::Value> =
                __Lazy::new(|| {
                    serde_json::from_str(#const_json).expect("Failed to parse const value")
                });
            return Some(__err::constant(
                #schema_path, __path.into(), #err_instance, CONST_EXPECTED.clone(),
            ));
        }
    };

    CompiledExpr::with_validate_blocks(is_valid, validate_block)
}
