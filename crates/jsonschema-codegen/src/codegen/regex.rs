use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::context::{CompileContext, PatternEngineConfig};

use super::errors::invalid_regex_expression;

pub(super) fn translate_and_validate_regex(
    ctx: &CompileContext<'_>,
    keyword: &str,
    pattern: &str,
) -> Result<String, TokenStream> {
    let Ok(translated) = jsonschema_regex::to_rust_regex(pattern) else {
        return Err(invalid_regex_expression(keyword, pattern));
    };
    let translated = translated.into_owned();
    let valid = match ctx.config.pattern_options {
        PatternEngineConfig::FancyRegex {
            backtrack_limit,
            size_limit,
            dfa_size_limit,
        } => {
            let mut builder = fancy_regex::RegexBuilder::new(&translated);
            if let Some(limit) = backtrack_limit {
                builder.backtrack_limit(limit);
            }
            if let Some(limit) = size_limit {
                builder.delegate_size_limit(limit);
            }
            if let Some(limit) = dfa_size_limit {
                builder.delegate_dfa_size_limit(limit);
            }
            builder.build().is_ok()
        }
        PatternEngineConfig::Regex {
            size_limit,
            dfa_size_limit,
        } => {
            let mut builder = regex::RegexBuilder::new(&translated);
            if let Some(limit) = size_limit {
                builder.size_limit(limit);
            }
            if let Some(limit) = dfa_size_limit {
                builder.dfa_size_limit(limit);
            }
            builder.build().is_ok()
        }
    };
    if !valid {
        return Err(invalid_regex_expression(keyword, pattern));
    }
    Ok(translated)
}

fn get_or_create_regex_helper(ctx: &mut CompileContext<'_>, pattern: &str) -> proc_macro2::Ident {
    if let Some(name) = ctx.regex_to_helper.get(pattern) {
        return format_ident!("{}", name);
    }
    let name = format!("__jsonschema_regex_{}", ctx.regex_counter);
    ctx.regex_counter += 1;
    ctx.regex_to_helper
        .insert(pattern.to_string(), name.clone());
    ctx.regex_helpers.push((name.clone(), pattern.to_string()));
    format_ident!("{}", name)
}

pub(super) fn compile_regex_match(
    ctx: &mut CompileContext<'_>,
    pattern: &str,
    subject: &TokenStream,
) -> TokenStream {
    let helper = get_or_create_regex_helper(ctx, pattern);
    quote! {
        #helper(#subject)
    }
}
