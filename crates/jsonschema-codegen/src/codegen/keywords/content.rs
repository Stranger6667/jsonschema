use super::{
    super::{draft::supports_content_validation_keywords, CompileContext, CompiledExpr},
    format,
};
use quote::quote;
use serde_json::{Map, Value};

pub(crate) fn compile(
    ctx: &CompileContext<'_>,
    schema: &Map<String, Value>,
) -> Option<CompiledExpr> {
    if !supports_content_validation_keywords(ctx.draft)
        || !format::validates_formats_by_default(ctx.draft)
    {
        return None;
    }
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
            let enc_path = ctx.schema_path_for_keyword("contentEncoding");
            let media_path = ctx.schema_path_for_keyword("contentMediaType");
            Some(CompiledExpr::with_validate_blocks(
                quote! { jsonschema::keywords_helpers::content::is_valid_base64_json(s) },
                quote! {
                    if let Some(fail_kw) = jsonschema::keywords_helpers::content::check_base64_json(s) {
                        let sp: &str = if fail_kw == "contentEncoding" { #enc_path } else { #media_path };
                        return Some(jsonschema::keywords_helpers::error::false_schema(
                            sp, __path.clone(), instance,
                        ));
                    }
                },
                quote! {
                    if let Some(fail_kw) = jsonschema::keywords_helpers::content::check_base64_json(s) {
                        let sp: &str = if fail_kw == "contentEncoding" { #enc_path } else { #media_path };
                        __errors.push(jsonschema::keywords_helpers::error::false_schema(
                            sp, __path.clone(), instance,
                        ));
                    }
                },
            ))
        }
        (Some("base64"), None) => {
            let schema_path = ctx.schema_path_for_keyword("contentEncoding");
            Some(CompiledExpr::from_bool_expr(
                quote! { jsonschema::keywords_helpers::content::is_valid_base64(s) },
                &schema_path,
            ))
        }
        (None, Some("application/json")) => {
            let schema_path = ctx.schema_path_for_keyword("contentMediaType");
            Some(CompiledExpr::from_bool_expr(
                quote! { jsonschema::keywords_helpers::content::is_valid_json_str(s) },
                &schema_path,
            ))
        }
        _ => None,
    }
}
