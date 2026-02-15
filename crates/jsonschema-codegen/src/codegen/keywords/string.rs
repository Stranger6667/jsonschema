use super::super::{
    compile_format, compile_regex_match, invalid_schema_type_expression,
    parse_nonnegative_integer_keyword, supports_content_validation_keywords,
    supports_validation_vocabulary, translate_and_validate_regex, validates_formats_by_default,
    CompileContext,
};
use proc_macro2::TokenStream;
use quote::quote;
use serde_json::{Map, Value};

/// Compile string-specific keywords.
pub(in super::super) fn compile(
    ctx: &mut CompileContext<'_>,
    schema: &Map<String, Value>,
) -> TokenStream {
    let mut items = Vec::new();
    let validation_vocab_enabled = supports_validation_vocabulary(ctx);
    let min_length = if validation_vocab_enabled {
        match schema.get("minLength") {
            Some(value) => match parse_nonnegative_integer_keyword(ctx.draft, value) {
                Ok(parsed) => Some(parsed),
                Err(error) => {
                    items.push(error);
                    None
                }
            },
            None => None,
        }
    } else {
        None
    };
    let max_length = if validation_vocab_enabled {
        match schema.get("maxLength") {
            Some(value) => match parse_nonnegative_integer_keyword(ctx.draft, value) {
                Ok(parsed) => Some(parsed),
                Err(error) => {
                    items.push(error);
                    None
                }
            },
            None => None,
        }
    } else {
        None
    };
    let has_length_constraint = min_length.is_some() || max_length.is_some();

    // Length checks - calculate once and reuse
    match (min_length, max_length) {
        (Some(min), Some(max)) if min == max => {
            items.push(quote! { len == #min as usize });
        }
        (Some(min), Some(max)) => {
            items.push(quote! { len >= #min as usize });
            items.push(quote! { len <= #max as usize });
        }
        (Some(min), None) => items.push(quote! { len >= #min as usize }),
        (None, Some(max)) => items.push(quote! { len <= #max as usize }),
        (None, None) => {}
    }

    if validation_vocab_enabled {
        if let Some(pattern_value) = schema.get("pattern") {
            if let Some(pattern) = pattern_value.as_str() {
                match jsonschema_regex::analyze_pattern(pattern) {
                    Some(jsonschema_regex::PatternAnalysis::Prefix(prefix)) => {
                        let prefix: &str = prefix.as_ref();
                        items.push(quote! { s.starts_with(#prefix) });
                    }
                    Some(jsonschema_regex::PatternAnalysis::Exact(exact)) => {
                        let exact: &str = exact.as_ref();
                        items.push(quote! { s == #exact });
                    }
                    Some(jsonschema_regex::PatternAnalysis::Alternation(alts)) => {
                        let alts: Vec<&str> = alts.iter().map(String::as_str).collect();
                        let s_as_str = ctx.config.backend.string_as_str(quote! { s });
                        items.push(quote! { matches!(#s_as_str, #(#alts)|*) });
                    }
                    None => match translate_and_validate_regex(ctx, "pattern", pattern) {
                        Ok(pattern) => {
                            items.push(compile_regex_match(ctx, &pattern, &quote! { s }));
                        }
                        Err(error_expr) => items.push(error_expr),
                    },
                }
            } else {
                items.push(invalid_schema_type_expression(pattern_value, &["string"]));
            }
        }
    }

    if let Some(compiled) = schema.get("format").and_then(|v| compile_format(ctx, v)) {
        items.push(compiled);
    }

    // Content encoding/media type validation (draft 7+, enforced for optional tests)
    if supports_content_validation_keywords(ctx.draft) && validates_formats_by_default(ctx.draft) {
        let encoding = schema
            .get("contentEncoding")
            .and_then(|v| v.as_str())
            .map(str::to_ascii_lowercase);
        let media_type = schema
            .get("contentMediaType")
            .and_then(|v| v.as_str())
            .map(str::to_ascii_lowercase);

        match (encoding.as_deref(), media_type.as_deref()) {
            (Some("base64"), Some("application/json")) => {
                items.push(quote! {
                    jsonschema::keywords_helpers::content::is_valid_base64_json(s)
                });
            }
            (Some("base64"), None) => {
                items.push(quote! {
                    jsonschema::keywords_helpers::content::is_valid_base64(s)
                });
            }
            (None, Some("application/json")) => {
                items.push(quote! {
                    jsonschema::keywords_helpers::content::is_valid_json_str(s)
                });
            }
            _ => {}
        }
    }

    if items.is_empty() {
        quote! { true }
    } else {
        let combined = quote! { ( #(#items)&&* ) };

        // If we have length checks, wrap in a block to calculate length once
        if has_length_constraint {
            quote! {
                {
                    let len = s.chars().count();
                    #combined
                }
            }
        } else {
            combined
        }
    }
}
