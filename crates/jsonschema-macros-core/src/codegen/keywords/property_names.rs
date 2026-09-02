use super::super::{
    compile_schema, expr::ValidateBlock, refs::resolve_lone_top_level_ref, CompileContext,
    CompiledExpr,
};
use crate::codegen::emit::ValueEmitter;
use quote::{format_ident, quote};
use serde_json::Value;
use std::borrow::Cow;

pub(crate) fn compile<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    value: &Value,
) -> CompiledExpr {
    let err_instance = E::err_instance(format_ident!("instance"));
    if value == &Value::Bool(false) {
        let schema_path = ctx.schema_path_for_keyword("propertyNames");
        // Like the runtime: a non-empty object fails, and the error is reported against the
        // whole object, not individual keys.
        let is_empty = E::object_is_empty(format_ident!("obj"));
        return CompiledExpr::from_bool_expr(is_empty, &schema_path, &err_instance);
    }

    let node = E::node_param(None);
    let key_as_value_ref = E::key_as_value_ref(format_ident!("key"));

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
                        let keys_iter = E::object_keys_iter(format_ident!("obj"));
                        let declare_key = E::declare_key_node(format_ident!("key"));
                        let key_node = E::key_node_expr(format_ident!("key"));
                        let subject = E::key_as_string_subject(format_ident!("key"));
                        CompiledExpr::with_validate_and_collect_blocks(
                            E::object_keys_all_strings(keys_iter.clone(), is_valid.clone()),
                            quote! {
                                for key in #keys_iter {
                                    let s = #subject;
                                    if !(#is_valid) {
                                        #declare_key
                                        let instance = #key_node;
                                        if let Some(__e) = (|| -> Option<__VE<'_>> {
                                            #expr
                                            None
                                        })().map(|e| e.to_owned()) {
                                            return Some(__e);
                                        }
                                    }
                                }
                            },
                            collect_property_name_errors::<E>(&child_collect, true),
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
            let keys_iter = E::object_keys_iter(format_ident!("obj"));
            let declare_key = E::declare_key_node(format_ident!("key"));
            let key_node = E::key_node_expr(format_ident!("key"));
            CompiledExpr::with_validate_and_collect_blocks(
                quote! {
                    #keys_iter.all(|key| {
                        (|instance: #node| #is_valid)(#key_as_value_ref)
                    })
                },
                quote! {
                    for key in #keys_iter {
                        #declare_key
                        if let Some(__e) = (|| -> Option<__VE<'_>> {
                            let instance = #key_node;
                            #expr
                            None
                        })().map(|e| e.to_owned()) {
                            return Some(__e);
                        }
                    }
                },
                collect_property_name_errors::<E>(&child_collect, false),
            )
        }
        ValidateBlock::AlwaysValid => CompiledExpr::always_true(),
    }
}

/// Per-key `collect` loop: each key becomes a temporary `Value::String`, the child collects into a
/// scratch vec, and every error is re-owned into `__errors`. `bind_s` also binds `s` for the string
/// fast path, whose checks read the `&str` subject directly.
fn collect_property_name_errors<E: ValueEmitter>(
    child_collect: &proc_macro2::TokenStream,
    bind_s: bool,
) -> proc_macro2::TokenStream {
    let subject = E::key_as_string_subject(format_ident!("key"));
    let s_binding = if bind_s {
        quote! { let s = #subject; }
    } else {
        quote! {}
    };
    let keys_iter = E::object_keys_iter(format_ident!("obj"));
    let declare_key = E::declare_key_node(format_ident!("key"));
    let key_node = E::key_node_expr(format_ident!("key"));
    quote! {
        for key in #keys_iter {
            #declare_key
            let mut __key_errors: Vec<__VE<'_>> = Vec::new();
            {
                #s_binding
                let instance = #key_node;
                let __errors = &mut __key_errors;
                #child_collect
            }
            for __e in __key_errors {
                __errors.push(__e.to_owned());
            }
        }
    }
}
