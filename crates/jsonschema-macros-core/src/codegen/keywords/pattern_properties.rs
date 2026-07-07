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

    let mut pattern_checks = Vec::new();

    for (pattern, schema) in patterns {
        // Validate the pattern regex first so an invalid regex is reported even when the
        // sub-schema is trivial.
        let key_matches = match key_match_expr(ctx, pattern) {
            Ok(condition) => condition,
            Err(error_expr) => {
                pattern_checks.push(error_expr);
                continue;
            }
        };

        let schema_check = ctx.with_schema_path_segment("patternProperties", |ctx| {
            ctx.with_schema_path_segment(pattern, |ctx| {
                ctx.with_instance_scope(|ctx| compile_schema(ctx, schema))
            })
        });

        if schema_check.is_trivially_true() {
            continue;
        }
        let schema_is_valid = schema_check.is_valid_token_stream();

        let check = match &schema_check.validate {
            ValidateBlock::Expr(expr) => CompiledExpr::with_validate_blocks(
                quote! {
                    obj.iter()
                        .filter(|(key, _)| #key_matches)
                        .all(|(_, instance)| { #schema_is_valid })
                },
                quote! {
                    for (key, value) in obj.iter() {
                        if #key_matches {
                            let instance = value;
                            let __path = &__path.push(key.as_str());
                            #expr
                        }
                    }
                },
            ),
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

/// Boolean condition over `key.as_str()` that a key matches `pattern`.
/// `Err` carries the invalid-schema expression when regex translation fails.
pub(crate) fn key_match_expr(
    ctx: &mut CompileContext<'_>,
    pattern: &str,
) -> Result<proc_macro2::TokenStream, CompiledExpr> {
    match jsonschema_regex::analyze_pattern(pattern) {
        Some(jsonschema_regex::PatternAnalysis::Prefix(prefix)) => {
            let prefix: &str = prefix.as_ref();
            Ok(quote! { key.as_str().starts_with(#prefix) })
        }
        Some(jsonschema_regex::PatternAnalysis::Exact(exact)) => {
            let exact: &str = exact.as_ref();
            Ok(quote! { key.as_str() == #exact })
        }
        Some(jsonschema_regex::PatternAnalysis::Alternation(alts)) => {
            let alts: Vec<&str> = alts.iter().map(String::as_str).collect();
            Ok(quote! { matches!(key.as_str(), #(#alts)|*) })
        }
        Some(jsonschema_regex::PatternAnalysis::NoWhitespace) => Ok(
            quote! { !key.as_str().chars().any(jsonschema::__private::regex::is_ecma_whitespace) },
        ),
        None => match translate_and_validate_regex(ctx, "patternProperties", pattern) {
            Ok(translated) => {
                let regex_check = compile_regex_match(ctx, &translated, &quote! { key.as_str() });
                Ok(quote! { { #regex_check } })
            }
            Err(error_expr) => Err(error_expr),
        },
    }
}
