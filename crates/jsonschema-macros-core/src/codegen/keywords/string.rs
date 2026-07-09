use super::{
    super::{draft::DraftExt, parse_nonnegative_integer_keyword, CompileContext, CompiledExpr},
    compile_count_limit, content, format, pattern, Limit,
};
use quote::quote;
use serde_json::{Map, Value};

/// Compile all string-specific keywords.
pub(in super::super) fn compile(
    ctx: &mut CompileContext<'_>,
    schema: &Map<String, Value>,
) -> CompiledExpr {
    let mut items: Vec<CompiledExpr> = Vec::new();
    let validation_vocab_enabled = ctx.supports_validation_vocabulary();

    // Parse min/max length together to share the `len` binding when both are present.
    let min_length = if validation_vocab_enabled {
        schema
            .get("minLength")
            .map(|value| parse_nonnegative_integer_keyword(ctx.draft, value))
    } else {
        None
    };
    let max_length = if validation_vocab_enabled {
        schema
            .get("maxLength")
            .map(|value| parse_nonnegative_integer_keyword(ctx.draft, value))
    } else {
        None
    };

    match (&min_length, &max_length) {
        (Some(Ok(0)), Some(Ok(0))) => {
            let max_path = ctx.schema_path_for_keyword("maxLength");
            items.push(CompiledExpr::from_check_and_error(
                quote! { s.is_empty() },
                quote! {
                    jsonschema::__private::error::max_length(#max_path, __path.into(), instance, 0)
                },
            ));
        }
        (Some(Ok(min)), Some(Ok(max))) => {
            let min_path = ctx.schema_path_for_keyword("minLength");
            let max_path = ctx.schema_path_for_keyword("maxLength");
            let min = proc_macro2::Literal::u64_unsuffixed(*min);
            let max = proc_macro2::Literal::u64_unsuffixed(*max);
            items.push(CompiledExpr::with_validate_blocks(
                quote! { { let len = s.chars().count(); (len as u64) >= #min && (len as u64) <= #max } },
                quote! {
                    let len = s.chars().count();
                    if (len as u64) < #min {
                        return Some(jsonschema::__private::error::min_length(
                            #min_path, __path.into(), instance, #min,
                        ));
                    }
                    if (len as u64) > #max {
                        return Some(jsonschema::__private::error::max_length(
                            #max_path, __path.into(), instance, #max,
                        ));
                    }
                }));
        }
        _ => {
            let length = quote! { s.chars().count() };
            if matches!(&min_length, Some(Ok(1))) {
                let min_path = ctx.schema_path_for_keyword("minLength");
                items.push(CompiledExpr::from_check_and_error(
                    quote! { !s.is_empty() },
                    quote! {
                        jsonschema::__private::error::min_length(#min_path, __path.into(), instance, 1)
                    },
                ));
            } else if let Some(value) = schema.get("minLength").filter(|_| validation_vocab_enabled)
            {
                items.push(compile_count_limit(
                    ctx,
                    value,
                    &length,
                    "minLength",
                    "min_length",
                    &Limit::Min,
                ));
            }
            if matches!(&max_length, Some(Ok(0))) {
                let max_path = ctx.schema_path_for_keyword("maxLength");
                items.push(CompiledExpr::from_check_and_error(
                    quote! { s.is_empty() },
                    quote! {
                        jsonschema::__private::error::max_length(#max_path, __path.into(), instance, 0)
                    },
                ));
            } else if let Some(value) = schema.get("maxLength").filter(|_| validation_vocab_enabled)
            {
                items.push(compile_count_limit(
                    ctx,
                    value,
                    &length,
                    "maxLength",
                    "max_length",
                    &Limit::Max,
                ));
            }
        }
    }

    if validation_vocab_enabled {
        if let Some(value) = schema.get("pattern") {
            items.push(pattern::compile(ctx, value));
        }
    }

    if let Some(compiled) = schema
        .get("format")
        .and_then(|value| format::compile(ctx, value))
    {
        items.push(compiled);
    }

    if ctx.draft.supports_content_validation_keywords() {
        if let Some(compiled) = content::compile(ctx, schema) {
            items.push(compiled);
        }
    }

    CompiledExpr::combine_and(items)
}
