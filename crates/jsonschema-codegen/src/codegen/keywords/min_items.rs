use super::super::{parse_nonnegative_integer_keyword, CompileContext, CompiledExpr};
use quote::quote;
use serde_json::Value;

pub(crate) fn compile(ctx: &CompileContext<'_>, value: &Value) -> CompiledExpr {
    let array_len = ctx.config.backend.array_len(quote! { arr });
    match parse_nonnegative_integer_keyword(ctx.draft, value) {
        Ok(min) => {
            let schema_path = ctx.schema_path_for_keyword("minItems");
            CompiledExpr::with_validate_blocks(
                quote! { #array_len >= #min as usize },
                quote! {
                    if #array_len < #min as usize {
                        return Some(jsonschema::keywords_helpers::error::min_items(
                            #schema_path, __path.clone(), instance, #min,
                        ));
                    }
                },
                quote! {
                    if #array_len < #min as usize {
                        __errors.push(jsonschema::keywords_helpers::error::min_items(
                            #schema_path, __path.clone(), instance, #min,
                        ));
                    }
                },
            )
        }
        Err(e) => e,
    }
}
