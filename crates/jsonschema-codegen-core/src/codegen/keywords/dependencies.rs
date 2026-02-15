use std::collections::HashSet;

use super::super::{
    compile_schema,
    errors::{invalid_schema_expression, invalid_schema_type_expression},
    expr::ValidateBlock,
    CompileContext, CompiledExpr,
};
use quote::quote;
use serde_json::Value;

/// Compile the legacy `dependencies` keyword (Draft 4/6/7).
pub(crate) fn compile(ctx: &mut CompileContext<'_>, value: &Value) -> Option<CompiledExpr> {
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
                    CompiledExpr::with_validate_blocks(
                        quote! {
                            if obj.contains_key(#prop) {
                                #(obj.contains_key(#props))&&*
                            } else {
                                true
                            }
                        },
                        quote! {
                            if obj.contains_key(#prop) {
                                #(
                                    if !obj.contains_key(#props) {
                                        return Some(jsonschema::__private::error::required(
                                            #schema_path, __path.into(), instance, #props,
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
                match &compiled.validate {
                    ValidateBlock::Expr(expr) => CompiledExpr::with_validate_blocks(
                        quote! { if obj.contains_key(#prop) { #is_valid } else { true } },
                        quote! { if obj.contains_key(#prop) { #expr } },
                    ),
                    ValidateBlock::AlwaysValid => CompiledExpr::always_true(),
                }
            }
        })
        .collect();

    Some(CompiledExpr::combine_and(checks))
}

/// Compile the `dependentRequired` keyword (Draft 2019-09+).
pub(crate) fn compile_dependent_required(
    ctx: &mut CompileContext<'_>,
    value: &Value,
) -> Option<CompiledExpr> {
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
            CompiledExpr::with_validate_blocks(
                quote! {
                    if obj.contains_key(#prop) {
                        #(obj.contains_key(#required_props))&&*
                    } else {
                        true
                    }
                },
                quote! {
                    if obj.contains_key(#prop) {
                        #(
                            if !obj.contains_key(#required_props) {
                                return Some(jsonschema::__private::error::required(
                                    #schema_path, __path.into(), instance, #required_props,
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
pub(crate) fn compile_dependent_schemas(
    ctx: &mut CompileContext<'_>,
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
            match &compiled.validate {
                ValidateBlock::Expr(expr) => CompiledExpr::with_validate_blocks(
                    quote! { if obj.contains_key(#prop) { #is_valid } else { true } },
                    quote! { if obj.contains_key(#prop) { #expr } },
                ),
                ValidateBlock::AlwaysValid => CompiledExpr::always_true(),
            }
        })
        .collect();
    Some(CompiledExpr::combine_and(checks))
}
