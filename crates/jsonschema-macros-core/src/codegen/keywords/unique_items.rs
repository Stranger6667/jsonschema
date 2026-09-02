use super::super::{errors::invalid_schema_type_expression, CompileContext, CompiledExpr};
use crate::codegen::emit::ValueEmitter;
use quote::{format_ident, quote};
use serde_json::Value;

pub(crate) fn compile<E: ValueEmitter>(
    ctx: &CompileContext<'_, E>,
    value: &Value,
) -> Option<CompiledExpr> {
    let err_instance = E::err_instance(format_ident!("instance"));
    let is_unique = E::array_is_unique(format_ident!("arr"));
    match value.as_bool() {
        Some(true) => {
            let schema_path = ctx.schema_path_for_keyword("uniqueItems");
            Some(CompiledExpr::with_validate_blocks(
                is_unique.clone(),
                quote! {
                    if !#is_unique {
                        return Some(__err::unique_items(
                            #schema_path, __path.into(), #err_instance,
                        ));
                    }
                },
            ))
        }
        Some(false) => None,
        None => Some(invalid_schema_type_expression(value, &["boolean"])),
    }
}
