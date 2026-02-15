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

    match (then_schema, else_schema) {
        (Some(then_val), Some(else_val)) => {
            let if_check = ctx.with_schema_path_segment("if", |ctx| compile_schema(ctx, if_schema));
            let then_check =
                ctx.with_schema_path_segment("then", |ctx| compile_schema(ctx, then_val));
            let else_check =
                ctx.with_schema_path_segment("else", |ctx| compile_schema(ctx, else_val));
            let if_ts = if_check.is_valid_ts();
            let then_ts = then_check.is_valid_ts();
            let else_ts = else_check.is_valid_ts();
            let is_valid_ts = quote! { if #if_ts { #then_ts } else { #else_ts } };

            let then_v = then_check.validate.as_ts();
            let then_e = then_check.iter_errors.as_ts();
            let else_v = else_check.validate.as_ts();
            let else_e = else_check.iter_errors.as_ts();

            Some(CompiledExpr::with_validate_blocks(
                is_valid_ts,
                quote! { if #if_ts { #then_v } else { #else_v } },
                quote! { if #if_ts { #then_e } else { #else_e } },
            ))
        }
        (Some(then_val), None) => {
            let if_check = ctx.with_schema_path_segment("if", |ctx| compile_schema(ctx, if_schema));
            let then_check =
                ctx.with_schema_path_segment("then", |ctx| compile_schema(ctx, then_val));
            let if_ts = if_check.is_valid_ts();
            let then_ts = then_check.is_valid_ts();
            let is_valid_ts = quote! { if #if_ts { #then_ts } else { true } };

            let then_v = then_check.validate.as_ts();
            let then_e = then_check.iter_errors.as_ts();

            Some(CompiledExpr::with_validate_blocks(
                is_valid_ts,
                quote! { if #if_ts { #then_v } },
                quote! { if #if_ts { #then_e } },
            ))
        }
        (None, Some(else_val)) => {
            let if_check = ctx.with_schema_path_segment("if", |ctx| compile_schema(ctx, if_schema));
            let else_check =
                ctx.with_schema_path_segment("else", |ctx| compile_schema(ctx, else_val));
            let if_ts = if_check.is_valid_ts();
            let else_ts = else_check.is_valid_ts();
            let is_valid_ts = quote! { if #if_ts { true } else { #else_ts } };

            let else_v = else_check.validate.as_ts();
            let else_e = else_check.iter_errors.as_ts();

            Some(CompiledExpr::with_validate_blocks(
                is_valid_ts,
                quote! { if #if_ts { } else { #else_v } },
                quote! { if #if_ts { } else { #else_e } },
            ))
        }
        (None, None) => None,
    }
}
