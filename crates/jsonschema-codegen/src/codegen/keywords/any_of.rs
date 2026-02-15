use super::super::{
    compile_schema,
    errors::{invalid_schema_non_empty_array_expression, invalid_schema_type_expression},
    CompileContext, CompiledExpr,
};
use quote::quote;
use serde_json::Value;

pub(crate) fn compile(ctx: &mut CompileContext<'_>, value: &Value) -> CompiledExpr {
    let Some(schemas) = value.as_array() else {
        return invalid_schema_type_expression(value, &["array"]);
    };
    if schemas.is_empty() {
        return invalid_schema_non_empty_array_expression();
    }
    let compiled: Vec<_> = schemas.iter().map(|s| compile_schema(ctx, s)).collect();
    if compiled.iter().any(CompiledExpr::is_trivially_true) {
        return CompiledExpr::always_true();
    }

    let schema_path = ctx.schema_path_for_keyword("anyOf");
    let is_valid_checks: Vec<_> = compiled.iter().map(CompiledExpr::is_valid_ts).collect();
    let is_valid_ts = quote! { (#(#is_valid_checks)||*) };

    CompiledExpr::with_validate_blocks(
        is_valid_ts.clone(),
        quote! {
            if !(#is_valid_ts) {
                return Some(jsonschema::keywords_helpers::error::any_of(
                    #schema_path, __path.clone(), instance,
                ));
            }
        },
        quote! {
            if !(#is_valid_ts) {
                __errors.push(jsonschema::keywords_helpers::error::any_of(
                    #schema_path, __path.clone(), instance,
                ));
            }
        },
    )
}
