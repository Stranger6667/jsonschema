use super::super::{compile_schema, CompileContext, CompiledExpr};
use crate::codegen::emit::ValueEmitter;
use quote::{format_ident, quote};
use serde_json::Value;

pub(crate) fn compile<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    value: &Value,
    prefix_len: Option<usize>,
) -> CompiledExpr {
    if let Some(prefix_len) = prefix_len {
        compile_with_prefix(ctx, value, prefix_len)
    } else {
        compile_plain(ctx, value)
    }
}

fn compile_plain<E: ValueEmitter>(ctx: &mut CompileContext<'_, E>, value: &Value) -> CompiledExpr {
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
                let get_expr = E::array_get(format_ident!("arr"), idx);
                CompiledExpr::with_validate_and_collect_blocks(
                    quote! { #get_expr.map_or(true, |instance| #is_valid) },
                    quote! {
                        if let Some(instance) = #get_expr {
                            let __path = &__path.push(#idx);
                            #expr
                        }
                    },
                    quote! {
                        if let Some(instance) = #get_expr {
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
        let iter_expr = E::array_iter(format_ident!("arr"));
        CompiledExpr::with_validate_and_collect_blocks(
            quote! { #iter_expr.all(|instance| #is_valid) },
            quote! {
                for (idx, item) in #iter_expr.enumerate() {
                    let instance = item;
                    let __path = &__path.push(idx);
                    #expr
                }
            },
            quote! {
                for (idx, item) in #iter_expr.enumerate() {
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

fn compile_with_prefix<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    value: &Value,
    prefix_len: usize,
) -> CompiledExpr {
    let err_instance = E::err_instance(format_ident!("instance"));
    let schema_path = ctx.schema_path_for_keyword("items");
    match value {
        Value::Bool(true) => CompiledExpr::always_true(),
        Value::Bool(false) => {
            let len_expr = E::array_len(format_ident!("arr"));
            let get_expr = E::array_get(format_ident!("arr"), prefix_len);
            let iter_expr = E::array_iter(format_ident!("arr"));
            CompiledExpr::with_validate_and_collect_blocks(
                quote! { #len_expr <= #prefix_len },
                quote! {
                    if let Some(item) = #get_expr {
                        let instance = item;
                        let __path = &__path.push(#prefix_len);
                        return Some(__err::false_schema(
                            #schema_path, __path.into(), #err_instance,
                        ));
                    }
                },
                quote! {
                    for (idx, item) in #iter_expr.enumerate().skip(#prefix_len) {
                        let instance = item;
                        let __path = &__path.push(idx);
                        __errors.push(__err::false_schema(
                            #schema_path, __path.into(), #err_instance,
                        ));
                    }
                },
            )
        }
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
            let iter_expr = E::array_iter(format_ident!("arr"));
            CompiledExpr::with_validate_and_collect_blocks(
                quote! { #iter_expr.skip(#prefix_len).all(|instance| #is_valid) },
                quote! {
                    for (idx, item) in #iter_expr.enumerate().skip(#prefix_len) {
                        let instance = item;
                        let __path = &__path.push(idx);
                        #expr
                    }
                },
                quote! {
                    for (idx, item) in #iter_expr.enumerate().skip(#prefix_len) {
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
