use super::super::{parse_nonnegative_integer_keyword, CompileContext, CompiledExpr};
use quote::quote;
use serde_json::Value;

pub(crate) fn compile(ctx: &CompileContext<'_>, value: &Value) -> CompiledExpr {
    match parse_nonnegative_integer_keyword(ctx.draft, value) {
        Ok(max) => {
            let schema_path = ctx.current_schema_path().to_owned();
            CompiledExpr::with_validate_blocks(
                quote! { s.chars().count() <= #max as usize },
                quote! {
                    if s.chars().count() > #max as usize {
                        return Some(jsonschema::keywords_helpers::error::max_length(
                            #schema_path, __path.clone(), instance, #max,
                        ));
                    }
                },
                quote! {
                    if s.chars().count() > #max as usize {
                        __errors.push(jsonschema::keywords_helpers::error::max_length(
                            #schema_path, __path.clone(), instance, #max,
                        ));
                    }
                },
            )
        }
        Err(e) => e,
    }
}
