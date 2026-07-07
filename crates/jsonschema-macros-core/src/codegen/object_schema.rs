use quote::{format_ident, quote};
use referencing::Draft;
use serde_json::{Map, Value};

use crate::context::CompileContext;

use super::{
    dispatch::compile_typed, draft::DraftExt, helpers::get_or_create_is_valid_fn, keywords,
    CompiledExpr,
};

fn type_check_is_redundant(schema: &Map<String, Value>, draft: Draft) -> bool {
    let const_implied_type: Option<&str> = draft
        .supports_const_keyword()
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

    if let Some(implied_type) = const_implied_type.or(enum_implied_type) {
        schema
            .get("type")
            .and_then(|v| v.as_str())
            .is_some_and(|t| t == implied_type)
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
    if !ctx.draft.supports_adjacent_validation() {
        if let Some(ref_value) = schema.get("$ref") {
            return keywords::ref_::compile(ctx, ref_value);
        }
    }

    let id_key = match ctx.draft {
        Draft::Draft4 => "id",
        _ => "$id",
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
            && ctx.draft.supports_recursive_ref_keyword()
            && ctx.schema_depth > 1
            && schema.get("$recursiveAnchor").and_then(Value::as_bool) == Some(true)
        {
            let location = ctx.current_base_uri.to_string();
            let in_progress_here = ctx
                .compiling_stack
                .last()
                .is_some_and(|current| current == &location);
            if !in_progress_here {
                // Clone the schema only on a cache miss.
                let func_name = if let Some(name) = ctx.is_valid_fns.get_name(&location) {
                    name.clone()
                } else {
                    get_or_create_is_valid_fn(
                        ctx,
                        &location,
                        &Value::Object(schema.clone()),
                        ctx.current_base_uri.clone(),
                    )
                };
                let func_ident = format_ident!("{}", func_name);
                let validate_ident = format_ident!("{}_validate", func_name);
                return CompiledExpr::with_validate_blocks(
                    quote! { #func_ident(instance) },
                    quote! {
                        if let Some(__err) = #validate_ident(instance, __path) {
                            return Some(__err);
                        }
                    },
                );
            }
        }

        let validation_vocab_enabled = ctx.supports_validation_vocabulary();
        let applicator_vocab_enabled = ctx.supports_applicator_vocabulary();

        let has_type_constraint = schema.contains_key("type") && validation_vocab_enabled;

        let typed = compile_typed(ctx, schema, has_type_constraint);
        let has_typed_checks = typed.is_some();

        let mut untyped: Vec<CompiledExpr> = Vec::new();
        let type_is_redundant = type_check_is_redundant(schema, ctx.draft);

        if has_type_constraint && !has_typed_checks && !type_is_redundant {
            if let Some(value) = schema.get("type") {
                untyped.push(keywords::type_::compile(ctx, value));
            }
        }
        if ctx.draft.supports_const_keyword() && validation_vocab_enabled {
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
        if ctx.draft.supports_dynamic_ref_keyword() {
            if let Some(value) = schema.get("$dynamicRef") {
                untyped.push(keywords::ref_::compile_dynamic(ctx, value));
            }
        }
        if ctx.draft.supports_recursive_ref_keyword() {
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
            if ctx.draft.supports_if_then_else_keyword() {
                if let Some(value) = schema.get("if") {
                    if let Some(compiled) = keywords::if_::compile(ctx, schema, value) {
                        untyped.push(compiled);
                    }
                }
            }
        }

        if !ctx.config.custom_keywords.is_empty() {
            for (name, value) in schema {
                if ctx.config.custom_keywords.contains_key(name.as_str()) {
                    untyped.push(keywords::custom::compile(ctx, name, schema, value));
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
