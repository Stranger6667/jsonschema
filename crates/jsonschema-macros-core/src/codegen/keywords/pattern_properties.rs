use super::{
    super::{
        compile_regex_match, errors::invalid_schema_type_expression, expr::ValidateBlock,
        translate_and_validate_regex, CompileContext, CompiledExpr, RegexSite,
    },
    object_pass::ClusterSubschemas,
};
use crate::codegen::emit::ValueEmitter;
use quote::{format_ident, quote};
use serde_json::Value;

pub(crate) fn compile<E: ValueEmitter>(
    value: &Value,
    cluster: &ClusterSubschemas<'_>,
) -> Option<CompiledExpr> {
    let Value::Object(patterns) = value else {
        return Some(invalid_schema_type_expression(value, &["object"]));
    };

    if patterns.is_empty() {
        return None;
    }

    let mut pattern_checks = Vec::new();

    for (_, key_match, check) in &cluster.patterns {
        let (key_matches_is_valid, key_matches) = match key_match {
            Ok(condition) => (condition.is_valid.clone(), condition.located.clone()),
            Err(error_expr) => {
                pattern_checks.push(error_expr.clone());
                continue;
            }
        };

        let schema_check = check.as_ref().expect("pattern subschema precompiled");

        if schema_check.is_trivially_true() {
            continue;
        }
        let schema_is_valid = schema_check.is_valid_token_stream();

        let check = match &schema_check.validate {
            ValidateBlock::Expr(expr) => {
                let child_collect = schema_check.collect.as_token_stream();
                let entries = E::object_iter_entries(format_ident!("obj"));
                let key_as_str = E::key_as_str(format_ident!("key"));
                CompiledExpr::with_validate_and_collect_blocks(
                    quote! {
                        #entries
                            .filter(|(key, _)| #key_matches_is_valid)
                            .all(|(_, instance)| { #schema_is_valid })
                    },
                    quote! {
                        for (key, value) in #entries {
                            if #key_matches {
                                let instance = value;
                                let __path = &__path.push(#key_as_str);
                                #expr
                            }
                        }
                    },
                    quote! {
                        for (key, value) in #entries {
                            if #key_matches {
                                let instance = value;
                                let __path = &__path.push(#key_as_str);
                                #child_collect
                            }
                        }
                    },
                )
            }
            ValidateBlock::AlwaysValid => CompiledExpr::always_true(),
        };
        pattern_checks.push(check);
    }

    if pattern_checks.is_empty() {
        None
    } else {
        Some(CompiledExpr::combine_and(pattern_checks))
    }
}

/// Boolean conditions over `key.as_str()` that a key matches a `patternProperties` pattern.
#[derive(Clone)]
pub(crate) struct KeyMatch {
    /// For `is_valid` code.
    pub(crate) is_valid: proc_macro2::TokenStream,
    /// For `validate` and `collect_errors` code, where `__path` is the object's location.
    pub(crate) located: proc_macro2::TokenStream,
}

/// `Err` carries the invalid-schema expression when regex translation fails.
pub(crate) fn key_match_expr<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    pattern: &str,
) -> Result<KeyMatch, CompiledExpr> {
    let key_as_str = E::key_as_str(format_ident!("key"));
    let literal = |condition: proc_macro2::TokenStream| KeyMatch {
        is_valid: condition.clone(),
        located: condition,
    };
    match jsonschema_regex::analyze_pattern(pattern) {
        Some(jsonschema_regex::PatternAnalysis::Prefix(prefix)) => {
            let prefix: &str = prefix.as_ref();
            Ok(literal(quote! { #key_as_str.starts_with(#prefix) }))
        }
        Some(jsonschema_regex::PatternAnalysis::Exact(exact)) => {
            let exact: &str = exact.as_ref();
            Ok(literal(quote! { #key_as_str == #exact }))
        }
        Some(jsonschema_regex::PatternAnalysis::Alternation(alts)) => {
            let alts: Vec<&str> = alts.iter().map(String::as_str).collect();
            Ok(literal(quote! { matches!(#key_as_str, #(#alts)|*) }))
        }
        Some(jsonschema_regex::PatternAnalysis::NoWhitespace) => Ok(literal(
            quote! { !__re::contains_ecma_whitespace(#key_as_str) },
        )),
        None => match translate_and_validate_regex(ctx, "patternProperties", pattern) {
            Ok(translated) => {
                let schema_path = ctx.with_schema_path_segment("patternProperties", |ctx| {
                    ctx.schema_path_for_keyword(pattern)
                });
                let mut site = RegexSite {
                    schema_path: &schema_path,
                    pattern: None,
                    located: false,
                };
                let is_valid =
                    compile_regex_match(ctx, &translated, &quote! { #key_as_str }, &site);
                site.located = true;
                let located = compile_regex_match(ctx, &translated, &quote! { #key_as_str }, &site);
                Ok(KeyMatch {
                    is_valid: quote! { { #is_valid } },
                    located: quote! { { #located } },
                })
            }
            Err(error_expr) => Err(error_expr),
        },
    }
}
