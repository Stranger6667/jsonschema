use crate::codegen::emit::ValueEmitter;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use referencing::{Draft, Uri};
use serde_json::{Map, Value};
use std::sync::Arc;

use crate::context::CompileContext;

use super::{
    dispatch::compile_typed,
    draft::DraftExt,
    expr::{call_errors, ErrorsBlock, IsValidExpr},
    helpers::get_or_create_is_valid_fn,
    keywords, CompiledExpr,
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

/// Keyword groups in the order the runtime reports their errors.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Phase {
    Type,
    Values,
    Typed,
    Applicators,
    Unevaluated,
    References,
    Custom,
}

/// The base URI the keywords of `schema` resolve against: its `$id`'s, or the current one.
pub(super) fn resource_base_uri<E: ValueEmitter>(
    ctx: &CompileContext<'_, E>,
    schema: &Map<String, Value>,
) -> Arc<Uri<String>> {
    let id_key = match ctx.draft {
        Draft::Draft4 => "id",
        _ => "$id",
    };
    schema
        .get(id_key)
        .and_then(Value::as_str)
        .and_then(|id_str| {
            let resolver = ctx
                .config
                .registry
                .resolver((*ctx.current_base_uri).clone());
            resolver
                .lookup(id_str)
                .ok()
                .map(|resolved_id| resolved_id.resolver().base_uri().clone())
        })
        .unwrap_or_else(|| ctx.current_base_uri.clone())
}

/// Compile an object schema; the root keeps the configured draft.
pub(super) fn compile_object_schema<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    schema: &Map<String, Value>,
) -> CompiledExpr {
    if ctx
        .config
        .schema
        .as_object()
        .is_some_and(|root| std::ptr::eq(root, schema))
    {
        compile_in_dialect(ctx, schema)
    } else {
        ctx.with_dialect_of(schema, |ctx| compile_in_dialect(ctx, schema))
    }
}

fn compile_in_dialect<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    schema: &Map<String, Value>,
) -> CompiledExpr {
    // Up to Draft 7, keywords beside `$ref` are ignored.
    if !ctx.draft.supports_adjacent_validation() {
        if let Some(ref_value) = schema.get("$ref") {
            return keywords::ref_::compile(ctx, ref_value);
        }
    }

    let schema_base_uri = resource_base_uri(ctx, schema);

    // A subschema with an `$id` of its own is the resource its keywords are located in, with no
    // absolute location under the default base.
    let embedded = (schema_base_uri.as_str() != ctx.current_base_uri.as_str()).then(|| {
        (
            super::helpers::located_base(&schema_base_uri),
            ctx.current_schema_path().len(),
        )
    });
    let mut compiled = ctx.with_base_uri_scope(schema_base_uri, |ctx| {
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
            // A target the cycle comes back to on its instance keeps code of its own: the runtime
            // guards it by the name its references use, and code shared with another name would
            // close the cycle one visit early.
            let own_cycle_target = ctx
                .helper_root_depths
                .last()
                .is_some_and(|depth| ctx.schema_depth == depth + 1)
                && ctx
                    .compiling_stack
                    .last()
                    .is_some_and(|current| ctx.same_instance_ref_targets.contains(current));
            if !in_progress_here && !own_cycle_target {
                // Clone the schema only on a cache miss.
                let func_name = if let Some(name) = ctx.is_valid_fns.get_name(&location) {
                    name.clone()
                } else {
                    get_or_create_is_valid_fn(
                        ctx,
                        &location,
                        0,
                        &Value::Object(schema.clone()),
                        ctx.current_base_uri.clone(),
                        ctx.dialect(),
                    )
                };
                let func_ident = format_ident!("{}", func_name);
                let errors_ident = format_ident!("{}_errors", func_name);
                let site = ctx.current_schema_path();
                // The error function runs `is_valid` first and reports every error of the
                // subschema.
                return CompiledExpr::with_errors(
                    quote! { #func_ident(instance) },
                    call_errors(&quote! {
                        {
                            let __start = __errors.mark();
                            let __stop = #errors_ident(instance, __path, __errors, "");
                            __errors.inside(__start, #site);
                            __stop
                        }
                    }),
                )
                .with_skips_valid(true);
            }
        }

        let validation_vocab_enabled = ctx.supports_validation_vocabulary();
        let applicator_vocab_enabled = ctx.supports_applicator_vocabulary();

        let type_value = schema.get("type").filter(|_| validation_vocab_enabled);
        let has_type_constraint = type_value.is_some();
        let has_const_or_enum = validation_vocab_enabled
            && ((ctx.draft.supports_const_keyword() && schema.contains_key("const"))
                || schema.contains_key("enum"));
        let has_applicators = applicator_vocab_enabled
            && (["allOf", "anyOf", "oneOf", "not"]
                .iter()
                .any(|keyword| schema.contains_key(*keyword))
                || (ctx.draft.supports_if_then_else_keyword() && schema.contains_key("if")));

        // Errors of `const` and `enum` go between the `type` error and the typed keywords' errors,
        // those of `allOf` to `if` between the typed keywords' and `unevaluated*` errors.
        let typed = compile_typed(
            ctx,
            schema,
            has_type_constraint,
            !has_const_or_enum,
            has_applicators,
        );
        let has_typed_checks = typed.is_some();

        // `is_valid` runs checks in push order; errors follow `Phase`.
        let mut untyped: Vec<(Phase, CompiledExpr)> = Vec::new();
        let mut errors_only: Vec<(Phase, CompiledExpr)> = Vec::new();
        if let Some(value) = type_value.filter(|_| !has_typed_checks || has_const_or_enum) {
            let type_check = keywords::type_::compile(ctx, value);
            // The typed dispatch checks the type already, and so do `const` or `enum` that
            // accept only values of that type.
            if has_typed_checks || type_check_is_redundant(schema, ctx.draft) {
                errors_only.push((Phase::Type, type_check.split_errors().1));
            } else {
                untyped.push((Phase::Type, type_check));
            }
        }
        if ctx.draft.supports_const_keyword() && validation_vocab_enabled {
            if let Some(value) = schema.get("const") {
                untyped.push((Phase::Values, keywords::const_::compile(ctx, value)));
            }
        }
        if validation_vocab_enabled {
            if let Some(value) = schema.get("enum") {
                untyped.push((Phase::Values, keywords::enum_::compile(ctx, value)));
            }
        }
        if let Some(value) = schema.get("$ref") {
            untyped.push((Phase::References, keywords::ref_::compile(ctx, value)));
        }
        if ctx.draft.supports_dynamic_ref_keyword() {
            if let Some(value) = schema.get("$dynamicRef") {
                untyped.push((
                    Phase::References,
                    keywords::ref_::compile_dynamic(ctx, value),
                ));
            }
        }
        if ctx.draft.supports_recursive_ref_keyword() {
            if let Some(value) = schema.get("$recursiveRef") {
                untyped.push((
                    Phase::References,
                    keywords::ref_::compile_recursive(ctx, value),
                ));
            }
        }
        if applicator_vocab_enabled {
            if let Some(value) = schema.get("allOf") {
                untyped.push((Phase::Applicators, keywords::all_of::compile(ctx, value)));
            }
            if let Some(value) = schema.get("anyOf") {
                untyped.push((Phase::Applicators, keywords::any_of::compile(ctx, value)));
            }
            if let Some(value) = schema.get("oneOf") {
                untyped.push((Phase::Applicators, keywords::one_of::compile(ctx, value)));
            }
            if let Some(value) = schema.get("not") {
                untyped.push((Phase::Applicators, keywords::not::compile(ctx, value)));
            }
            if ctx.draft.supports_if_then_else_keyword() {
                if let Some(value) = schema.get("if") {
                    if let Some(compiled) = keywords::if_::compile(ctx, schema, value) {
                        untyped.push((Phase::Applicators, compiled));
                    }
                }
            }
        }

        if !ctx.config.custom_keywords.is_empty() {
            for (name, value) in schema {
                if ctx.config.custom_keywords.contains_key(name.as_str()) {
                    untyped.push((
                        Phase::Custom,
                        keywords::custom::compile(ctx, name, schema, value),
                    ));
                }
            }
        }

        let mut untyped: Vec<(Phase, CompiledExpr)> = untyped
            .into_iter()
            .filter(|(_, check)| !check.is_trivially_true())
            .collect();
        let mut typed = typed.and_then(|(typed, unevaluated)| {
            if let Some(unevaluated) = unevaluated {
                errors_only.push((Phase::Unevaluated, unevaluated));
            }
            (!typed.is_trivially_true()).then_some(typed)
        });

        // `is_valid` runs the typed dispatch first so instances of other types bail before the
        // applicators.
        let untyped_is_valid = untyped
            .iter_mut()
            .map(|(_, check)| std::mem::replace(&mut check.is_valid, IsValidExpr::AlwaysTrue));
        let is_valid = match typed.as_mut() {
            Some(typed) => {
                let typed_is_valid =
                    std::mem::replace(&mut typed.is_valid, IsValidExpr::AlwaysTrue);
                let untyped_is_valid: Vec<TokenStream> = untyped_is_valid
                    .map(IsValidExpr::into_token_stream)
                    .collect();
                if untyped_is_valid.is_empty() {
                    typed_is_valid
                } else {
                    let typed_is_valid = typed_is_valid.into_token_stream();
                    IsValidExpr::Expr(quote! {
                        (#typed_is_valid) #(&& (#untyped_is_valid))*
                    })
                }
            }
            None => untyped_is_valid.fold(IsValidExpr::AlwaysTrue, IsValidExpr::and),
        };

        let mut parts = untyped;
        parts.extend(typed.map(|typed| (Phase::Typed, typed)));
        parts.extend(errors_only);
        parts.sort_by_key(|(phase, _)| *phase);
        CompiledExpr {
            is_valid,
            ..parts
                .into_iter()
                .fold(CompiledExpr::always_true(), |combined, (_, part)| {
                    combined.and(part)
                })
        }
    });
    if let (Some((base, offset)), ErrorsBlock::Expr(errors)) = (embedded, &compiled.errors) {
        let base = base.map_or_else(|| quote! { None }, |base| quote! { Some(#base) });
        compiled.errors = ErrorsBlock::Expr(quote! {
            let __start = __errors.mark();
            let __stop = (|| -> bool { #errors false })();
            __errors.within(__start, #base, #offset);
            if __stop {
                return true;
            }
        });
    }
    compiled
}
