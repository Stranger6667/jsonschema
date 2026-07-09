use super::super::{compile_schema, CompileContext, CompiledExpr};
use quote::quote;
use serde_json::{Map, Value};

pub(crate) fn compile(
    ctx: &mut CompileContext<'_>,
    parent: &Map<String, Value>,
    if_schema: &Value,
) -> Option<CompiledExpr> {
    let then_schema = parent.get("then");
    let else_schema = parent.get("else");
    if then_schema.is_none() && else_schema.is_none() {
        return None;
    }

    let if_check = ctx.with_schema_path_segment("if", |ctx| compile_schema(ctx, if_schema));
    let then_check =
        then_schema.map(|s| ctx.with_schema_path_segment("then", |ctx| compile_schema(ctx, s)));
    let else_check =
        else_schema.map(|s| ctx.with_schema_path_segment("else", |ctx| compile_schema(ctx, s)));

    // A present branch that is trivially true adds no constraint; when every
    // present branch is trivial the whole keyword is a no-op.
    let then_trivial = then_check
        .as_ref()
        .is_none_or(CompiledExpr::is_trivially_true);
    let else_trivial = else_check
        .as_ref()
        .is_none_or(CompiledExpr::is_trivially_true);
    if then_trivial && else_trivial {
        return None;
    }

    let if_is_valid = if_check.is_valid_token_stream();
    let then_is_valid = then_check
        .as_ref()
        .map_or_else(|| quote! { true }, CompiledExpr::is_valid_token_stream);
    let else_is_valid = else_check
        .as_ref()
        .map_or_else(|| quote! { true }, CompiledExpr::is_valid_token_stream);
    let then_validate = then_check
        .as_ref()
        .map_or_else(|| quote! {}, |check| check.validate.as_token_stream());
    let else_validate = else_check
        .as_ref()
        .map_or_else(|| quote! {}, |check| check.validate.as_token_stream());
    let then_collect = then_check
        .as_ref()
        .map_or_else(|| quote! {}, |check| check.collect.as_token_stream());
    let else_collect = else_check
        .as_ref()
        .map_or_else(|| quote! {}, |check| check.collect.as_token_stream());

    Some(CompiledExpr::with_validate_and_collect_blocks(
        quote! { if #if_is_valid { #then_is_valid } else { #else_is_valid } },
        quote! { if #if_is_valid { #then_validate } else { #else_validate } },
        quote! { if #if_is_valid { #then_collect } else { #else_collect } },
    ))
}
