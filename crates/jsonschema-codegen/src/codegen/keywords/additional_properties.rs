use super::super::{compile_schema, expr::ValidateBlock, CompileContext, CompiledExpr};
use proc_macro2::TokenStream;
use quote::quote;
use serde_json::Value;

use super::pattern_coverage::build_pattern_coverage;

pub(super) fn compile_wildcard_arm(
    ctx: &mut CompileContext<'_>,
    additional_properties: Option<&Value>,
    pattern_properties: Option<&Value>,
) -> (Vec<TokenStream>, CompiledExpr) {
    let coverage = match build_pattern_coverage(ctx, pattern_properties) {
        Ok(coverage) => coverage,
        Err(error_expr) => return (Vec::new(), error_expr),
    };

    let schema_path = ctx.schema_path_for_keyword("additionalProperties");
    let statics = coverage.statics.clone();
    let pattern_cover_check = coverage.combined_check();

    let arm_body = match additional_properties {
        None | Some(Value::Bool(true)) => CompiledExpr::always_true(),
        Some(Value::Bool(false)) => match pattern_cover_check {
            None => CompiledExpr::always_false(),
            Some(check) => CompiledExpr::from_bool_expr(check, &schema_path),
        },
        Some(schema) => {
            let schema_check = ctx.with_schema_path_segment("additionalProperties", |ctx| {
                compile_schema(ctx, schema)
            });
            if schema_check.is_trivially_true() {
                CompiledExpr::always_true()
            } else {
                let schema_ts = schema_check.is_valid_ts();
                // Bind `instance = value` and extend `__path` here so that
                // the sub-schema validation sees the property value/path. The
                // wildcard match arm in `build_validate_blocks` does NOT rebind
                // these, keeping `instance` = outer object for the `false` case.
                match &schema_check.validate {
                    ValidateBlock::Expr(v) => match pattern_cover_check {
                        None => CompiledExpr::with_validate_blocks(
                            quote! { { #schema_ts } },
                            quote! {
                                let instance = value;
                                let __path = __path.join(key_str);
                                #v
                            },
                        ),
                        Some(check) => CompiledExpr::with_validate_blocks(
                            quote! { (#check) || { #schema_ts } },
                            quote! {
                                if !(#check) {
                                    let instance = value;
                                    let __path = __path.join(key_str);
                                    #v
                                }
                            },
                        ),
                    },
                    ValidateBlock::AlwaysValid => CompiledExpr::always_true(),
                }
            }
        }
    };

    (statics, arm_body)
}

pub(crate) fn compile(
    ctx: &mut CompileContext<'_>,
    additional_properties: Option<&Value>,
    properties: Option<&Value>,
    pattern_properties: Option<&Value>,
) -> Option<CompiledExpr> {
    let additional_properties_val = additional_properties?;

    let known_props: Vec<&str> = properties
        .and_then(|v| v.as_object())
        .map(|obj| obj.keys().map(String::as_str).collect())
        .unwrap_or_default();

    let coverage = match build_pattern_coverage(ctx, pattern_properties) {
        Ok(coverage) => coverage,
        Err(error_expr) => return Some(error_expr),
    };

    let schema_path = ctx.schema_path_for_keyword("additionalProperties");
    let mut statics: Vec<TokenStream> = Vec::new();

    if !known_props.is_empty() {
        statics.push(quote! {
            static KNOWN: &[&str] = &[#(#known_props),*];
        });
    }
    statics.extend(coverage.statics.iter().cloned());
    let prefix_check = &coverage.prefix_check;
    let literal_check = &coverage.literal_check;
    let regex_check = &coverage.regex_check;

    match additional_properties_val {
        Value::Bool(false) => {
            let mut covered: Vec<TokenStream> = Vec::new();
            if let Some(check) = &prefix_check {
                covered.push(check.clone());
            }
            if let Some(check) = &literal_check {
                covered.push(check.clone());
            }
            if !known_props.is_empty() {
                covered.push(quote! { KNOWN.contains(&key_str) });
            }
            if let Some(check) = &regex_check {
                covered.push(check.clone());
            }

            if covered.is_empty() {
                Some(CompiledExpr::from_bool_expr(
                    quote! { obj.is_empty() },
                    &schema_path,
                ))
            } else {
                Some(CompiledExpr::from_bool_expr(
                    quote! {
                        {
                            #(#statics)*
                            obj.keys().all(|key| {
                                let key_str = key.as_str();
                                #(#covered)||*
                            })
                        }
                    },
                    &schema_path,
                ))
            }
        }
        Value::Bool(true) => None,
        schema => {
            let schema_check = ctx.with_schema_path_segment("additionalProperties", |ctx| {
                compile_schema(ctx, schema)
            });
            if schema_check.is_trivially_true() {
                return None;
            }
            let schema_ts = schema_check.is_valid_ts();

            let mut excluded: Vec<TokenStream> = Vec::new();
            if let Some(check) = &prefix_check {
                excluded.push(quote! { !(#check) });
            }
            if let Some(check) = &literal_check {
                excluded.push(quote! { !(#check) });
            }
            if !known_props.is_empty() {
                excluded.push(quote! { !KNOWN.contains(&key_str) });
            }
            if let Some(check) = &regex_check {
                excluded.push(quote! { !(#check) });
            }

            if excluded.is_empty() {
                match &schema_check.validate {
                    ValidateBlock::Expr(v) => Some(CompiledExpr::with_validate_blocks(
                        quote! { obj.values().all(|instance| #schema_ts) },
                        quote! {
                            for (key, value) in obj.iter() {
                                let instance = value;
                                let __path = __path.join(key.as_str());
                                #v
                            }
                        },
                    )),
                    ValidateBlock::AlwaysValid => None,
                }
            } else {
                match &schema_check.validate {
                    ValidateBlock::Expr(v) => Some(CompiledExpr::with_validate_blocks(
                        quote! {
                            {
                                #(#statics)*
                                obj.iter()
                                    .filter(|(key, _)| {
                                        let key_str = key.as_str();
                                        #(#excluded)&&*
                                    })
                                    .all(|(_, instance)| {
                                        #schema_ts
                                    })
                            }
                        },
                        quote! {
                            #(#statics)*
                            for (key, value) in obj.iter() {
                                let key_str = key.as_str();
                                if #(#excluded)&&* {
                                    let instance = value;
                                    let __path = __path.join(key_str);
                                    #v
                                }
                            }
                        },
                    )),
                    ValidateBlock::AlwaysValid => None,
                }
            }
        }
    }
}
