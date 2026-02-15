use crate::context::CompileContext;
use quote::quote;
use serde_json::Value;

use super::super::{invalid_schema_type_expression, CompiledExpr};

// Buckets for grouping enum variants by emitted match arm.
const ENUM_ARRAY_IDX: usize = 0;
const ENUM_BOOL_IDX: usize = 1;
const ENUM_NULL_IDX: usize = 2;
const ENUM_NUM_IDX: usize = 3;
const ENUM_OBJ_IDX: usize = 4;
const ENUM_STR_IDX: usize = 5;

/// Compile the "enum" keyword.
pub(in super::super) fn compile(ctx: &CompileContext<'_>, value: &Value) -> CompiledExpr {
    let Value::Array(variants) = value else {
        return invalid_schema_type_expression(value, &["array"]);
    };
    let schema_path = ctx.schema_path_for_keyword("enum");
    let enum_json = serde_json::to_string(value).expect("Failed to serialize enum value");

    let mut by_type: [Vec<&Value>; 6] = Default::default();

    for variant in variants {
        let idx = match variant {
            Value::Null => ENUM_NULL_IDX,
            Value::Bool(_) => ENUM_BOOL_IDX,
            Value::Number(_) => ENUM_NUM_IDX,
            Value::String(_) => ENUM_STR_IDX,
            Value::Array(_) => ENUM_ARRAY_IDX,
            Value::Object(_) => ENUM_OBJ_IDX,
        };
        by_type[idx].push(variant);
    }

    let mut match_arms = Vec::new();

    // Null: there is only one null value
    if !by_type[ENUM_NULL_IDX].is_empty() {
        let null_pattern = crate::codegen::emit_serde::pattern_null();
        match_arms.push(quote! { #null_pattern => true });
    }

    // Boolean: compare the inner bool directly, no Value wrapping needed
    let booleans = &by_type[ENUM_BOOL_IDX];
    if !booleans.is_empty() {
        let has_true = booleans.iter().any(|v| v.as_bool() == Some(true));
        let has_false = booleans.iter().any(|v| v.as_bool() == Some(false));
        let arm = match (has_true, has_false) {
            (true, true) => {
                let bool_pattern = crate::codegen::emit_serde::pattern_boolean();
                quote! { #bool_pattern => true }
            }
            (true, false) => crate::codegen::emit_serde::match_boolean_arm(quote! { *b }),
            (false, true) => crate::codegen::emit_serde::match_boolean_arm(quote! { !*b }),
            (false, false) => unreachable!(),
        };
        match_arms.push(arm);
    }

    // String: compare as &str without any Value wrapping, avoids LazyLock
    let strings = &by_type[ENUM_STR_IDX];
    if !strings.is_empty() {
        let str_values: Vec<&str> = strings.iter().filter_map(|v| v.as_str()).collect();
        let arm = if let &[s] = str_values.as_slice() {
            let s_as_str = crate::codegen::emit_serde::string_as_str(quote! { s });
            crate::codegen::emit_serde::match_string_arm(quote! { #s_as_str == #s })
        } else {
            let s_as_str = crate::codegen::emit_serde::string_as_str(quote! { s });
            crate::codegen::emit_serde::match_string_arm(
                quote! { matches!(#s_as_str, #(#str_values)|*) },
            )
        };
        match_arms.push(arm);
    }

    // Numbers: use jsonschema-aware comparison to handle cross-type equality
    // (e.g. 0 == 0.0).
    let num_variants = &by_type[ENUM_NUM_IDX];
    if !num_variants.is_empty() {
        let all_numbers: Vec<Value> = num_variants.iter().map(|v| (*v).clone()).collect();
        let numbers_json =
            serde_json::to_string(&all_numbers).expect("Failed to serialize number variants");
        let number_pattern = crate::codegen::emit_serde::pattern_number();
        match_arms.push(quote! {
            #number_pattern => {
            static NUMBER_VARIANTS: std::sync::LazyLock<Vec<serde_json::Value>> =
                std::sync::LazyLock::new(|| {
                    serde_json::from_str::<Vec<serde_json::Value>>(#numbers_json)
                        .expect("Failed to parse number variants")
                });
            NUMBER_VARIANTS.iter().any(|v| jsonschema::__private::cmp::equal(v, instance))
        }
        });
    }

    // Arrays and objects: use jsonschema-aware comparison
    let array_variants = &by_type[ENUM_ARRAY_IDX];
    let object_variants = &by_type[ENUM_OBJ_IDX];
    let has_arrays = !array_variants.is_empty();
    let has_objects = !object_variants.is_empty();
    if has_arrays || has_objects {
        let complex: Vec<Value> = array_variants
            .iter()
            .chain(object_variants.iter())
            .map(|v| (*v).clone())
            .collect();
        let complex_json =
            serde_json::to_string(&complex).expect("Failed to serialize complex variants");
        let arm_pattern = match (has_arrays, has_objects) {
            (true, true) => {
                let array_pattern = crate::codegen::emit_serde::pattern_array();
                let object_pattern = crate::codegen::emit_serde::pattern_object();
                quote! { #array_pattern | #object_pattern }
            }
            (true, false) => crate::codegen::emit_serde::pattern_array(),
            (false, true) => crate::codegen::emit_serde::pattern_object(),
            (false, false) => unreachable!(),
        };
        match_arms.push(quote! {
            #arm_pattern => {
                static COMPLEX_VARIANTS: std::sync::LazyLock<Vec<serde_json::Value>> =
                    std::sync::LazyLock::new(|| {
                        serde_json::from_str::<Vec<serde_json::Value>>(#complex_json)
                            .expect("Failed to parse complex variants")
                    });
                COMPLEX_VARIANTS.iter().any(|v| jsonschema::__private::cmp::equal(v, instance))
            }
        });
    }

    // Default: fast rejection for any type not present in the enum
    match_arms.push(quote! { _ => false });

    let is_valid_ts = quote! {
        match instance {
            #(#match_arms),*
        }
    };
    CompiledExpr::with_validate_blocks(
        is_valid_ts.clone(),
        quote! {
            if !(#is_valid_ts) {
                static ENUM_OPTIONS: std::sync::LazyLock<serde_json::Value> =
                    std::sync::LazyLock::new(|| {
                        serde_json::from_str(#enum_json).expect("Failed to parse enum options")
                    });
                return Some(jsonschema::__private::error::enumeration(
                    #schema_path, __path.clone(), instance, &*ENUM_OPTIONS,
                ));
            }
        },
    )
}
