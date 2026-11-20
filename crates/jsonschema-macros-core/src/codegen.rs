use crate::codegen::emit::ValueEmitter;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use referencing::Draft;
use serde_json::Value;
use syn::Ident;

use self::{
    draft::DraftExt,
    emit_root::emit_root_module,
    helpers::{
        collect_dynamic_anchor_bindings, get_or_create_is_valid_fn, get_or_create_item_eval_fn,
        get_or_create_key_eval_fn,
    },
    numeric::value_as_u64,
    object_schema::resource_base_uri,
    regex::{analyze_pattern, compile_regex_match, translate_and_validate_regex, RegexSite},
};
use crate::context::{CodegenConfig, CompileContext, RefResultsKey};
use errors::{invalid_schema_minimum_expression, invalid_schema_type_expression};

mod dispatch;
mod draft;
pub(crate) mod emit;
pub(crate) mod emit_magnus;
pub(crate) mod emit_pyo3;
mod emit_root;
pub(crate) mod emit_serde;
mod errors;
pub(crate) mod evaluation;
mod expr;
mod helpers;
mod keywords;
mod numeric;
mod object_schema;
mod ref_revisits;
mod refs;
mod regex;
mod stack_emit;

pub(crate) use expr::CompiledExpr;
pub(crate) use helpers::DynamicAnchorBinding;

/// Entry point: generate validator impl methods from a `CodegenConfig`.
///
/// # Errors
///
/// A meta-schema of the schema or of a referenced resource requires a vocabulary this crate does
/// not implement and the caller did not declare support for.
pub(crate) fn generate_from_config<E: ValueEmitter>(
    config: &CodegenConfig,
    recompile_trigger: &TokenStream,
    name: &Ident,
    impl_mod_name: &Ident,
) -> syn::Result<TokenStream> {
    let mut ctx = CompileContext::<E>::new(config);
    let ref_cycles = ref_revisits::ref_cycles(&mut ctx, &config.schema);
    ctx.revisited_ref_targets = ref_cycles.revisited;
    ctx.same_instance_ref_targets = ref_cycles.same_instance;
    if ref_cycles.through_scope {
        ctx.ref_results_key = RefResultsKey::InstanceAndScope { calls: 0 };
    }
    // A `$ref` to `#` runs the root's code. On a cycle back to the root's instance, the root
    // compiles as that target, so references back to it take the target's cycle guard and the
    // root's own `$ref: "#"` applies nothing.
    let root = root_reference_target(&mut ctx, &config.schema);
    if ctx.closes_same_instance_cycle(&root.location) {
        get_or_create_is_valid_fn(
            &mut ctx,
            &root.location,
            root.resource_start,
            root.schema,
            root.base_uri,
            root.dialect,
        );
        ctx.root_target = Some(root.location);
    }
    let root_targets: Vec<String> = ctx.root_target.iter().cloned().collect();
    let validation_expr = ctx.within_targets(&root_targets, |ctx| {
        ctx.with_helper_root_scope(|ctx| compile_schema(ctx, &config.schema))
    });
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
    // An anchored root enters the dynamic scope as the code a `$recursiveRef` in it names.
    let root_recursive_target = root_recursive_anchor.then(|| {
        let root_base = config.schema.as_object().map_or_else(
            || config.base_uri.clone(),
            |root| resource_base_uri(&ctx, root),
        );
        let root = ctx
            .with_base_uri_scope(root_base, |ctx| refs::resolve_ref(ctx, "#"))
            .expect("the root resolves");
        format_ident!(
            "{}",
            get_or_create_is_valid_fn(
                &mut ctx,
                &root.location,
                root.resource_start,
                root.schema,
                root.base_uri,
                root.dialect,
            )
        )
    });
    let root_key_eval_ident = if recursive_stack_needed {
        let dialect = ctx.dialect();
        let name = get_or_create_key_eval_fn(
            &mut ctx,
            "__root_key_eval",
            &config.schema,
            config.base_uri.clone(),
            dialect,
        );
        Some(format_ident!("{}", name))
    } else {
        None
    };
    let root_item_eval_ident = if recursive_stack_needed {
        let dialect = ctx.dialect();
        let name = get_or_create_item_eval_fn(
            &mut ctx,
            "__root_item_eval",
            &config.schema,
            config.base_uri.clone(),
            dialect,
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
    if let Some(uri) = &ctx.undeclared_vocabulary {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("Unknown vocabulary: '{uri}' is required by the meta-schema. Adjust configuration to declare support for it"),
        ));
    }
    let evaluation_body = config
        .methods
        .evaluate
        .then(|| evaluation::compile(&mut ctx))
        .transpose()?;
    Ok(emit_root_module(
        &ctx,
        config.runtime_crate_alias.as_ref(),
        recompile_trigger,
        name,
        impl_mod_name,
        &validation_expr,
        evaluation_body.as_ref(),
        recursive_stack_needed,
        dynamic_stack_needed,
        root_recursive_target.as_ref(),
        root_key_eval_ident.as_ref(),
        root_item_eval_ident.as_ref(),
        &root_dynamic_bindings,
    ))
}

/// The target of a `$ref` naming the root by `#` from within it.
fn root_reference_target<'cfg, E: ValueEmitter>(
    ctx: &mut CompileContext<'cfg, E>,
    root: &Value,
) -> refs::ResolvedRef<'cfg> {
    let base = root.as_object().map_or_else(
        || ctx.config.base_uri.clone(),
        |root| resource_base_uri(ctx, root),
    );
    ctx.with_base_uri_scope(base, |ctx| refs::resolve_ref(ctx, "#"))
        .expect("the root resolves")
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
pub(crate) fn compile_schema<E: ValueEmitter>(
    ctx: &mut CompileContext<'_, E>,
    schema: &Value,
) -> CompiledExpr {
    let err_instance = E::err_instance(format_ident!("instance"));
    ctx.with_schema_scope(|ctx| match schema {
        Value::Bool(true) => CompiledExpr::always_true(),
        Value::Bool(false) => {
            let schema_path = ctx.current_schema_path().to_owned();
            CompiledExpr::from_first_error(
                quote! { false },
                quote! {
                    break '__first_error Some(__err::false_schema(
                        #schema_path, __path.into(), #err_instance,
                    ));
                },
            )
        }
        Value::Object(obj) => object_schema::compile_object_schema(ctx, obj),
        _ => invalid_schema_type_expression(schema, &["boolean", "object"]),
    })
}
