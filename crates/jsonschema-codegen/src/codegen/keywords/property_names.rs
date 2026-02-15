use super::super::{compile_schema, expr::ValidateBlock, CompileContext, CompiledExpr};
use quote::quote;
use serde_json::Value;

pub(crate) fn compile(ctx: &mut CompileContext<'_>, value: &Value) -> CompiledExpr {
    let schema_path = ctx.schema_path_for_keyword("propertyNames");
    let value_ty = ctx.config.backend.emit_symbols().value_ty();
    let key_as_value_ref = ctx.config.backend.key_as_value_ref(quote! { key });

    if let Value::Object(schema) = value {
        let only_string_keywords = schema.iter().all(|(k, v)| {
            matches!(k.as_str(), "minLength" | "maxLength" | "pattern" | "format")
                || (k == "type" && v.as_str() == Some("string"))
        });
        let has_string_keywords = schema.contains_key("minLength")
            || schema.contains_key("maxLength")
            || schema.contains_key("pattern")
            || schema.contains_key("format");
        if only_string_keywords && has_string_keywords {
            let string_check = ctx.with_schema_path_segment("propertyNames", |ctx| {
                super::string::compile(ctx, schema)
            });
            let is_valid_ts = string_check.is_valid_ts();
            return match (&string_check.validate, &string_check.iter_errors) {
                (ValidateBlock::Expr(v), ValidateBlock::Expr(ie)) => {
                    // Use outer `instance` (the object) for error construction to avoid
                    // temporary lifetime issues. `instance_path` stays at the object level
                    // (matching the dynamic compiler's behavior for propertyNames errors).
                    CompiledExpr::with_validate_blocks(
                        quote! { obj.keys().all(|s| { #is_valid_ts }) },
                        quote! {
                            for key in obj.keys() {
                                let s = key;
                                #v
                            }
                        },
                        quote! {
                            for key in obj.keys() {
                                let s = key;
                                #ie
                            }
                        },
                    )
                }
                (ValidateBlock::AlwaysValid, ValidateBlock::AlwaysValid) => {
                    CompiledExpr::always_true()
                }
                _ => CompiledExpr::from_bool_expr(
                    quote! { obj.keys().all(|s| { #is_valid_ts }) },
                    &schema_path,
                ),
            };
        }
    }

    let schema_check =
        ctx.with_schema_path_segment("propertyNames", |ctx| compile_schema(ctx, value));
    let is_valid_ts = schema_check.is_valid_ts();
    match (&schema_check.validate, &schema_check.iter_errors) {
        (ValidateBlock::Expr(v), ValidateBlock::Expr(ie)) => {
            // Wrap validation in a closure to avoid temporary lifetime issues.
            // The key is converted to a temporary Value::String inside the closure;
            // errors are converted to 'static via to_owned() before being returned.
            CompiledExpr::with_validate_blocks(
                quote! {
                    obj.keys().all(|key| {
                        (|instance: &#value_ty| #is_valid_ts)(#key_as_value_ref)
                    })
                },
                quote! {
                    for key in obj.keys() {
                        let __key_val: serde_json::Value = serde_json::Value::String(key.clone());
                        if let Some(__e) = (|| -> Option<jsonschema::ValidationError<'_>> {
                            let instance = &__key_val;
                            let __path = __path.clone();
                            #v
                            None
                        })().map(|e| e.to_owned()) {
                            return Some(__e);
                        }
                    }
                },
                quote! {
                    for key in obj.keys() {
                        let __key_val: serde_json::Value = serde_json::Value::String(key.clone());
                        let mut __key_errs: Vec<jsonschema::ValidationError<'_>> = Vec::new();
                        {
                            let __errors = &mut __key_errs;
                            let instance = &__key_val;
                            let __path = __path.clone();
                            #ie
                        }
                        __errors.extend(__key_errs.into_iter().map(|e| e.to_owned()));
                    }
                },
            )
        }
        (ValidateBlock::AlwaysValid, ValidateBlock::AlwaysValid) => CompiledExpr::always_true(),
        _ => CompiledExpr::from_bool_expr(
            quote! {
                obj.keys().all(|key| {
                    (|instance: &#value_ty| #is_valid_ts)(#key_as_value_ref)
                })
            },
            &schema_path,
        ),
    }
}
