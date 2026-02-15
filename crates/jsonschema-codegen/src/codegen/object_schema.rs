use quote::{format_ident, quote};
use referencing::Draft;
use serde_json::{Map, Value};

use crate::context::CompileContext;

use super::{
    dispatch::compile_typed,
    draft::{
        supports_adjacent_validation, supports_applicator_vocabulary, supports_const_keyword,
        supports_dynamic_ref_keyword, supports_if_then_else_keyword,
        supports_recursive_ref_keyword, supports_validation_vocabulary,
    },
    helpers::get_or_create_is_valid_fn,
    keywords, CompiledExpr,
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
) -> CompiledExpr {
    // Older drafts ignore siblings when `$ref` is present.
    if !supports_adjacent_validation(ctx.draft) {
        if let Some(ref_value) = schema.get("$ref") {
            return keywords::ref_::compile(ctx, ref_value);
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
                let func_name = get_or_create_is_valid_fn(
                    ctx,
                    &location,
                    &Value::Object(schema.clone()),
                    ctx.current_base_uri.clone(),
                );
                let func_ident = format_ident!("{}", func_name);
                let v_ident = format_ident!("{}_v", func_name);
                let e_ident = format_ident!("{}_e", func_name);
                return CompiledExpr::with_validate_blocks(
                    quote! { #func_ident(instance) },
                    quote! {
                        if let Some(__err) = #v_ident(instance, __path.clone()) {
                            return Some(__err);
                        }
                    },
                    quote! {
                        #e_ident(instance, __path.clone(), __errors);
                    },
                );
            }
        }

        let validation_vocab_enabled = supports_validation_vocabulary(ctx);
        let applicator_vocab_enabled = supports_applicator_vocabulary(ctx);

        let has_type_constraint = schema.contains_key("type") && validation_vocab_enabled;

        let typed = compile_typed(ctx, schema, has_type_constraint);
        let has_typed_checks = typed.is_some();

        let mut untyped: Vec<CompiledExpr> = Vec::new();
        let type_is_redundant = type_check_is_redundant(&Value::Object(schema.clone()), ctx.draft);

        if has_type_constraint && !has_typed_checks && !type_is_redundant {
            if let Some(value) = schema.get("type") {
                untyped.push(keywords::type_::compile(ctx, value));
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
            untyped.push(keywords::ref_::compile(ctx, value));
        }
        if supports_dynamic_ref_keyword(ctx.draft) {
            if let Some(value) = schema.get("$dynamicRef") {
                untyped.push(keywords::ref_::compile_dynamic(ctx, value));
            }
        }
        if supports_recursive_ref_keyword(ctx.draft) {
            if let Some(value) = schema.get("$recursiveRef") {
                untyped.push(keywords::ref_::compile_recursive(ctx, value));
            }
        }
        if applicator_vocab_enabled {
            if let Some(value) = schema.get("allOf") {
                untyped.push(keywords::all_of::compile(ctx, value));
            }
            if let Some(value) = schema.get("anyOf") {
                untyped.push(keywords::any_of::compile(ctx, value));
            }
            if let Some(value) = schema.get("oneOf") {
                untyped.push(keywords::one_of::compile(ctx, value));
            }
            if let Some(value) = schema.get("not") {
                untyped.push(keywords::not::compile(ctx, value));
            }
            if supports_if_then_else_keyword(ctx.draft) {
                if let Some(value) = schema.get("if") {
                    if let Some(compiled) = keywords::if_::compile(ctx, schema, value) {
                        untyped.push(compiled);
                    }
                }
            }
        }

        let mut all: Vec<CompiledExpr> = untyped
            .into_iter()
            .filter(|t| !t.is_trivially_true())
            .collect();
        if let Some(type_check) = typed {
            if !type_check.is_trivially_true() {
                all.push(type_check);
            }
        }

        CompiledExpr::combine_and(all)
    })
}
