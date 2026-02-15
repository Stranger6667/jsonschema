use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use referencing::Draft;
use serde_json::{Map, Value};

use crate::context::CompileContext;

use super::{
    combinators::{
        compile_all_of, compile_any_of, compile_if_then_else, compile_not, compile_one_of,
    },
    dispatch::{compile_type, compile_typed},
    draft::{
        supports_adjacent_validation, supports_applicator_vocabulary, supports_const_keyword,
        supports_dynamic_ref_keyword, supports_if_then_else_keyword,
        supports_recursive_ref_keyword, supports_validation_vocabulary,
    },
    helpers::get_or_create_function,
    keywords,
    refs::{compile_dynamic_ref, compile_recursive_ref, compile_ref},
};

pub(super) fn type_check_is_redundant(schema: &Value, draft: Draft) -> bool {
    let Value::Object(schema) = schema else {
        return false;
    };

    let const_implied_type: Option<&str> = supports_const_keyword(draft)
        .then(|| schema.get("const"))
        .flatten()
        .and_then(|v| match v {
            Value::String(_) => Some("string"),
            Value::Number(_) => Some("number"),
            Value::Bool(_) => Some("boolean"),
            Value::Null => Some("null"),
            _ => None,
        });
    // If all `enum` values are of the same type, the enum match already implies
    // the type check — suppress the redundant `instance.is_<type>()` prefix.
    let enum_implied_type: Option<&str> = schema.get("enum").and_then(|v| {
        let variants = v.as_array()?;
        if variants.is_empty() {
            return None;
        }
        let type_of = |v: &Value| -> &'static str {
            match v {
                Value::String(_) => "string",
                Value::Number(_) => "number",
                Value::Bool(_) => "boolean",
                Value::Null => "null",
                Value::Array(_) => "array",
                Value::Object(_) => "object",
            }
        };
        let first = type_of(&variants[0]);
        variants
            .iter()
            .all(|v| type_of(v) == first)
            .then_some(first)
    });

    if let Some(const_type) = const_implied_type {
        schema
            .get("type")
            .and_then(|v| v.as_str())
            .is_some_and(|t| t == const_type || t == "number" && const_type == "integer")
    } else if let Some(enum_type) = enum_implied_type {
        schema
            .get("type")
            .and_then(|v| v.as_str())
            .is_some_and(|t| t == enum_type || (t == "number" && enum_type == "integer"))
    } else {
        false
    }
}

/// Compile an object schema.
pub(super) fn compile_object_schema(
    ctx: &mut CompileContext<'_>,
    schema: &Map<String, Value>,
) -> TokenStream {
    // Older drafts ignore siblings when `$ref` is present.
    if !supports_adjacent_validation(ctx.draft) {
        if let Some(ref_value) = schema.get("$ref") {
            return compile_ref(ctx, ref_value);
        }
    }

    let id_key = if schema.contains_key("$id") {
        "$id"
    } else {
        "id"
    };
    let schema_base_uri = schema
        .get(id_key)
        .and_then(Value::as_str)
        .and_then(|id_str| {
            let current_base = ctx.current_base_uri.clone();
            let resolver = ctx.config.registry.resolver((*current_base).clone());
            resolver
                .lookup(id_str)
                .ok()
                .map(|resolved_id| resolved_id.resolver().base_uri().clone())
        })
        .unwrap_or_else(|| ctx.current_base_uri.clone());

    ctx.with_base_uri_scope(schema_base_uri, |ctx| {
        if ctx.uses_recursive_ref
            && supports_recursive_ref_keyword(ctx.draft)
            && ctx.schema_depth > 1
            && schema.get("$recursiveAnchor").and_then(Value::as_bool) == Some(true)
        {
            let location = ctx.current_base_uri.to_string();
            let in_progress_here = ctx
                .compiling_stack
                .last()
                .is_some_and(|current| current == &location);
            if !in_progress_here {
                let func_name = get_or_create_function(
                    ctx,
                    &location,
                    &Value::Object(schema.clone()),
                    ctx.current_base_uri.clone(),
                );
                let func_ident = format_ident!("{}", func_name);
                return quote! { #func_ident(instance) };
            }
        }

        let validation_vocab_enabled = supports_validation_vocabulary(ctx);
        let applicator_vocab_enabled = supports_applicator_vocabulary(ctx);

        // `type` is controlled by the validation vocabulary in modern drafts.
        let has_type_constraint = schema.contains_key("type") && validation_vocab_enabled;

        // Check if we'll generate type-specific checks
        let typed = compile_typed(ctx, schema, has_type_constraint);
        let has_typed_checks = typed.is_some();

        // Collect type-agnostic keywords
        let mut untyped = Vec::new();
        let type_is_redundant = type_check_is_redundant(&Value::Object(schema.clone()), ctx.draft);

        // Only add universal type check if we don't have type-specific checks.
        // When we have type-specific checks with a type constraint,
        // the match statement handles type checking (single discriminant check).
        if has_type_constraint && !has_typed_checks && !type_is_redundant {
            if let Some(value) = schema.get("type") {
                untyped.push(compile_type(ctx, value));
            }
        }
        if supports_const_keyword(ctx.draft) && validation_vocab_enabled {
            if let Some(value) = schema.get("const") {
                untyped.push(keywords::const_::compile(ctx, value));
            }
        }
        if validation_vocab_enabled {
            if let Some(value) = schema.get("enum") {
                untyped.push(keywords::enum_::compile(ctx, value));
            }
        }
        if let Some(value) = schema.get("$ref") {
            untyped.push(compile_ref(ctx, value));
        }
        if supports_dynamic_ref_keyword(ctx.draft) {
            if let Some(value) = schema.get("$dynamicRef") {
                untyped.push(compile_dynamic_ref(ctx, value));
            }
        }
        if supports_recursive_ref_keyword(ctx.draft) {
            if let Some(value) = schema.get("$recursiveRef") {
                untyped.push(compile_recursive_ref(ctx, value));
            }
        }
        if applicator_vocab_enabled {
            if let Some(value) = schema.get("allOf") {
                untyped.push(compile_all_of(ctx, value));
            }
            if let Some(value) = schema.get("anyOf") {
                untyped.push(compile_any_of(ctx, value));
            }
            if let Some(value) = schema.get("oneOf") {
                untyped.push(compile_one_of(ctx, value));
            }
            if let Some(value) = schema.get("not") {
                untyped.push(compile_not(ctx, value));
            }
            if supports_if_then_else_keyword(ctx.draft) {
                if let Some(value) = schema.get("if") {
                    if let Some(compiled) = compile_if_then_else(ctx, schema, value) {
                        untyped.push(compiled);
                    }
                }
            }
        }

        let mut all: Vec<TokenStream> = untyped
            .into_iter()
            .filter(|t| !super::is_trivially_true(t))
            .collect();
        if let Some(type_check) = typed {
            if !super::is_trivially_true(&type_check) {
                all.push(type_check);
            }
        }

        if all.is_empty() {
            // Empty schema - always valid
            quote! { true }
        } else {
            // Combine all checks
            quote! { ( #(#all)&&* ) }
        }
    })
}
