use super::super::{
    compile_schema, expr::ValidateBlock, refs::resolve_lone_top_level_ref, CompileContext,
    CompiledExpr,
};
use quote::quote;
use serde_json::Value;
use std::borrow::Cow;

pub(crate) fn compile(ctx: &mut CompileContext<'_>, value: &Value) -> CompiledExpr {
    if value == &Value::Bool(false) {
        let schema_path = ctx.schema_path_for_keyword("propertyNames");
        // Like the runtime: a non-empty object fails, and the error is reported against the
        // whole object, not individual keys.
        return CompiledExpr::from_bool_expr(quote! { obj.is_empty() }, &schema_path);
    }

    let value_ty = crate::codegen::emit_serde::value_ty();
    let key_as_value_ref = crate::codegen::emit_serde::key_as_value_ref(quote! { key });

    let resolved = resolve_lone_top_level_ref(ctx, value);
    if let Value::Object(schema) = resolved.as_ref() {
        let only_string_keywords = schema.iter().all(|(keyword, value)| {
            matches!(
                keyword.as_str(),
                "minLength" | "maxLength" | "pattern" | "format"
            ) || (keyword == "type" && value.as_str() == Some("string"))
        });
        if only_string_keywords {
            let string_check = ctx.with_schema_path_segment("propertyNames", |ctx| {
                super::string::compile(ctx, schema)
            });
            // Property names are always strings, so a check without string
            // constraints cannot fail.
            if string_check.is_trivially_true() {
                return CompiledExpr::always_true();
            }
            // A hopped `$ref` target must keep the shared fn for `validate`
            // error paths; only inline schemas take the string fast path.
            if matches!(resolved, Cow::Borrowed(_)) {
                let is_valid = string_check.is_valid_token_stream();
                // Report each offending property name as the error instance (like the runtime),
                // while keeping the fast `is_valid` scan and object-level instance path.
                return match &string_check.validate {
                    ValidateBlock::Expr(expr) => {
                        let child_collect = string_check.collect.as_token_stream();
                        CompiledExpr::with_validate_and_collect_blocks(
                            quote! { obj.keys().all(|s| { #is_valid }) },
                            quote! {
                                for key in obj.keys() {
                                    let s = key;
                                    if !(#is_valid) {
                                        let __key_val: serde_json::Value = serde_json::Value::String(key.clone());
                                        let instance = &__key_val;
                                        if let Some(__e) = (|| -> Option<jsonschema::ValidationError<'_>> {
                                            #expr
                                            None
                                        })().map(|e| e.to_owned()) {
                                            return Some(__e);
                                        }
                                    }
                                }
                            },
                            collect_property_name_errors(&child_collect, true),
                        )
                    }
                    ValidateBlock::AlwaysValid => CompiledExpr::always_true(),
                };
            }
        }
    }

    let schema_check = ctx.with_schema_path_segment("propertyNames", |ctx| {
        ctx.with_instance_scope(|ctx| compile_schema(ctx, value))
    });
    let is_valid = schema_check.is_valid_token_stream();
    // Closure avoids temporary lifetime issues: the key becomes a temporary Value::String
    // inside, and errors are made 'static via to_owned() before return.
    match &schema_check.validate {
        ValidateBlock::Expr(expr) => {
            let child_collect = schema_check.collect.as_token_stream();
            CompiledExpr::with_validate_and_collect_blocks(
                quote! {
                    obj.keys().all(|key| {
                        (|instance: &#value_ty| #is_valid)(#key_as_value_ref)
                    })
                },
                quote! {
                    for key in obj.keys() {
                        let __key_val: serde_json::Value = serde_json::Value::String(key.clone());
                        if let Some(__e) = (|| -> Option<jsonschema::ValidationError<'_>> {
                            let instance = &__key_val;
                            #expr
                            None
                        })().map(|e| e.to_owned()) {
                            return Some(__e);
                        }
                    }
                },
                collect_property_name_errors(&child_collect, false),
            )
        }
        ValidateBlock::AlwaysValid => CompiledExpr::always_true(),
    }
}

/// Per-key `collect` loop: each key becomes a temporary `Value::String`, the child collects into a
/// scratch vec, and every error is re-owned into `__errors`. `bind_s` also binds `s` for the string
/// fast path, whose checks read the `&str` subject directly.
fn collect_property_name_errors(
    child_collect: &proc_macro2::TokenStream,
    bind_s: bool,
) -> proc_macro2::TokenStream {
    let s_binding = if bind_s {
        quote! { let s = key; }
    } else {
        quote! {}
    };
    quote! {
        for key in obj.keys() {
            let __key_val: serde_json::Value = serde_json::Value::String(key.clone());
            let mut __key_errors: Vec<jsonschema::ValidationError<'_>> = Vec::new();
            {
                #s_binding
                let instance = &__key_val;
                let __errors = &mut __key_errors;
                #child_collect
            }
            for __e in __key_errors {
                __errors.push(__e.to_owned());
            }
        }
    }
}
