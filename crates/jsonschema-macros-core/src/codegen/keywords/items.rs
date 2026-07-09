use super::super::{compile_schema, CompileContext, CompiledExpr};
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
                    ctx.with_schema_path_segment(&idx_str, |ctx| {
                        ctx.with_instance_scope(|ctx| compile_schema(ctx, schema))
                    })
                });
                if compiled.is_trivially_true() {
                    return CompiledExpr::always_true();
                }
                let is_valid = compiled.is_valid_token_stream();
                let expr = compiled.validate.as_token_stream();
                let child_collect = compiled.collect.as_token_stream();
                CompiledExpr::with_validate_and_collect_blocks(
                    quote! { arr.get(#idx).map_or(true, |instance| #is_valid) },
                    quote! {
                        if let Some(instance) = arr.get(#idx) {
                            let __path = &__path.push(#idx);
                            #expr
                        }
                    },
                    quote! {
                        if let Some(instance) = arr.get(#idx) {
                            if !(#is_valid) {
                                let __path = &__path.push(#idx);
                                #child_collect
                            }
                        }
                    },
                )
            })
            .collect();
        CompiledExpr::combine_and(compiled)
    } else {
        let compiled = ctx.with_schema_path_segment("items", |ctx| {
            ctx.with_instance_scope(|ctx| compile_schema(ctx, value))
        });
        if compiled.is_trivially_true() {
            return CompiledExpr::always_true();
        }
        let is_valid = compiled.is_valid_token_stream();
        let expr = compiled.validate.as_token_stream();
        let child_collect = compiled.collect.as_token_stream();
        CompiledExpr::with_validate_and_collect_blocks(
            quote! { arr.iter().all(|instance| #is_valid) },
            quote! {
                for (idx, item) in arr.iter().enumerate() {
                    let instance = item;
                    let __path = &__path.push(idx);
                    #expr
                }
            },
            quote! {
                for (idx, item) in arr.iter().enumerate() {
                    let instance = item;
                    if !(#is_valid) {
                        let __path = &__path.push(idx);
                        #child_collect
                    }
                }
            },
        )
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
        Value::Bool(false) => CompiledExpr::with_validate_and_collect_blocks(
            quote! { arr.len() <= #prefix_len },
            quote! {
                if let Some(item) = arr.get(#prefix_len) {
                    let instance = item;
                    let __path = &__path.push(#prefix_len);
                    return Some(jsonschema::__private::error::false_schema(
                        #schema_path, __path.into(), instance,
                    ));
                }
            },
            quote! {
                for (idx, item) in arr.iter().enumerate().skip(#prefix_len) {
                    let instance = item;
                    let __path = &__path.push(idx);
                    __errors.push(jsonschema::__private::error::false_schema(
                        #schema_path, __path.into(), instance,
                    ));
                }
            },
        ),
        _ => {
            let compiled = ctx.with_schema_path_segment("items", |ctx| {
                ctx.with_instance_scope(|ctx| compile_schema(ctx, value))
            });
            if compiled.is_trivially_true() {
                return CompiledExpr::always_true();
            }
            let is_valid = compiled.is_valid_token_stream();
            let expr = compiled.validate.as_token_stream();
            let child_collect = compiled.collect.as_token_stream();
            CompiledExpr::with_validate_and_collect_blocks(
                quote! { arr.iter().skip(#prefix_len).all(|instance| #is_valid) },
                quote! {
                    for (idx, item) in arr.iter().enumerate().skip(#prefix_len) {
                        let instance = item;
                        let __path = &__path.push(idx);
                        #expr
                    }
                },
                quote! {
                    for (idx, item) in arr.iter().enumerate().skip(#prefix_len) {
                        let instance = item;
                        if !(#is_valid) {
                            let __path = &__path.push(idx);
                            #child_collect
                        }
                    }
                },
            )
        }
    }
}
