use super::super::{errors::invalid_schema_type_expression, CompileContext, CompiledExpr};
use quote::quote;
use serde_json::Value;

pub(crate) fn compile(ctx: &CompileContext<'_>, value: &Value) -> Option<CompiledExpr> {
    match value.as_bool() {
        Some(true) => {
            let schema_path = ctx.schema_path_for_keyword("uniqueItems");
            Some(CompiledExpr::with_validate_blocks(
                quote! { jsonschema::keywords_helpers::unique_items::is_unique(arr) },
                quote! {
                    if !jsonschema::keywords_helpers::unique_items::is_unique(arr) {
                        return Some(jsonschema::keywords_helpers::error::unique_items(
                            #schema_path, __path.clone(), instance,
                        ));
                    }
                },
                quote! {
                    if !jsonschema::keywords_helpers::unique_items::is_unique(arr) {
                        __errors.push(jsonschema::keywords_helpers::error::unique_items(
                            #schema_path, __path.clone(), instance,
                        ));
                    }
                },
            ))
        }
        Some(false) => None,
        None => Some(invalid_schema_type_expression(value, &["boolean"])),
    }
}
