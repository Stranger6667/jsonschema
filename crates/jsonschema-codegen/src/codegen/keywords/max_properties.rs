use super::super::{parse_nonnegative_integer_keyword, CompileContext, CompiledExpr};
use quote::quote;
use serde_json::Value;

pub(crate) fn compile(ctx: &CompileContext<'_>, value: &Value) -> CompiledExpr {
    let object_len = ctx.config.backend.object_len(quote! { obj });
    match parse_nonnegative_integer_keyword(ctx.draft, value) {
        Ok(max) => {
            let schema_path = ctx.schema_path_for_keyword("maxProperties");
            CompiledExpr::with_validate_blocks(
                quote! { #object_len <= #max as usize },
                quote! {
                    if #object_len > #max as usize {
                        return Some(jsonschema::keywords_helpers::error::max_properties(
                            #schema_path, __path.clone(), instance, #max,
                        ));
                    }
                },
                quote! {
                    if #object_len > #max as usize {
                        __errors.push(jsonschema::keywords_helpers::error::max_properties(
                            #schema_path, __path.clone(), instance, #max,
                        ));
                    }
                },
            )
        }
        Err(e) => e,
    }
}
