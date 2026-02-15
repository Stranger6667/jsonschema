use super::super::{parse_nonnegative_integer_keyword, CompileContext, CompiledExpr};
use quote::quote;
use serde_json::Value;

pub(crate) fn compile(ctx: &CompileContext<'_>, value: &Value) -> CompiledExpr {
    match parse_nonnegative_integer_keyword(ctx.draft, value) {
        Ok(min) => {
            let schema_path = ctx.current_schema_path().to_owned();
            CompiledExpr::with_validate_blocks(
                quote! { s.chars().count() >= #min as usize },
                quote! {
                    if s.chars().count() < #min as usize {
                        return Some(jsonschema::keywords_helpers::error::min_length(
                            #schema_path, __path.clone(), instance, #min,
                        ));
                    }
                },
                quote! {
                    if s.chars().count() < #min as usize {
                        __errors.push(jsonschema::keywords_helpers::error::min_length(
                            #schema_path, __path.clone(), instance, #min,
                        ));
                    }
                },
            )
        }
        Err(e) => e,
    }
}
