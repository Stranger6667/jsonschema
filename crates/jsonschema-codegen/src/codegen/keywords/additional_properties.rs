use std::borrow::Cow;

use super::super::{
    compile_regex_match, compile_schema, expr::ValidateBlock, translate_and_validate_regex,
    CompileContext, CompiledExpr,
};
use proc_macro2::TokenStream;
use quote::quote;
use serde_json::Value;

pub(super) fn collect_pattern_coverage_parts<'a>(
    ctx: &mut CompileContext<'_>,
    pattern_properties: Option<&'a Value>,
) -> (
    Vec<Cow<'a, str>>,
    Vec<String>,
    Vec<String>,
    Vec<CompiledExpr>,
) {
    pattern_properties
        .and_then(|v| v.as_object())
        .map(|obj| {
            let mut prefixes = Vec::new();
            let mut literals = Vec::new();
            let mut regex_patterns = Vec::new();
            let mut regex_errors = Vec::new();
            for p in obj.keys() {
                match jsonschema_regex::analyze_pattern(p) {
                    Some(jsonschema_regex::PatternAnalysis::Prefix(s)) => prefixes.push(s),
                    Some(jsonschema_regex::PatternAnalysis::Exact(s)) => {
                        literals.push(s.into_owned());
                    }
                    Some(jsonschema_regex::PatternAnalysis::Alternation(alts)) => {
                        literals.extend(alts);
                    }
                    None => match translate_and_validate_regex(ctx, "patternProperties", p) {
                        Ok(regex) => regex_patterns.push(regex),
                        Err(error_expr) => regex_errors.push(error_expr),
                    },
                }
            }
            (prefixes, literals, regex_patterns, regex_errors)
        })
        .unwrap_or_default()
}

pub(super) fn compile_wildcard_arm(
    ctx: &mut CompileContext<'_>,
    additional_properties: Option<&Value>,
    pattern_properties: Option<&Value>,
) -> (Vec<TokenStream>, CompiledExpr) {
    let (prefixes, literals, regex_patterns, regex_errors) =
        collect_pattern_coverage_parts(ctx, pattern_properties);

    if let Some(error_expr) = regex_errors.into_iter().next() {
        return (Vec::new(), error_expr);
    }

    let schema_path = ctx.schema_path_for_keyword("additionalProperties");
    let mut statics: Vec<TokenStream> = Vec::new();

    let prefix_check: Option<TokenStream> = match prefixes.as_slice() {
        [] => None,
        [p] => {
            let p: &str = p.as_ref();
            Some(quote! { key_str.starts_with(#p) })
        }
        _ => {
            let prefix_strs: Vec<&str> = prefixes.iter().map(Cow::as_ref).collect();
            statics.push(quote! {
                static PATTERN_PREFIXES: &[&str] = &[#(#prefix_strs),*];
            });
            Some(quote! { PATTERN_PREFIXES.iter().any(|p| key_str.starts_with(p)) })
        }
    };
    let literal_check: Option<TokenStream> = match literals.as_slice() {
        [] => None,
        [s] => Some(quote! { key_str == #s }),
        _ => {
            let lit_strs: Vec<&str> = literals.iter().map(String::as_str).collect();
            statics.push(quote! {
                static PATTERN_LITERALS: &[&str] = &[#(#lit_strs),*];
            });
            Some(quote! { PATTERN_LITERALS.contains(&key_str) })
        }
    };
    let regex_check: Option<TokenStream> = if regex_patterns.is_empty() {
        None
    } else {
        let checks: Vec<TokenStream> = regex_patterns
            .iter()
            .map(|pattern| compile_regex_match(ctx, pattern, &quote! { key_str }))
            .collect();
        Some(super::combine_or(checks))
    };

    let checks: Vec<TokenStream> = [prefix_check, literal_check, regex_check]
        .into_iter()
        .flatten()
        .collect();
    let pattern_cover_check: Option<TokenStream> = match checks.as_slice() {
        [] => None,
        [single] => Some(single.clone()),
        _ => Some(quote! { #(#checks)||* }),
    };

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
                match (&schema_check.validate, &schema_check.iter_errors) {
                    (ValidateBlock::Expr(v), ValidateBlock::Expr(ie)) => {
                        // Bind `instance = value` and extend `__path` here so that
                        // the sub-schema validation sees the property value/path. The
                        // wildcard match arm in `build_validate_blocks` does NOT rebind
                        // these, keeping `instance` = outer object for the `false` case.
                        match pattern_cover_check {
                            None => CompiledExpr::with_validate_blocks(
                                quote! { { #schema_ts } },
                                quote! {
                                    let instance = value;
                                    let __path = __path.join(key_str);
                                    #v
                                },
                                quote! {
                                    let instance = value;
                                    let __path = __path.join(key_str);
                                    #ie
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
                                quote! {
                                    if !(#check) {
                                        let instance = value;
                                        let __path = __path.join(key_str);
                                        #ie
                                    }
                                },
                            ),
                        }
                    }
                    (ValidateBlock::AlwaysValid, ValidateBlock::AlwaysValid) => {
                        CompiledExpr::always_true()
                    }
                    _ => match pattern_cover_check {
                        None => {
                            CompiledExpr::from_bool_expr(quote! { { #schema_ts } }, &schema_path)
                        }
                        Some(check) => CompiledExpr::from_bool_expr(
                            quote! { (#check) || { #schema_ts } },
                            &schema_path,
                        ),
                    },
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

    let (prefixes, literals, regex_patterns, regex_errors) =
        collect_pattern_coverage_parts(ctx, pattern_properties);

    if let Some(error_expr) = regex_errors.into_iter().next() {
        return Some(error_expr);
    }

    let schema_path = ctx.schema_path_for_keyword("additionalProperties");
    let mut statics: Vec<TokenStream> = Vec::new();

    if !known_props.is_empty() {
        statics.push(quote! {
            static KNOWN: &[&str] = &[#(#known_props),*];
        });
    }
    let prefix_check: Option<TokenStream> = match prefixes.as_slice() {
        [] => None,
        [p] => {
            let p: &str = p.as_ref();
            Some(quote! { key_str.starts_with(#p) })
        }
        _ => {
            let prefix_strs: Vec<&str> = prefixes.iter().map(Cow::as_ref).collect();
            statics.push(quote! {
                static PATTERN_PREFIXES: &[&str] = &[#(#prefix_strs),*];
            });
            Some(quote! { PATTERN_PREFIXES.iter().any(|p| key_str.starts_with(p)) })
        }
    };
    let literal_check: Option<TokenStream> = match literals.as_slice() {
        [] => None,
        [s] => Some(quote! { key_str == #s }),
        _ => {
            let lit_strs: Vec<&str> = literals.iter().map(String::as_str).collect();
            statics.push(quote! {
                static PATTERN_LITERALS: &[&str] = &[#(#lit_strs),*];
            });
            Some(quote! { PATTERN_LITERALS.contains(&key_str) })
        }
    };
    let regex_check: Option<TokenStream> = if regex_patterns.is_empty() {
        None
    } else {
        let checks: Vec<TokenStream> = regex_patterns
            .iter()
            .map(|pattern| compile_regex_match(ctx, pattern, &quote! { key_str }))
            .collect();
        Some(super::combine_or(checks))
    };

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
                match (&schema_check.validate, &schema_check.iter_errors) {
                    (ValidateBlock::Expr(v), ValidateBlock::Expr(ie)) => {
                        Some(CompiledExpr::with_validate_blocks(
                            quote! { obj.values().all(|instance| #schema_ts) },
                            quote! {
                                for (key, value) in obj.iter() {
                                    let instance = value;
                                    let __path = __path.join(key.as_str());
                                    #v
                                }
                            },
                            quote! {
                                for (key, value) in obj.iter() {
                                    let instance = value;
                                    let __path = __path.join(key.as_str());
                                    #ie
                                }
                            },
                        ))
                    }
                    (ValidateBlock::AlwaysValid, ValidateBlock::AlwaysValid) => None,
                    _ => Some(CompiledExpr::from_bool_expr(
                        quote! { obj.values().all(|instance| #schema_ts) },
                        &schema_path,
                    )),
                }
            } else {
                match (&schema_check.validate, &schema_check.iter_errors) {
                    (ValidateBlock::Expr(v), ValidateBlock::Expr(ie)) => {
                        Some(CompiledExpr::with_validate_blocks(
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
                            quote! {
                                #(#statics)*
                                for (key, value) in obj.iter() {
                                    let key_str = key.as_str();
                                    if #(#excluded)&&* {
                                        let instance = value;
                                        let __path = __path.join(key_str);
                                        #ie
                                    }
                                }
                            },
                        ))
                    }
                    (ValidateBlock::AlwaysValid, ValidateBlock::AlwaysValid) => None,
                    _ => Some(CompiledExpr::from_bool_expr(
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
                        &schema_path,
                    )),
                }
            }
        }
    }
}
