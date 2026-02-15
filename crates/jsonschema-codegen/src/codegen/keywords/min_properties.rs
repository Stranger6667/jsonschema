use super::super::{parse_nonnegative_integer_keyword, CompileContext, CompiledExpr};
use quote::quote;
use serde_json::Value;

pub(crate) fn compile(ctx: &CompileContext<'_>, value: &Value) -> CompiledExpr {
    let object_len = ctx.config.backend.object_len(quote! { obj });
    match parse_nonnegative_integer_keyword(ctx.draft, value) {
        Ok(min) => {
            let schema_path = ctx.schema_path_for_keyword("minProperties");
            CompiledExpr::with_validate_blocks(
                quote! { #object_len >= #min as usize },
                quote! {
                    if #object_len < #min as usize {
                        return Some(jsonschema::keywords_helpers::error::min_properties(
                            #schema_path, __path.clone(), instance, #min,
                        ));
                    }
                },
                quote! {
                    if #object_len < #min as usize {
                        __errors.push(jsonschema::keywords_helpers::error::min_properties(
                            #schema_path, __path.clone(), instance, #min,
                        ));
                    }
                },
            )
        }
        Err(e) => e,
    }
}
