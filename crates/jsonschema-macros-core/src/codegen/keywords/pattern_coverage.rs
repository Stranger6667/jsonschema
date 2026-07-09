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
    pub(super) checks: Vec<TokenStream>,
}

impl PatternCoverage {
    /// All checks OR-combined into one expression, or `None` when empty.
    pub(super) fn combined_check(&self) -> Option<TokenStream> {
        match self.checks.as_slice() {
            [] => None,
            [single] => Some(single.clone()),
            checks => Some(quote! { #(#checks)||* }),
        }
    }
}

/// Build the coverage checks for `patternProperties`, or `Err` with the first
/// invalid-regex diagnostic.
pub(super) fn build_pattern_coverage(
    ctx: &mut CompileContext<'_>,
    pattern_properties: Option<&Value>,
) -> Result<PatternCoverage, CompiledExpr> {
    let Some(obj) = pattern_properties.and_then(Value::as_object) else {
        return Ok(PatternCoverage {
            statics: Vec::new(),
            checks: Vec::new(),
        });
    };

    let mut prefixes: Vec<Cow<'_, str>> = Vec::new();
    let mut literals: Vec<String> = Vec::new();
    let mut regex_patterns: Vec<String> = Vec::new();
    let mut predicates: Vec<TokenStream> = Vec::new();
    for pattern in obj.keys() {
        match jsonschema_regex::analyze_pattern(pattern) {
            Some(jsonschema_regex::PatternAnalysis::Prefix(prefix)) => prefixes.push(prefix),
            Some(jsonschema_regex::PatternAnalysis::Exact(exact)) => {
                literals.push(exact.into_owned());
            }
            Some(jsonschema_regex::PatternAnalysis::Alternation(alts)) => literals.extend(alts),
            Some(jsonschema_regex::PatternAnalysis::NoWhitespace) => predicates.push(quote! {
                !key_str.chars().any(jsonschema::__private::regex::is_ecma_whitespace)
            }),
            None => {
                let regex = translate_and_validate_regex(ctx, "patternProperties", pattern)?;
                regex_patterns.push(regex);
            }
        }
    }

    let mut statics: Vec<TokenStream> = Vec::new();
    let mut checks: Vec<TokenStream> = Vec::new();
    match prefixes.as_slice() {
        [] => {}
        [prefix] => {
            let prefix: &str = prefix.as_ref();
            checks.push(quote! { key_str.starts_with(#prefix) });
        }
        _ => {
            let prefix_strs: Vec<&str> = prefixes.iter().map(Cow::as_ref).collect();
            statics.push(quote! {
                static PATTERN_PREFIXES: &[&str] = &[#(#prefix_strs),*];
            });
            checks
                .push(quote! { PATTERN_PREFIXES.iter().any(|prefix| key_str.starts_with(prefix)) });
        }
    }
    match literals.as_slice() {
        [] => {}
        [literal] => checks.push(quote! { key_str == #literal }),
        _ => {
            let literal_strs: Vec<&str> = literals.iter().map(String::as_str).collect();
            statics.push(quote! {
                static PATTERN_LITERALS: &[&str] = &[#(#literal_strs),*];
            });
            checks.push(quote! { PATTERN_LITERALS.contains(&key_str) });
        }
    }
    if !regex_patterns.is_empty() {
        let regex_checks: Vec<TokenStream> = regex_patterns
            .iter()
            .map(|pattern| compile_regex_match(ctx, pattern, &quote! { key_str }))
            .collect();
        checks.push(super::combine_or(regex_checks));
    }
    // Only `^\S*$` yields a predicate and object keys are unique, so at most one exists.
    checks.extend(predicates);

    Ok(PatternCoverage { statics, checks })
}
