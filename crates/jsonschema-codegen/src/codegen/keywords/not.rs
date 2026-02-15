use super::super::{compile_schema, CompileContext, CompiledExpr};
use quote::quote;
use serde_json::Value;

pub(crate) fn compile(ctx: &mut CompileContext<'_>, value: &Value) -> CompiledExpr {
    let inner = compile_schema(ctx, value);
    if inner.is_trivially_false() {
        // `not: false` => always valid
        return CompiledExpr::always_true();
    }
    let schema_path = ctx.schema_path_for_keyword("not");
    let not_schema_json = serde_json::to_string(value).expect("Failed to serialize not schema");

    if inner.is_trivially_true() {
        // `not: true` => always invalid — emit proper error with /not path
        return CompiledExpr::with_validate_blocks(
            quote! { false },
            quote! {
                static NOT_SCHEMA: std::sync::LazyLock<serde_json::Value> =
                    std::sync::LazyLock::new(|| {
                        serde_json::from_str(#not_schema_json).expect("Failed to parse not schema")
                    });
                let __r = Some(jsonschema::keywords_helpers::error::not(
                    #schema_path, __path.clone(), instance, NOT_SCHEMA.clone(),
                ));
                if let Some(__e) = __r { return Some(__e); }
            },
            quote! {
                static NOT_SCHEMA: std::sync::LazyLock<serde_json::Value> =
                    std::sync::LazyLock::new(|| {
                        serde_json::from_str(#not_schema_json).expect("Failed to parse not schema")
                    });
                __errors.push(jsonschema::keywords_helpers::error::not(
                    #schema_path, __path.clone(), instance, NOT_SCHEMA.clone(),
                ));
            },
        );
    }
    let is_valid_ts = inner.is_valid_ts();

    CompiledExpr::with_validate_blocks(
        quote! { !(#is_valid_ts) },
        quote! {
            if #is_valid_ts {
                static NOT_SCHEMA: std::sync::LazyLock<serde_json::Value> =
                    std::sync::LazyLock::new(|| {
                        serde_json::from_str(#not_schema_json).expect("Failed to parse not schema")
                    });
                return Some(jsonschema::keywords_helpers::error::not(
                    #schema_path, __path.clone(), instance, NOT_SCHEMA.clone(),
                ));
            }
        },
        quote! {
            if #is_valid_ts {
                static NOT_SCHEMA: std::sync::LazyLock<serde_json::Value> =
                    std::sync::LazyLock::new(|| {
                        serde_json::from_str(#not_schema_json).expect("Failed to parse not schema")
                    });
                __errors.push(jsonschema::keywords_helpers::error::not(
                    #schema_path, __path.clone(), instance, NOT_SCHEMA.clone(),
                ));
            }
        },
    )
}
