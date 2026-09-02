use crate::codegen::emit::ValueEmitter;
use std::collections::HashSet;

use super::super::{
    compile_schema,
    errors::{invalid_schema_expression, invalid_schema_type_expression},
    expr::ValidateBlock,
    CompileContext, CompiledExpr,
};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use serde_json::Value;

/// Compile the legacy `dependencies` keyword (Draft 4/6/7).
pub(crate) fn compile<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    value: &Value,
) -> Option<CompiledExpr> {
    let err_instance = E::err_instance(format_ident!("instance"));
    let Value::Object(dependencies) = value else {
        return Some(invalid_schema_type_expression(value, &["object"]));
    };
    if dependencies.is_empty() {
        return None;
    }
    let schema_path = ctx.schema_path_for_keyword("dependencies");
    let checks: Vec<CompiledExpr> = dependencies
        .iter()
        .map(|(prop, dependency)| match dependency {
            Value::Array(required_props) => {
                let mut props = Vec::with_capacity(required_props.len());
                for required_prop in required_props {
                    let Some(prop_name) = required_prop.as_str() else {
                        return invalid_schema_type_expression(required_prop, &["string"]);
                    };
                    props.push(prop_name);
                }
                if props.is_empty() {
                    CompiledExpr::always_true()
                } else {
                    let guard = E::object_contains_key(format_ident!("obj"), prop);
                    let checks: Vec<TokenStream> = props
                        .iter()
                        .map(|prop| E::object_contains_key(format_ident!("obj"), prop))
                        .collect();
                    CompiledExpr::with_validate_and_collect_blocks(
                        quote! {
                            if #guard {
                                #(#checks)&&*
                            } else {
                                true
                            }
                        },
                        quote! {
                            if #guard {
                                #(
                                    if !#checks {
                                        return Some(__err::required(
                                            #schema_path, __path.into(), #err_instance, #props,
                                        ));
                                    }
                                )*
                            }
                        },
                        quote! {
                            if #guard {
                                #(
                                    if !#checks {
                                        __errors.push(__err::required(
                                            #schema_path, __path.into(), #err_instance, #props,
                                        ));
                                    }
                                )*
                            }
                        },
                    )
                }
            }
            schema => {
                let compiled = ctx.with_schema_path_segment("dependencies", |ctx| {
                    ctx.with_schema_path_segment(prop, |ctx| compile_schema(ctx, schema))
                });
                let is_valid = compiled.is_valid_token_stream();
                let guard = E::object_contains_key(format_ident!("obj"), prop);
                match &compiled.validate {
                    ValidateBlock::Expr(expr) => {
                        let child_collect = compiled.collect.as_token_stream();
                        CompiledExpr::with_validate_and_collect_blocks(
                            quote! { if #guard { #is_valid } else { true } },
                            quote! { if #guard { #expr } },
                            quote! { if #guard { #child_collect } },
                        )
                    }
                    ValidateBlock::AlwaysValid => CompiledExpr::always_true(),
                }
            }
        })
        .collect();

    Some(CompiledExpr::combine_and(checks))
}

/// Compile the `dependentRequired` keyword (Draft 2019-09+).
pub(crate) fn compile_dependent_required<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    value: &Value,
) -> Option<CompiledExpr> {
    let err_instance = E::err_instance(format_ident!("instance"));
    let Value::Object(dependencies) = value else {
        return Some(invalid_schema_type_expression(value, &["object"]));
    };
    if dependencies.is_empty() {
        return None;
    }
    let schema_path = ctx.schema_path_for_keyword("dependentRequired");
    let checks: Vec<CompiledExpr> = dependencies
        .iter()
        .map(|(prop, required)| {
            let Value::Array(required_array) = required else {
                return invalid_schema_type_expression(required, &["array"]);
            };
            let mut seen = HashSet::with_capacity(required_array.len());
            let mut required_props: Vec<&str> = Vec::with_capacity(required_array.len());
            for required_prop in required_array {
                let Some(required_name) = required_prop.as_str() else {
                    return invalid_schema_type_expression(required_prop, &["string"]);
                };
                if !seen.insert(required_name) {
                    return invalid_schema_expression(&format!(
                        "{required} has non-unique elements"
                    ));
                }
                required_props.push(required_name);
            }
            if required_props.is_empty() {
                return CompiledExpr::always_true();
            }
            let guard = E::object_contains_key(format_ident!("obj"), prop);
            let required_checks: Vec<TokenStream> = required_props
                .iter()
                .map(|required_prop| E::object_contains_key(format_ident!("obj"), required_prop))
                .collect();
            CompiledExpr::with_validate_and_collect_blocks(
                quote! {
                    if #guard {
                        #(#required_checks)&&*
                    } else {
                        true
                    }
                },
                quote! {
                    if #guard {
                        #(
                            if !#required_checks {
                                return Some(__err::required(
                                    #schema_path, __path.into(), #err_instance, #required_props,
                                ));
                            }
                        )*
                    }
                },
                quote! {
                    if #guard {
                        #(
                            if !#required_checks {
                                __errors.push(__err::required(
                                    #schema_path, __path.into(), #err_instance, #required_props,
                                ));
                            }
                        )*
                    }
                },
            )
        })
        .collect();

    Some(CompiledExpr::combine_and(checks))
}

/// Compile the `dependentSchemas` keyword (Draft 2019-09+).
pub(crate) fn compile_dependent_schemas<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    value: &Value,
) -> Option<CompiledExpr> {
    let Value::Object(dependencies) = value else {
        return Some(invalid_schema_type_expression(value, &["object"]));
    };
    if dependencies.is_empty() {
        return None;
    }
    let checks: Vec<CompiledExpr> = dependencies
        .iter()
        .map(|(prop, subschema)| {
            let compiled = ctx.with_schema_path_segment("dependentSchemas", |ctx| {
                ctx.with_schema_path_segment(prop, |ctx| compile_schema(ctx, subschema))
            });
            let is_valid = compiled.is_valid_token_stream();
            let guard = E::object_contains_key(format_ident!("obj"), prop);
            match &compiled.validate {
                ValidateBlock::Expr(expr) => {
                    let child_collect = compiled.collect.as_token_stream();
                    CompiledExpr::with_validate_and_collect_blocks(
                        quote! { if #guard { #is_valid } else { true } },
                        quote! { if #guard { #expr } },
                        quote! { if #guard { #child_collect } },
                    )
                }
                ValidateBlock::AlwaysValid => CompiledExpr::always_true(),
            }
        })
        .collect();
    Some(CompiledExpr::combine_and(checks))
}
