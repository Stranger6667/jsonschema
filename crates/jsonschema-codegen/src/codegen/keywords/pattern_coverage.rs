use std::borrow::Cow;

use proc_macro2::TokenStream;
use quote::quote;
use serde_json::Value;

use super::super::{compile_regex_match, translate_and_validate_regex, CompileContext};
use crate::codegen::CompiledExpr;

/// Emitted checks deciding whether a property key is covered by
/// `patternProperties`. Built once and shared by `additionalProperties`,
/// the wildcard properties arm, and `unevaluatedProperties`.
pub(super) struct PatternCoverage {
    pub(super) statics: Vec<TokenStream>,
    pub(super) prefix_check: Option<TokenStream>,
    pub(super) literal_check: Option<TokenStream>,
    pub(super) regex_check: Option<TokenStream>,
}

impl PatternCoverage {
    pub(super) fn checks(&self) -> Vec<TokenStream> {
        [&self.prefix_check, &self.literal_check, &self.regex_check]
            .into_iter()
            .flatten()
            .cloned()
            .collect()
    }

    /// All checks OR-combined into one expression, or `None` when empty.
    pub(super) fn combined_check(&self) -> Option<TokenStream> {
        let checks = self.checks();
        match checks.as_slice() {
            [] => None,
            [single] => Some(single.clone()),
            _ => Some(quote! { #(#checks)||* }),
        }
    }
}

/// Classify `patternProperties` keys into prefix/literal/regex buckets.
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

/// Build the coverage checks for `patternProperties`, or `Err` with the first
/// invalid-regex diagnostic.
pub(super) fn build_pattern_coverage(
    ctx: &mut CompileContext<'_>,
    pattern_properties: Option<&Value>,
) -> Result<PatternCoverage, CompiledExpr> {
    let (prefixes, literals, regex_patterns, regex_errors) =
        collect_pattern_coverage_parts(ctx, pattern_properties);
    if let Some(error_expr) = regex_errors.into_iter().next() {
        return Err(error_expr);
    }

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

    Ok(PatternCoverage {
        statics,
        prefix_check,
        literal_check,
        regex_check,
    })
}
