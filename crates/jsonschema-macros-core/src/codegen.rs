use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use referencing::Draft;
use serde_json::Value;
use syn::Ident;

use self::{
    draft::DraftExt,
    emit_root::emit_root_module,
    helpers::{
        collect_dynamic_anchor_bindings, get_or_create_item_eval_fn, get_or_create_key_eval_fn,
    },
    numeric::value_as_u64,
    refs::resolve_ref,
    regex::{compile_regex_match, translate_and_validate_regex},
};
use crate::context::{CodegenConfig, CompileContext};
use errors::{invalid_schema_minimum_expression, invalid_schema_type_expression};

mod dispatch;
mod draft;
mod emit_root;
mod emit_serde;
mod errors;
mod evaluation;
mod expr;
mod helpers;
mod keywords;
mod numeric;
mod object_schema;
pub(crate) mod refs;
mod regex;
mod stack_emit;
mod unevaluated_scan;

pub(crate) use expr::CompiledExpr;
pub(crate) use helpers::DynamicAnchorBinding;
pub(crate) use unevaluated_scan::scan_uses_unevaluated_over;

/// Entry point: generate validator impl methods from a `CodegenConfig`.
pub(crate) fn generate_from_config(
    config: &CodegenConfig,
    recompile_trigger: &TokenStream,
    name: &Ident,
    impl_mod_name: &Ident,
) -> TokenStream {
    let mut ctx = CompileContext::new(config);
    let validation_expr = compile_schema(&mut ctx, &config.schema);
    // `evaluate = false` skips the entire evaluation walk: no evaluation helpers, no node-location
    // statics, no evaluate function. This is the dominant generate-time cost, so it is gated first.
    let evaluation_expr = if config.method_gates.evaluate {
        evaluation::compile(&mut ctx, &config.schema)
    } else {
        None
    };
    let recursive_stack_needed = ctx.uses_recursive_ref;
    let dynamic_stack_needed = ctx.uses_dynamic_ref;
    let root_recursive_anchor = recursive_stack_needed
        && config.draft.supports_recursive_ref_keyword()
        && config
            .schema
            .as_object()
            .and_then(|obj| obj.get("$recursiveAnchor"))
            .and_then(Value::as_bool)
            == Some(true);
    let root_key_eval_ident = if recursive_stack_needed {
        let name = get_or_create_key_eval_fn(
            &mut ctx,
            "__root_key_eval",
            &config.schema,
            config.base_uri.clone(),
        );
        Some(format_ident!("{}", name))
    } else {
        None
    };
    let root_item_eval_ident = if recursive_stack_needed {
        let name = get_or_create_item_eval_fn(
            &mut ctx,
            "__root_item_eval",
            &config.schema,
            config.base_uri.clone(),
        );
        Some(format_ident!("{}", name))
    } else {
        None
    };
    let root_dynamic_bindings = if dynamic_stack_needed {
        collect_dynamic_anchor_bindings(&mut ctx, config.base_uri.clone())
    } else {
        Vec::new()
    };
    let root_evaluation_ident = if config.method_gates.evaluate && recursive_stack_needed {
        resolve_ref(&mut ctx, "#").ok().map(|resolved| {
            let name = evaluation::get_or_create_evaluation_fn(
                &mut ctx,
                &resolved.location,
                resolved.schema,
                resolved.base_uri,
            );
            format_ident!("{name}")
        })
    } else {
        None
    };
    emit_root_module(
        &ctx,
        config.runtime_crate_alias.as_ref(),
        recompile_trigger,
        name,
        impl_mod_name,
        &validation_expr,
        evaluation_expr.as_ref(),
        recursive_stack_needed,
        dynamic_stack_needed,
        root_recursive_anchor,
        root_key_eval_ident.as_ref(),
        root_item_eval_ident.as_ref(),
        root_evaluation_ident.as_ref(),
        &root_dynamic_bindings,
    )
}

// Draft 4 does not treat integer-valued floats like `-2.0` as integers.
// The `jsonschema` runtime helpers can't be reused here without a dependency cycle.
fn is_negative_integer_valued_number(draft: Draft, value: &Value) -> bool {
    if value.as_i64().is_some_and(|n| n < 0) {
        return true;
    }
    if matches!(draft, Draft::Draft4) {
        return false;
    }
    value
        .as_f64()
        .is_some_and(|n| n.is_finite() && n < 0.0 && n.fract() == 0.0)
}

pub(crate) fn parse_nonnegative_integer_keyword(
    draft: Draft,
    value: &Value,
) -> Result<u64, CompiledExpr> {
    if let Some(parsed) = value_as_u64(draft, value) {
        Ok(parsed)
    } else if is_negative_integer_valued_number(draft, value) {
        Err(invalid_schema_minimum_expression(value, "0"))
    } else {
        Err(invalid_schema_type_expression(value, &["integer"]))
    }
}

/// Compile a schema into validation code.
pub(crate) fn compile_schema(ctx: &mut CompileContext<'_>, schema: &Value) -> CompiledExpr {
    ctx.with_schema_scope(|ctx| match schema {
        Value::Bool(true) => CompiledExpr::always_true(),
        Value::Bool(false) => {
            let schema_path = ctx.current_schema_path().to_owned();
            CompiledExpr::with_validate_blocks(
                quote! { false },
                quote! {
                    return Some(__err::false_schema(
                        #schema_path, __path.into(), instance,
                    ));
                },
            )
        }
        Value::Object(obj) => object_schema::compile_object_schema(ctx, obj),
        _ => invalid_schema_type_expression(schema, &["boolean", "object"]),
    })
}
