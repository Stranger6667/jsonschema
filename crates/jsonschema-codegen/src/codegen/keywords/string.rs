use super::{
    super::{
        parse_nonnegative_integer_keyword, supports_content_validation_keywords,
        supports_validation_vocabulary, CompileContext, CompiledExpr,
    },
    content, format, max_length, min_length, pattern,
};
use quote::quote;
use serde_json::{Map, Value};

/// Compile all string-specific keywords.
pub(in super::super) fn compile(
    ctx: &mut CompileContext<'_>,
    schema: &Map<String, Value>,
) -> CompiledExpr {
    let mut items: Vec<CompiledExpr> = Vec::new();
    let validation_vocab_enabled = supports_validation_vocabulary(ctx);

    // Parse min/max length together to share the `len` binding when both are present.
    let min_val = if validation_vocab_enabled {
        schema
            .get("minLength")
            .map(|v| parse_nonnegative_integer_keyword(ctx.draft, v))
    } else {
        None
    };
    let max_val = if validation_vocab_enabled {
        schema
            .get("maxLength")
            .map(|v| parse_nonnegative_integer_keyword(ctx.draft, v))
    } else {
        None
    };

    match (&min_val, &max_val) {
        (Some(Ok(min)), Some(Ok(max))) if min == max => {
            let min_path = ctx.schema_path_for_keyword("minLength");
            let max_path = ctx.schema_path_for_keyword("maxLength");
            items.push(CompiledExpr::with_validate_blocks(
                quote! { { let len = s.chars().count(); len == #min as usize } },
                quote! {
                    let len = s.chars().count();
                    if len < #min as usize {
                        return Some(jsonschema::keywords_helpers::error::min_length(
                            #min_path, __path.clone(), instance, #min,
                        ));
                    }
                    if len > #min as usize {
                        return Some(jsonschema::keywords_helpers::error::max_length(
                            #max_path, __path.clone(), instance, #min,
                        ));
                    }
                },
                quote! {
                    let len = s.chars().count();
                    if len < #min as usize {
                        __errors.push(jsonschema::keywords_helpers::error::min_length(
                            #min_path, __path.clone(), instance, #min,
                        ));
                    }
                    if len > #min as usize {
                        __errors.push(jsonschema::keywords_helpers::error::max_length(
                            #max_path, __path.clone(), instance, #min,
                        ));
                    }
                },
            ));
        }
        (Some(Ok(min)), Some(Ok(max))) => {
            let min_path = ctx.schema_path_for_keyword("minLength");
            let max_path = ctx.schema_path_for_keyword("maxLength");
            items.push(CompiledExpr::with_validate_blocks(
                quote! { { let len = s.chars().count(); len >= #min as usize && len <= #max as usize } },
                quote! {
                    let len = s.chars().count();
                    if len < #min as usize {
                        return Some(jsonschema::keywords_helpers::error::min_length(
                            #min_path, __path.clone(), instance, #min,
                        ));
                    }
                    if len > #max as usize {
                        return Some(jsonschema::keywords_helpers::error::max_length(
                            #max_path, __path.clone(), instance, #max,
                        ));
                    }
                },
                quote! {
                    let len = s.chars().count();
                    if len < #min as usize {
                        __errors.push(jsonschema::keywords_helpers::error::min_length(
                            #min_path, __path.clone(), instance, #min,
                        ));
                    }
                    if len > #max as usize {
                        __errors.push(jsonschema::keywords_helpers::error::max_length(
                            #max_path, __path.clone(), instance, #max,
                        ));
                    }
                },
            ));
        }
        _ => {
            if let Some(v) = schema.get("minLength").filter(|_| validation_vocab_enabled) {
                items.push(
                    ctx.with_schema_path_segment("minLength", |ctx| min_length::compile(ctx, v)),
                );
            }
            if let Some(v) = schema.get("maxLength").filter(|_| validation_vocab_enabled) {
                items.push(
                    ctx.with_schema_path_segment("maxLength", |ctx| max_length::compile(ctx, v)),
                );
            }
        }
    }

    if validation_vocab_enabled {
        if let Some(v) = schema.get("pattern") {
            items.push(pattern::compile(ctx, v));
        }
    }

    if let Some(compiled) = schema.get("format").and_then(|v| format::compile(ctx, v)) {
        items.push(compiled);
    }

    if supports_content_validation_keywords(ctx.draft) {
        if let Some(compiled) = content::compile(ctx, schema) {
            items.push(compiled);
        }
    }

    CompiledExpr::combine_and(items)
}
