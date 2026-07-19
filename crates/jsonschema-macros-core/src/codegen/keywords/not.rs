use super::super::{compile_schema, CompileContext, CompiledExpr};
use quote::quote;
use serde_json::Value;

pub(crate) fn compile(ctx: &mut CompileContext<'_>, value: &Value) -> CompiledExpr {
    let inner = compile_schema(ctx, value);
    let schema_path = ctx.schema_path_for_keyword("not");
    let not_schema_json = serde_json::to_string(value).expect("Failed to serialize not schema");

    let report_error = quote! {
        static NOT_SCHEMA: __Lazy<serde_json::Value> =
            __Lazy::new(|| {
                serde_json::from_str(#not_schema_json).expect("Failed to parse not schema")
            });
        return Some(__err::not(
            #schema_path, __path.into(), instance, NOT_SCHEMA.clone(),
        ));
    };

    if inner.is_trivially_true() {
        // `not: true`: always invalid; report the error at the /not path.
        return CompiledExpr::with_validate_blocks(quote! { false }, report_error);
    }
    let is_valid = inner.is_valid_token_stream();

    CompiledExpr::with_validate_blocks(
        quote! { !(#is_valid) },
        quote! {
            if #is_valid {
                #report_error
            }
        },
    )
}
