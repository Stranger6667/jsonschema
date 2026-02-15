use super::super::{
    compile_regex_match, errors::invalid_schema_type_expression, translate_and_validate_regex,
    CompileContext, CompiledExpr,
};
use quote::quote;
use serde_json::Value;

pub(crate) fn compile(ctx: &mut CompileContext<'_>, value: &Value) -> CompiledExpr {
    let Some(pattern) = value.as_str() else {
        return invalid_schema_type_expression(value, &["string"]);
    };
    let schema_path = ctx.schema_path_for_keyword("pattern");
    match jsonschema_regex::analyze_pattern(pattern) {
        Some(jsonschema_regex::PatternAnalysis::Prefix(prefix)) => {
            let prefix: &str = prefix.as_ref();
            CompiledExpr::from_bool_expr(quote! { s.starts_with(#prefix) }, &schema_path)
        }
        Some(jsonschema_regex::PatternAnalysis::Exact(exact)) => {
            let exact: &str = exact.as_ref();
            CompiledExpr::from_bool_expr(quote! { s == #exact }, &schema_path)
        }
        Some(jsonschema_regex::PatternAnalysis::Alternation(alts)) => {
            let alts: Vec<&str> = alts.iter().map(String::as_str).collect();
            let s_as_str = ctx.config.backend.string_as_str(quote! { s });
            CompiledExpr::from_bool_expr(quote! { matches!(#s_as_str, #(#alts)|*) }, &schema_path)
        }
        None => match translate_and_validate_regex(ctx, "pattern", pattern) {
            Ok(p) => CompiledExpr::from_bool_expr(
                compile_regex_match(ctx, &p, &quote! { s }),
                &schema_path,
            ),
            Err(e) => e,
        },
    }
}
