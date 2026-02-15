use super::super::{parse_nonnegative_integer_keyword, CompileContext, CompiledExpr};
use quote::quote;
use serde_json::Value;

pub(crate) fn compile(ctx: &CompileContext<'_>, value: &Value) -> CompiledExpr {
    let array_len = ctx.config.backend.array_len(quote! { arr });
    match parse_nonnegative_integer_keyword(ctx.draft, value) {
        Ok(max) => {
            let schema_path = ctx.schema_path_for_keyword("maxItems");
            CompiledExpr::with_validate_blocks(
                quote! { #array_len <= #max as usize },
                quote! {
                    if #array_len > #max as usize {
                        return Some(jsonschema::keywords_helpers::error::max_items(
                            #schema_path, __path.clone(), instance, #max,
                        ));
                    }
                },
                quote! {
                    if #array_len > #max as usize {
                        __errors.push(jsonschema::keywords_helpers::error::max_items(
                            #schema_path, __path.clone(), instance, #max,
                        ));
                    }
                },
            )
        }
        Err(e) => e,
    }
}
