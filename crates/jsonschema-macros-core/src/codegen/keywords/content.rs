use proc_macro2::TokenStream;
use quote::quote;
use serde_json::{Map, Value};

use super::super::{errors::invalid_schema_type_expression, CompileContext, CompiledExpr};

// Helper fns mirror the runtime validator's default content maps.
fn builtin_encoding_fns(name: &str) -> Option<(TokenStream, TokenStream)> {
    let fns = match name {
        "base64" => (
            quote! { jsonschema::__private::content::is_base64 },
            quote! { jsonschema::__private::content::from_base64 },
        ),
        "base64url" => (
            quote! { jsonschema::__private::content::is_base64url },
            quote! { jsonschema::__private::content::from_base64url },
        ),
        "base32" => (
            quote! { jsonschema::__private::content::is_base32 },
            quote! { jsonschema::__private::content::from_base32 },
        ),
        "base32hex" => (
            quote! { jsonschema::__private::content::is_base32hex },
            quote! { jsonschema::__private::content::from_base32hex },
        ),
        "base16" => (
            quote! { jsonschema::__private::content::is_base16 },
            quote! { jsonschema::__private::content::from_base16 },
        ),
        _ => return None,
    };
    Some(fns)
}

/// `(check, convert)` fns for an encoding: user-configured first, then builtins.
fn resolve_encoding(ctx: &CompileContext<'_>, name: &str) -> Option<(TokenStream, TokenStream)> {
    if let Some((check, convert)) = ctx.config.content_encodings.get(name) {
        return Some((check.clone(), convert.clone()));
    }
    builtin_encoding_fns(name)
}

/// Check fn for a media type: user-configured first, then builtins.
fn resolve_media_type(ctx: &CompileContext<'_>, name: &str) -> Option<TokenStream> {
    if let Some(check) = ctx.config.content_media_types.get(name) {
        return Some(check.clone());
    }
    (name == "application/json").then(|| quote! { jsonschema::__private::content::is_json })
}

pub(crate) fn compile(
    ctx: &CompileContext<'_>,
    schema: &Map<String, Value>,
) -> Option<CompiledExpr> {
    // Mirrors the runtime dispatch: `contentMediaType` owns the combined case,
    // and `contentEncoding` is skipped whenever `contentMediaType` is present.
    if let Some(media_value) = schema.get("contentMediaType") {
        let Some(media_name) = media_value.as_str() else {
            return Some(invalid_schema_type_expression(media_value, &["string"]));
        };
        let media_check = resolve_media_type(ctx, media_name)?;
        let media_path = ctx.schema_path_for_keyword("contentMediaType");
        if let Some(encoding_value) = schema.get("contentEncoding") {
            let Some(encoding_name) = encoding_value.as_str() else {
                return Some(invalid_schema_type_expression(encoding_value, &["string"]));
            };
            let (_, convert) = resolve_encoding(ctx, encoding_name)?;
            let encoding_path = ctx.schema_path_for_keyword("contentEncoding");
            Some(CompiledExpr::with_validate_blocks(
                quote! {
                    match #convert(s) {
                        Ok(Some(__decoded)) => #media_check(&__decoded),
                        _ => false,
                    }
                },
                quote! {
                    match #convert(s) {
                        Ok(Some(__decoded)) => {
                            if !#media_check(&__decoded) {
                                return Some(__err::content_media_type(
                                    #media_path, __path.into(), instance, #media_name,
                                ));
                            }
                        }
                        Ok(None) => {
                            return Some(__err::content_encoding(
                                #encoding_path, __path.into(), instance, #encoding_name,
                            ));
                        }
                        Err(__err) => return Some(__err),
                    }
                },
            ))
        } else {
            Some(CompiledExpr::with_validate_blocks(
                quote! { #media_check(s) },
                quote! {
                    if !#media_check(s) {
                        return Some(__err::content_media_type(
                            #media_path, __path.into(), instance, #media_name,
                        ));
                    }
                },
            ))
        }
    } else if let Some(encoding_value) = schema.get("contentEncoding") {
        let Some(encoding_name) = encoding_value.as_str() else {
            return Some(invalid_schema_type_expression(encoding_value, &["string"]));
        };
        let (check, _) = resolve_encoding(ctx, encoding_name)?;
        let encoding_path = ctx.schema_path_for_keyword("contentEncoding");
        Some(CompiledExpr::with_validate_blocks(
            quote! { #check(s) },
            quote! {
                if !#check(s) {
                    return Some(__err::content_encoding(
                        #encoding_path, __path.into(), instance, #encoding_name,
                    ));
                }
            },
        ))
    } else {
        None
    }
}
