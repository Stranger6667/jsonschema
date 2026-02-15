use super::super::{
    compile_regex_match, compile_schema, errors::invalid_schema_type_expression,
    expr::ValidateBlock, translate_and_validate_regex, CompileContext, CompiledExpr,
};
use quote::quote;
use serde_json::Value;

pub(crate) fn compile(ctx: &mut CompileContext<'_>, value: &Value) -> Option<CompiledExpr> {
    let Value::Object(patterns) = value else {
        return Some(invalid_schema_type_expression(value, &["object"]));
    };

    if patterns.is_empty() {
        return None;
    }

    let schema_path = ctx.schema_path_for_keyword("patternProperties");
    let mut pattern_checks = Vec::new();

    for (pattern, schema) in patterns {
        let analysis = jsonschema_regex::analyze_pattern(pattern);
        let translated_regex = match analysis {
            Some(_) => None,
            None => match translate_and_validate_regex(ctx, "patternProperties", pattern) {
                Ok(translated) => Some(translated),
                Err(error_expr) => {
                    pattern_checks.push(error_expr);
                    continue;
                }
            },
        };

        // Compile sub-schema with proper path context
        let schema_check = ctx.with_schema_path_segment("patternProperties", |ctx| {
            ctx.with_schema_path_segment(pattern, |ctx| compile_schema(ctx, schema))
        });

        if schema_check.is_trivially_true() {
            continue;
        }
        let schema_ts = schema_check.is_valid_ts();

        // Build the key match condition (compute regex match after sub-schema, since ctx is free)
        let key_matches: proc_macro2::TokenStream = match analysis {
            Some(jsonschema_regex::PatternAnalysis::Prefix(prefix)) => {
                let prefix: &str = prefix.as_ref();
                quote! { key.as_str().starts_with(#prefix) }
            }
            Some(jsonschema_regex::PatternAnalysis::Exact(exact)) => {
                let exact: &str = exact.as_ref();
                quote! { key.as_str() == #exact }
            }
            Some(jsonschema_regex::PatternAnalysis::Alternation(alts)) => {
                let alts: Vec<&str> = alts.iter().map(String::as_str).collect();
                quote! { matches!(key.as_str(), #(#alts)|*) }
            }
            None => {
                let p = translated_regex.expect("Regex translation must be present");
                let regex_check = compile_regex_match(ctx, &p, &quote! { key.as_str() });
                quote! { { #regex_check } }
            }
        };

        let check = match (&schema_check.validate, &schema_check.iter_errors) {
            (ValidateBlock::Expr(v), ValidateBlock::Expr(ie)) => {
                CompiledExpr::with_validate_blocks(
                    quote! {
                        obj.iter()
                            .filter(|(key, _)| #key_matches)
                            .all(|(_, instance)| { #schema_ts })
                    },
                    quote! {
                        for (key, value) in obj.iter() {
                            if #key_matches {
                                let instance = value;
                                let __path = __path.join(key.as_str());
                                #v
                            }
                        }
                    },
                    quote! {
                        for (key, value) in obj.iter() {
                            if #key_matches {
                                let instance = value;
                                let __path = __path.join(key.as_str());
                                #ie
                            }
                        }
                    },
                )
            }
            (ValidateBlock::AlwaysValid, ValidateBlock::AlwaysValid) => CompiledExpr::always_true(),
            _ => CompiledExpr::from_bool_expr(
                quote! {
                    obj.iter()
                        .filter(|(key, _)| #key_matches)
                        .all(|(_, instance)| { #schema_ts })
                },
                &schema_path,
            ),
        };
        pattern_checks.push(check);
    }

    if pattern_checks.is_empty() {
        None
    } else {
        Some(CompiledExpr::combine_and(pattern_checks))
    }
}
