use super::super::{CompileContext, CompiledExpr};
use quote::quote;

/// Compile a required check for a single field name.
pub(crate) fn compile_single(ctx: &CompileContext<'_>, name: &str) -> CompiledExpr {
    let check = ctx.config.backend.object_contains_key(quote! { obj }, name);
    let not_check = quote! { !(#check) };
    let schema_path = ctx.schema_path_for_keyword("required");
    CompiledExpr::with_validate_blocks(
        check,
        quote! {
            if #not_check {
                return Some(jsonschema::keywords_helpers::error::required(
                    #schema_path, __path.clone(), instance, #name,
                ));
            }
        },
        quote! {
            if #not_check {
                __errors.push(jsonschema::keywords_helpers::error::required(
                    #schema_path, __path.clone(), instance, #name,
                ));
            }
        },
    )
}
