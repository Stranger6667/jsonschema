use super::super::{CompileContext, CompiledExpr};
use quote::quote;

/// Compile a required check for a single field name.
pub(crate) fn compile_single(ctx: &CompileContext<'_>, name: &str) -> CompiledExpr {
    let check = crate::codegen::emit_serde::object_contains_key(quote! { obj }, name);
    let schema_path = ctx.schema_path_for_keyword("required");
    CompiledExpr::with_validate_blocks(
        quote! { #check },
        quote! {
            if !(#check) {
                return Some(jsonschema::__private::error::required(
                    #schema_path, __path.into(), instance, #name,
                ));
            }
        },
    )
}
