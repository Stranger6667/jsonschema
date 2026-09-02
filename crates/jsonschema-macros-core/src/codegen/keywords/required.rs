use super::super::{CompileContext, CompiledExpr};
use crate::codegen::emit::ValueEmitter;
use quote::{format_ident, quote};

/// Compile a required check for a single field name.
pub(crate) fn compile_single<E: ValueEmitter>(
    ctx: &CompileContext<'_, E>,
    name: &str,
) -> CompiledExpr {
    let err_instance = E::err_instance(format_ident!("instance"));
    let check = E::object_contains_key(format_ident!("obj"), name);
    let schema_path = ctx.schema_path_for_keyword("required");
    CompiledExpr::with_validate_blocks(
        quote! { #check },
        quote! {
            if !(#check) {
                return Some(__err::required(
                    #schema_path, __path.into(), #err_instance, #name,
                ));
            }
        },
    )
}
