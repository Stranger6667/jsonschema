use super::super::{compile_schema, expr::ValidateBlock, CompileContext, CompiledExpr};
use quote::quote;
use serde_json::Value;

pub(crate) fn compile(
    ctx: &mut CompileContext<'_>,
    value: &Value,
    prefix_len: Option<usize>,
) -> CompiledExpr {
    if let Some(prefix_len) = prefix_len {
        compile_with_prefix(ctx, value, prefix_len)
    } else {
        compile_plain(ctx, value)
    }
}

fn compile_plain(ctx: &mut CompileContext<'_>, value: &Value) -> CompiledExpr {
    if let Value::Array(schemas) = value {
        // Tuple validation (draft <= 2019-09 only)
        let compiled: Vec<CompiledExpr> = schemas
            .iter()
            .enumerate()
            .map(|(idx, schema)| {
                let idx_str = idx.to_string();
                let compiled = ctx.with_schema_path_segment("items", |ctx| {
                    ctx.with_schema_path_segment(&idx_str, |ctx| compile_schema(ctx, schema))
                });
                if compiled.is_trivially_true() {
                    return CompiledExpr::always_true();
                }
                let is_valid_ts = compiled.is_valid_ts();
                match &compiled.validate {
                    ValidateBlock::Expr(v) => CompiledExpr::with_validate_blocks(
                        quote! { arr.get(#idx).map_or(true, |instance| #is_valid_ts) },
                        quote! {
                            if let Some(instance) = arr.get(#idx) {
                                let __path = __path.join(#idx);
                                #v
                            }
                        },
                    ),
                    ValidateBlock::AlwaysValid => CompiledExpr::always_true(),
                }
            })
            .collect();
        CompiledExpr::combine_and(compiled)
    } else {
        let compiled = ctx.with_schema_path_segment("items", |ctx| compile_schema(ctx, value));
        if compiled.is_trivially_true() {
            return CompiledExpr::always_true();
        }
        let is_valid_ts = compiled.is_valid_ts();
        match &compiled.validate {
            ValidateBlock::Expr(v) => CompiledExpr::with_validate_blocks(
                quote! { arr.iter().all(|instance| #is_valid_ts) },
                quote! {
                    for (idx, item) in arr.iter().enumerate() {
                        let instance = item;
                        let __path = __path.join(idx);
                        #v
                    }
                },
            ),
            ValidateBlock::AlwaysValid => CompiledExpr::always_true(),
        }
    }
}

fn compile_with_prefix(
    ctx: &mut CompileContext<'_>,
    value: &Value,
    prefix_len: usize,
) -> CompiledExpr {
    let schema_path = ctx.schema_path_for_keyword("items");
    match value {
        Value::Bool(true) => CompiledExpr::always_true(),
        Value::Bool(false) => CompiledExpr::with_validate_blocks(
            quote! { arr.len() <= #prefix_len },
            quote! {
                if let Some(item) = arr.get(#prefix_len) {
                    let instance = item;
                    let __path = __path.join(#prefix_len);
                    return Some(jsonschema::__private::error::false_schema(
                        #schema_path, __path.clone(), instance,
                    ));
                }
            },
        ),
        _ => {
            let compiled = ctx.with_schema_path_segment("items", |ctx| compile_schema(ctx, value));
            if compiled.is_trivially_true() {
                return CompiledExpr::always_true();
            }
            let is_valid_ts = compiled.is_valid_ts();
            match &compiled.validate {
                ValidateBlock::Expr(v) => CompiledExpr::with_validate_blocks(
                    quote! { arr.iter().skip(#prefix_len).all(|instance| #is_valid_ts) },
                    quote! {
                        for (idx, item) in arr.iter().enumerate().skip(#prefix_len) {
                            let instance = item;
                            let __path = __path.join(idx);
                            #v
                        }
                    },
                ),
                ValidateBlock::AlwaysValid => CompiledExpr::always_true(),
            }
        }
    }
}
