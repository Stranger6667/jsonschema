use super::{
    super::{compile_schema, expr::ValidateBlock, CompileContext, CompiledExpr},
    pattern_properties::key_match_expr,
};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use serde_json::{Map, Value};

pub(crate) struct ClusterSubschemas<'a> {
    pub(crate) properties: Vec<(&'a str, CompiledExpr)>,
    pub(crate) patterns: Vec<(
        &'a str,
        Result<TokenStream, CompiledExpr>,
        Option<CompiledExpr>,
    )>,
    pub(crate) additional: Option<CompiledExpr>,
}

pub(crate) fn compile_cluster_subschemas<'a>(
    ctx: &mut CompileContext<'_>,
    properties: Option<&'a Map<String, Value>>,
    pattern_properties: Option<&'a Value>,
    additional_properties: Option<&'a Value>,
) -> ClusterSubschemas<'a> {
    let additional = match additional_properties {
        Some(schema @ Value::Object(_)) => {
            Some(ctx.with_schema_path_segment("additionalProperties", |ctx| {
                ctx.with_instance_scope(|ctx| compile_schema(ctx, schema))
            }))
        }
        _ => None,
    };
    let mut compiled_properties = Vec::new();
    if let Some(properties) = properties {
        for (name, subschema) in properties {
            let check = ctx.with_schema_path_segment("properties", |ctx| {
                ctx.with_schema_path_segment(name, |ctx| {
                    ctx.with_instance_scope(|ctx| compile_schema(ctx, subschema))
                })
            });
            compiled_properties.push((name.as_str(), check));
        }
    }
    let mut patterns = Vec::new();
    if let Some(Value::Object(pattern_map)) = pattern_properties {
        for (pattern, subschema) in pattern_map {
            let key_match = key_match_expr(ctx, pattern);
            let check = key_match.is_ok().then(|| {
                ctx.with_schema_path_segment("patternProperties", |ctx| {
                    ctx.with_schema_path_segment(pattern, |ctx| {
                        ctx.with_instance_scope(|ctx| compile_schema(ctx, subschema))
                    })
                })
            });
            patterns.push((pattern.as_str(), key_match, check));
        }
    }
    ClusterSubschemas {
        properties: compiled_properties,
        patterns,
        additional,
    }
}

/// Single-pass `validate` for the object cluster: per key, validate the matching property, then
/// matching patterns, then `additionalProperties` for keys covered by neither. Returns `None` when
/// there is no `patternProperties` to merge or a pattern regex is invalid.
pub(crate) fn compile_validate(
    ctx: &mut CompileContext<'_>,
    cluster: &ClusterSubschemas<'_>,
    additional_properties: Option<&Value>,
) -> Option<TokenStream> {
    if cluster.patterns.is_empty() {
        return None;
    }
    let additional_properties_path = ctx.schema_path_for_keyword("additionalProperties");

    let (additional_properties_fallback, track_covered): (TokenStream, bool) =
        match additional_properties {
            Some(Value::Bool(false)) => (
                quote! {
                    if !covered {
                        return Some(jsonschema::__private::error::additional_properties(
                            #additional_properties_path, __path.into(), instance, vec![key.clone()],
                        ));
                    }
                },
                true,
            ),
            Some(Value::Object(_)) => {
                let check = cluster
                    .additional
                    .as_ref()
                    .expect("additionalProperties schema precompiled");
                match &check.validate {
                    ValidateBlock::Expr(expr) => (
                        quote! {
                            if !covered {
                                let instance = value;
                                let __path = &__path.push(key_str);
                                #expr
                            }
                        },
                        true,
                    ),
                    ValidateBlock::AlwaysValid => (quote! {}, false),
                }
            }
            _ => return None,
        };
    let set_covered = if track_covered {
        quote! { covered = true; }
    } else {
        quote! {}
    };
    let covered_decl = if track_covered {
        quote! { let mut covered = false; }
    } else {
        quote! {}
    };

    let mut match_arms: Vec<TokenStream> = Vec::new();
    for (name_str, check) in &cluster.properties {
        match &check.validate {
            ValidateBlock::Expr(expr) => match_arms.push(quote! {
                #name_str => {
                    let instance = value;
                    let __path = &__path.push(#name_str);
                    #expr
                    #set_covered
                }
            }),
            ValidateBlock::AlwaysValid => match_arms.push(quote! {
                #name_str => { #set_covered }
            }),
        }
    }

    let mut pattern_checks: Vec<TokenStream> = Vec::new();
    for (_, key_match, check) in &cluster.patterns {
        let Ok(key_match) = key_match else {
            return None;
        };
        let check = check.as_ref().expect("pattern subschema precompiled");
        match &check.validate {
            ValidateBlock::Expr(expr) => pattern_checks.push(quote! {
                if #key_match {
                    let instance = value;
                    let __path = &__path.push(key_str);
                    #expr
                    #set_covered
                }
            }),
            ValidateBlock::AlwaysValid => {
                if track_covered {
                    pattern_checks.push(quote! { if #key_match { #set_covered } });
                }
            }
        }
    }

    let match_block = if match_arms.is_empty() {
        quote! {}
    } else {
        quote! { match key_str { #(#match_arms)* _ => {} } }
    };

    Some(quote! {
        for (key, value) in obj.iter() {
            let key_str = key.as_str();
            #covered_decl
            #match_block
            #(#pattern_checks)*
            #additional_properties_fallback
        }
    })
}

/// Single-pass `is_valid` for the object cluster (`properties` + `patternProperties` +
/// `additionalProperties` + `required`). Returns `None` when no merge is warranted or a pattern
/// regex is invalid.
pub(crate) fn compile_is_valid(
    cluster: &ClusterSubschemas<'_>,
    additional_properties: Option<&Value>,
    required_names: &[&str],
) -> Option<TokenStream> {
    let has_properties = !cluster.properties.is_empty();
    let has_patterns = !cluster.patterns.is_empty();
    // Merging is worthwhile when patternProperties is present, or when `required`
    // can piggyback on a pass that already scans every key.
    if !has_patterns && required_names.is_empty() {
        return None;
    }

    // additionalProperties fallback, applied only to keys covered by neither a property
    // nor a pattern.
    let additional_properties_check: Option<TokenStream> = match additional_properties {
        Some(Value::Bool(false)) => Some(quote! { return false; }),
        Some(Value::Object(_)) => {
            let check = cluster
                .additional
                .as_ref()
                .expect("additionalProperties schema precompiled");
            if check.is_trivially_true() {
                None
            } else {
                let is_valid = check.is_valid_token_stream();
                Some(quote! { if !(#is_valid) { return false; } })
            }
        }
        _ => None,
    };
    if !has_patterns && !has_properties && additional_properties_check.is_none() {
        return None;
    }
    let track_covered = additional_properties_check.is_some();
    let set_covered = if track_covered {
        quote! { covered = true; }
    } else {
        quote! {}
    };

    let req_idents: Vec<(&str, proc_macro2::Ident)> = required_names
        .iter()
        .enumerate()
        .map(|(i, name)| (*name, format_ident!("__req_{}", i)))
        .collect();
    let req_ident = |name: &str| {
        req_idents
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, id)| id.clone())
    };

    let mut match_arms: Vec<TokenStream> = Vec::new();
    for (name_str, check) in &cluster.properties {
        let set_req = req_ident(name_str).map(|id| quote! { #id = true; });
        if check.is_trivially_true() {
            match_arms.push(quote! { #name_str => { #set_req #set_covered } });
        } else {
            let is_valid = check.is_valid_token_stream();
            match_arms.push(quote! {
                #name_str => { if !(#is_valid) { return false; } #set_req #set_covered }
            });
        }
    }
    for name in required_names {
        let in_props = cluster
            .properties
            .iter()
            .any(|(prop_name, _)| prop_name == name);
        if !in_props {
            let id = req_ident(name).expect("required ident exists");
            match_arms.push(quote! { #name => { #id = true; } });
        }
    }

    let mut pattern_checks: Vec<TokenStream> = Vec::new();
    for (_, key_match, check) in &cluster.patterns {
        let Ok(key_match) = key_match else {
            return None;
        };
        let check = check.as_ref().expect("pattern subschema precompiled");
        if check.is_trivially_true() {
            if track_covered {
                pattern_checks.push(quote! { if #key_match { #set_covered } });
            }
        } else {
            let is_valid = check.is_valid_token_stream();
            pattern_checks.push(quote! {
                if #key_match { if !(#is_valid) { return false; } #set_covered }
            });
        }
    }

    let covered_decl = if track_covered {
        quote! { let mut covered = false; }
    } else {
        quote! {}
    };
    let additional_properties_fallback = if let Some(check) = &additional_properties_check {
        quote! { if !covered { #check } }
    } else {
        quote! {}
    };
    let match_block = if match_arms.is_empty() {
        quote! {}
    } else {
        quote! { match key.as_str() { #(#match_arms)* _ => {} } }
    };
    let req_decls = req_idents
        .iter()
        .map(|(_, id)| quote! { let mut #id = false; });
    let req_checks = req_idents.iter().map(|(_, id)| quote! { #id });

    let pass = crate::codegen::emit_serde::object_iter_all(
        quote! { obj },
        quote! {
            #covered_decl
            #match_block
            #(#pattern_checks)*
            #additional_properties_fallback
            true
        },
    );

    Some(quote! {
        {
            #(#req_decls)*
            let __object_pass = #pass;
            __object_pass #(&& #req_checks)*
        }
    })
}
