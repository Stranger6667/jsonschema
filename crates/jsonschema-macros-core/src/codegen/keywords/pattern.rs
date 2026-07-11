use super::super::{
    compile_regex_match, errors::invalid_schema_type_expression, translate_and_validate_regex,
    CompileContext, CompiledExpr,
};
use proc_macro2::TokenStream;
use quote::quote;
use serde_json::Value;

fn pattern_check(schema_path: &str, pattern: &str, check: TokenStream) -> CompiledExpr {
    let validate = quote! {
        if !(#check) {
            return Some(__err::pattern(
                #schema_path, __path.into(), instance, #pattern,
            ));
        }
    };
    CompiledExpr::with_validate_blocks(check, validate)
}

pub(crate) fn compile(ctx: &mut CompileContext<'_>, value: &Value) -> CompiledExpr {
    let Some(pattern) = value.as_str() else {
        return invalid_schema_type_expression(value, &["string"]);
    };
    let schema_path = ctx.schema_path_for_keyword("pattern");
    match jsonschema_regex::analyze_pattern(pattern) {
        Some(jsonschema_regex::PatternAnalysis::Prefix(prefix)) => {
            let prefix: &str = prefix.as_ref();
            pattern_check(&schema_path, pattern, quote! { s.starts_with(#prefix) })
        }
        Some(jsonschema_regex::PatternAnalysis::Exact(exact)) => {
            let exact: &str = exact.as_ref();
            pattern_check(&schema_path, pattern, quote! { s == #exact })
        }
        Some(jsonschema_regex::PatternAnalysis::Alternation(alts)) => {
            let alts: Vec<&str> = alts.iter().map(String::as_str).collect();
            let instance_as_str = crate::codegen::emit_serde::string_as_str(quote! { s });
            pattern_check(
                &schema_path,
                pattern,
                quote! { matches!(#instance_as_str, #(#alts)|*) },
            )
        }
        Some(jsonschema_regex::PatternAnalysis::NoWhitespace) => pattern_check(
            &schema_path,
            pattern,
            quote! { !s.chars().any(jsonschema::__private::regex::is_ecma_whitespace) },
        ),
        None => match translate_and_validate_regex(ctx, "pattern", pattern) {
            Ok(regex) => pattern_check(
                &schema_path,
                pattern,
                compile_regex_match(ctx, &regex, &quote! { s }),
            ),
            Err(error) => error,
        },
    }
}
