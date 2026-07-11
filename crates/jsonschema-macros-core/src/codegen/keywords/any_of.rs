use super::super::{
    compile_schema,
    errors::{invalid_schema_non_empty_array_expression, invalid_schema_type_expression},
    CompileContext, CompiledExpr,
};
use quote::{format_ident, quote};
use serde_json::Value;

pub(crate) fn compile(ctx: &mut CompileContext<'_>, value: &Value) -> CompiledExpr {
    let Some(schemas) = value.as_array() else {
        return invalid_schema_type_expression(value, &["array"]);
    };
    if schemas.is_empty() {
        return invalid_schema_non_empty_array_expression();
    }
    let compiled: Vec<_> = ctx.with_schema_path_segment("anyOf", |ctx| {
        schemas
            .iter()
            .enumerate()
            .map(|(idx, schema)| {
                ctx.with_schema_path_segment(&idx.to_string(), |ctx| compile_schema(ctx, schema))
            })
            .collect()
    });
    if compiled.iter().any(CompiledExpr::is_trivially_true) {
        return CompiledExpr::always_true();
    }

    let branch_helpers: Vec<_> = compiled
        .iter()
        .map(|compiled| {
            ctx.register_branch_helper(
                compiled.is_valid_token_stream(),
                compiled.collect.as_token_stream(),
            )
        })
        .collect();
    let schema_path = ctx.schema_path_for_keyword("anyOf");
    let is_valid_checks: Vec<_> = branch_helpers
        .iter()
        .map(|idx| {
            let ident = format_ident!("is_branch_valid_{}", idx);
            quote! { #ident(instance) }
        })
        .collect();
    let is_valid = quote! { (#(#is_valid_checks)||*) };
    let branch_collectors: Vec<_> = branch_helpers
        .iter()
        .map(|idx| format_ident!("collect_branch_errors_{}", idx))
        .collect();
    let branch_count = branch_collectors.len();

    CompiledExpr::with_validate_blocks(
        is_valid.clone(),
        quote! {
            if !(#is_valid) {
                let mut __context = Vec::with_capacity(#branch_count);
                #({
                    let mut __branch_errors = Vec::new();
                    #branch_collectors(instance, __path, &mut __branch_errors);
                    __context.push(__branch_errors);
                })*
                return Some(jsonschema::__private::error::any_of(
                    #schema_path, __path.into(), instance, __context,
                ));
            }
        },
    )
}
