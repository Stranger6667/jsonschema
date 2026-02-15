use proc_macro2::TokenStream;
use quote::quote;
use referencing::Draft;
use serde_json::Value;

use super::super::{
    draft::{
        supports_draft201909_plus_formats, supports_draft6_plus_formats,
        supports_draft7_plus_formats,
    },
    errors::invalid_schema_type_expression,
    CompileContext, CompiledExpr,
};

pub(crate) fn validates_formats_by_default(draft: Draft) -> bool {
    matches!(draft, Draft::Draft4 | Draft::Draft6 | Draft::Draft7)
}

fn compile_email_options_argument(ctx: &CompileContext<'_>) -> TokenStream {
    let Some(options) = ctx.config.email_options else {
        return quote! { None };
    };

    let mut expr = quote! { jsonschema::EmailOptions::default() };
    if let Some(minimum_sub_domains) = options.minimum_sub_domains {
        expr = quote! { #expr.with_minimum_sub_domains(#minimum_sub_domains) };
    }
    if options.no_minimum_sub_domains {
        expr = quote! { #expr.with_no_minimum_sub_domains() };
    }
    if options.required_tld {
        expr = quote! { #expr.with_required_tld() };
    }
    if let Some(allow_domain_literal) = options.allow_domain_literal {
        expr = if allow_domain_literal {
            quote! { #expr.with_domain_literal() }
        } else {
            quote! { #expr.without_domain_literal() }
        };
    }
    if let Some(allow_display_text) = options.allow_display_text {
        expr = if allow_display_text {
            quote! { #expr.with_display_text() }
        } else {
            quote! { #expr.without_display_text() }
        };
    }

    quote! { Some(&(#expr)) }
}

fn compile_builtin_format_check(
    ctx: &CompileContext<'_>,
    format_name: &str,
) -> Option<TokenStream> {
    let draft = ctx.draft;
    match format_name {
        "date" => Some(quote! { jsonschema::keywords_helpers::format::is_valid_date(s) }),
        "date-time" => Some(quote! { jsonschema::keywords_helpers::format::is_valid_datetime(s) }),
        "duration" if supports_draft201909_plus_formats(draft) => {
            Some(quote! { jsonschema::keywords_helpers::format::is_valid_duration(s) })
        }
        "email" => {
            let options = compile_email_options_argument(ctx);
            Some(quote! {
                jsonschema::keywords_helpers::format::is_valid_email_with_options(s, #options)
            })
        }
        "hostname" if matches!(draft, Draft::Draft4 | Draft::Draft6) => {
            Some(quote! { jsonschema::keywords_helpers::format::is_valid_hostname_rfc1034(s) })
        }
        "hostname" => Some(quote! { jsonschema::keywords_helpers::format::is_valid_hostname(s) }),
        "idn-email" => {
            let options = compile_email_options_argument(ctx);
            Some(quote! {
                jsonschema::keywords_helpers::format::is_valid_idn_email_with_options(s, #options)
            })
        }
        "idn-hostname" if supports_draft7_plus_formats(draft) => {
            Some(quote! { jsonschema::keywords_helpers::format::is_valid_idn_hostname(s) })
        }
        "ipv4" => Some(quote! { jsonschema::keywords_helpers::format::is_valid_ipv4(s) }),
        "ipv6" => Some(quote! { jsonschema::keywords_helpers::format::is_valid_ipv6(s) }),
        "iri" if supports_draft7_plus_formats(draft) => {
            Some(quote! { jsonschema::keywords_helpers::format::is_valid_iri(s) })
        }
        "iri-reference" if supports_draft7_plus_formats(draft) => {
            Some(quote! { jsonschema::keywords_helpers::format::is_valid_iri_reference(s) })
        }
        "json-pointer" if supports_draft6_plus_formats(draft) => {
            Some(quote! { jsonschema::keywords_helpers::format::is_valid_json_pointer(s) })
        }
        "regex" => Some(quote! { jsonschema::keywords_helpers::format::is_valid_regex(s) }),
        "relative-json-pointer" if supports_draft7_plus_formats(draft) => {
            Some(quote! { jsonschema::keywords_helpers::format::is_valid_relative_json_pointer(s) })
        }
        "time" => Some(quote! { jsonschema::keywords_helpers::format::is_valid_time(s) }),
        "uri" => Some(quote! { jsonschema::keywords_helpers::format::is_valid_uri(s) }),
        "uri-reference" if supports_draft6_plus_formats(draft) => {
            Some(quote! { jsonschema::keywords_helpers::format::is_valid_uri_reference(s) })
        }
        "uri-template" if supports_draft6_plus_formats(draft) => {
            Some(quote! { jsonschema::keywords_helpers::format::is_valid_uri_template(s) })
        }
        "uuid" if supports_draft201909_plus_formats(draft) => {
            Some(quote! { jsonschema::keywords_helpers::format::is_valid_uuid(s) })
        }
        _ => None,
    }
}

pub(crate) fn format_emits_assertion(ctx: &CompileContext<'_>, value: &Value) -> bool {
    let Some(format_name) = value.as_str() else {
        return true;
    };

    let should_validate = ctx
        .config
        .validate_formats
        .unwrap_or_else(|| validates_formats_by_default(ctx.draft));
    if !should_validate {
        return false;
    }

    if ctx.config.custom_formats.contains_key(format_name) {
        return true;
    }
    if compile_builtin_format_check(ctx, format_name).is_some() {
        return true;
    }
    !ctx.config.ignore_unknown_formats
}

/// Compile the "format" keyword.
pub(crate) fn compile(ctx: &mut CompileContext<'_>, value: &Value) -> Option<CompiledExpr> {
    let Some(format_name) = value.as_str() else {
        return Some(invalid_schema_type_expression(value, &["string"]));
    };

    let should_validate = ctx
        .config
        .validate_formats
        .unwrap_or_else(|| validates_formats_by_default(ctx.draft));
    if !should_validate {
        return None;
    }

    let schema_path = ctx.schema_path_for_keyword("format");

    if let Some(custom_call_path) = ctx.config.custom_formats.get(format_name) {
        return Some(CompiledExpr::from_bool_expr(
            quote! { #custom_call_path(s) },
            &schema_path,
        ));
    }

    if let Some(validation_call) = compile_builtin_format_check(ctx, format_name) {
        return Some(CompiledExpr::from_bool_expr(validation_call, &schema_path));
    }

    if ctx.config.ignore_unknown_formats {
        None
    } else {
        let message = format!(
            "Unknown format: '{format_name}'. Adjust configuration to ignore unrecognized formats"
        );
        Some(CompiledExpr::from(quote! {{
            compile_error!(#message);
            false
        }}))
    }
}
