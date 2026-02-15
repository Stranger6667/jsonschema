use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use referencing::Draft;
use serde_json::Value;

use self::{
    draft::{
        supports_applicator_vocabulary, supports_contains_keyword,
        supports_content_validation_keywords, supports_dependent_required_keyword,
        supports_dependent_schemas_keyword, supports_prefix_items_keyword,
        supports_property_names_keyword, supports_recursive_ref_keyword,
        supports_unevaluated_items_keyword_for_context,
        supports_unevaluated_properties_keyword_for_context, supports_validation_vocabulary,
    },
    emit_root::emit_root_module,
    helpers::{
        collect_dynamic_anchor_bindings, get_or_create_item_eval_fn, get_or_create_key_eval_fn,
    },
    numeric::value_as_u64,
    regex::{compile_regex_match, translate_and_validate_regex},
};
use crate::context::CompileContext;
use errors::{invalid_schema_minimum_expression, invalid_schema_type_expression};

pub(crate) mod backend;
mod dispatch;
mod draft;
mod emit_root;
mod errors;
mod expr;
mod helpers;
mod keywords;
mod numeric;
mod object_schema;
mod refs;
mod regex;
mod stack_emit;
pub(crate) mod symbols;

pub(crate) use self::{expr::CompiledExpr, helpers::DynamicAnchorBinding};

/// Entry point: generate validator impl methods from a `CodegenConfig`.
pub(crate) fn generate_from_config(
    config: &crate::context::CodegenConfig,
    recompile_trigger: &TokenStream,
    name: &proc_macro2::Ident,
    impl_mod_name: &proc_macro2::Ident,
) -> TokenStream {
    let mut ctx = CompileContext::new(config);
    let runtime_crate_alias = config
        .runtime_crate_alias
        .clone()
        .map(|path| quote! { use #path as jsonschema; });
    let validation_expr = compile_schema(&mut ctx, &config.schema);
    let recursive_stack_needed = ctx.uses_recursive_ref;
    let dynamic_stack_needed = ctx.uses_dynamic_ref;
    let root_recursive_anchor = recursive_stack_needed
        && supports_recursive_ref_keyword(config.draft)
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
    emit_root_module(
        &ctx,
        runtime_crate_alias.as_ref(),
        recompile_trigger,
        name,
        impl_mod_name,
        &validation_expr,
        recursive_stack_needed,
        dynamic_stack_needed,
        root_recursive_anchor,
        root_key_eval_ident.as_ref(),
        root_item_eval_ident.as_ref(),
        &root_dynamic_bindings,
    )
}

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
                    let __r = Some(jsonschema::keywords_helpers::error::false_schema(
                        #schema_path, __path.clone(), instance,
                    ));
                    if let Some(__e) = __r { return Some(__e); }
                },
                quote! {
                    __errors.push(jsonschema::keywords_helpers::error::false_schema(
                        #schema_path, __path.clone(), instance,
                    ));
                },
            )
        }
        Value::Object(obj) => object_schema::compile_object_schema(ctx, obj),
        _ => invalid_schema_type_expression(schema, &["boolean", "object"]),
    })
}

#[cfg(test)]
mod schema_compile_phase_tests {
    use super::*;
    use serde_json::json;
    use test_case::test_case;

    #[test_case(json!({"type":"number","const":1}), Draft::Draft7, true; "const_integer_implies_number")]
    #[test_case(json!({"type":"string","const":"x"}), Draft::Draft7, true; "const_string_matches")]
    #[test_case(json!({"type":"integer","const":1.5}), Draft::Draft7, false; "const_mismatch")]
    #[test_case(json!({"type":"number","enum":[1, 2]}), Draft::Draft7, true; "enum_integer_implies_number")]
    #[test_case(json!({"type":"string","enum":["a", "b"]}), Draft::Draft7, true; "enum_string_matches")]
    #[test_case(json!({"type":"integer","enum":[1.5]}), Draft::Draft7, false; "enum_mismatch")]
    #[allow(clippy::needless_pass_by_value)]
    fn type_redundancy_detection(schema: Value, draft: Draft, expected: bool) {
        assert_eq!(
            object_schema::type_check_is_redundant(&schema, draft),
            expected
        );
    }
}
