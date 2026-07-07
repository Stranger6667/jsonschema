use super::{
    super::{compile_schema, expr::ValidateBlock, CompileContext, CompiledExpr},
    pattern_coverage::build_pattern_coverage,
};
use proc_macro2::TokenStream;
use quote::quote;
use serde_json::Value;

// The wildcard arm for keys that are not defined properties. It runs only without patternProperties
// (patterns route through `object_pass`), so a key reaching it is simply not a defined property, and
// `additionalProperties: false` is instead rejected by the known-keys precheck.
pub(super) fn compile_wildcard_arm(
    ctx: &mut CompileContext<'_>,
    additional_properties: Option<&Value>,
) -> CompiledExpr {
    match additional_properties {
        None | Some(Value::Bool(true)) => CompiledExpr::always_true(),
        Some(schema) => {
            let schema_check = ctx.with_schema_path_segment("additionalProperties", |ctx| {
                ctx.with_instance_scope(|ctx| compile_schema(ctx, schema))
            });
            if schema_check.is_trivially_true() {
                CompiledExpr::always_true()
            } else {
                let schema_is_valid = schema_check.is_valid_token_stream();
                // Bind `instance = value` and extend `__path` so the sub-schema validation sees the
                // property value/path (the match arm in `build_validate_block` does not rebind them).
                match &schema_check.validate {
                    ValidateBlock::Expr(expr) => CompiledExpr::with_validate_blocks(
                        quote! { { #schema_is_valid } },
                        quote! {
                            let instance = value;
                            let __path = &__path.push(key_str);
                            #expr
                        },
                    ),
                    ValidateBlock::AlwaysValid => CompiledExpr::always_true(),
                }
            }
        }
    }
}

pub(crate) fn compile(
    ctx: &mut CompileContext<'_>,
    additional_properties: Option<&Value>,
    pattern_properties: Option<&Value>,
) -> Option<CompiledExpr> {
    let additional_properties = additional_properties?;

    // `patternProperties` combined with a false or schema `additionalProperties` is fused into a
    // single instance-order pass by `object_pass`, so this path never covers keys by pattern; the
    // only role of `pattern_properties` here is surfacing an invalid pattern regex.
    if let Err(err) = build_pattern_coverage(ctx, pattern_properties) {
        return Some(err);
    }

    let schema_path = ctx.schema_path_for_keyword("additionalProperties");
    match additional_properties {
        Value::Bool(false) => Some(CompiledExpr::from_bool_expr(
            quote! { obj.is_empty() },
            &schema_path,
        )),
        Value::Bool(true) => None,
        schema => {
            let schema_check = ctx.with_schema_path_segment("additionalProperties", |ctx| {
                ctx.with_instance_scope(|ctx| compile_schema(ctx, schema))
            });
            if schema_check.is_trivially_true() {
                return None;
            }
            let schema_is_valid = schema_check.is_valid_token_stream();
            match &schema_check.validate {
                ValidateBlock::Expr(expr) => Some(CompiledExpr::with_validate_blocks(
                    quote! { obj.values().all(|instance| #schema_is_valid) },
                    quote! {
                        for (key, value) in obj.iter() {
                            let instance = value;
                            let __path = &__path.push(key.as_str());
                            #expr
                        }
                    },
                )),
                ValidateBlock::AlwaysValid => None,
            }
        }
    }
}

/// Build a `validate` block for `additionalProperties: false`: return an `AdditionalProperties`
/// error for the first key not covered by `properties` (`known_props`), matching the runtime's
/// fail-fast reporting.
pub(super) fn compile_first_unexpected_check(
    known_properties: &[&str],
    schema_path: &str,
) -> TokenStream {
    let covered = if known_properties.is_empty() {
        quote! { false }
    } else {
        quote! { matches!(key_str, #(#known_properties)|*) }
    };
    quote! {
        if let Some(obj) = instance.as_object() {
            for key in obj.keys() {
                let key_str = key.as_str();
                if !(#covered) {
                    return Some(jsonschema::__private::error::additional_properties(
                        #schema_path, __path.into(), instance, vec![key.clone()],
                    ));
                }
            }
        }
    }
}
