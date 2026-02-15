use crate::context::CompileContext;
use quote::quote;
use serde_json::Value;

use super::super::{invalid_schema_type_expression, CompiledExpr};

/// Compile the "enum" keyword.
pub(in super::super) fn compile(ctx: &CompileContext<'_>, value: &Value) -> CompiledExpr {
    let Value::Array(variants) = value else {
        return invalid_schema_type_expression(value, &["array"]);
    };
    let schema_path = ctx.schema_path_for_keyword("enum");
    let enum_json = serde_json::to_string(value).expect("Failed to serialize enum value");

    // Variants grouped by the match arm they are emitted into.
    let mut nulls: Vec<&Value> = Vec::new();
    let mut booleans: Vec<&Value> = Vec::new();
    let mut num_variants: Vec<&Value> = Vec::new();
    let mut strings: Vec<&Value> = Vec::new();
    let mut array_variants: Vec<&Value> = Vec::new();
    let mut object_variants: Vec<&Value> = Vec::new();
    for variant in variants {
        match variant {
            Value::Null => nulls.push(variant),
            Value::Bool(_) => booleans.push(variant),
            Value::Number(_) => num_variants.push(variant),
            Value::String(_) => strings.push(variant),
            Value::Array(_) => array_variants.push(variant),
            Value::Object(_) => object_variants.push(variant),
        }
    }

    let mut match_arms = Vec::new();

    // Null: there is only one null value
    if !nulls.is_empty() {
        let null_pattern = crate::codegen::emit_serde::pattern_null();
        match_arms.push(quote! { #null_pattern => true });
    }

    // Boolean: compare the inner bool directly, no Value wrapping needed
    if !booleans.is_empty() {
        let has_true = booleans
            .iter()
            .any(|variant| variant.as_bool() == Some(true));
        let has_false = booleans
            .iter()
            .any(|variant| variant.as_bool() == Some(false));
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
    if !strings.is_empty() {
        let str_values: Vec<&str> = strings
            .iter()
            .filter_map(|variant| variant.as_str())
            .collect();
        let arm = if let &[expected] = str_values.as_slice() {
            let instance_as_str = crate::codegen::emit_serde::string_as_str(quote! { s });
            crate::codegen::emit_serde::match_string_arm(quote! { #instance_as_str == #expected })
        } else {
            let instance_as_str = crate::codegen::emit_serde::string_as_str(quote! { s });
            crate::codegen::emit_serde::match_string_arm(
                quote! { matches!(#instance_as_str, #(#str_values)|*) },
            )
        };
        match_arms.push(arm);
    }

    // Numbers: use jsonschema-aware comparison to handle cross-type equality
    // (e.g. 0 == 0.0).
    if !num_variants.is_empty() {
        let all_numbers: Vec<Value> = num_variants
            .iter()
            .map(|variant| (*variant).clone())
            .collect();
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
            NUMBER_VARIANTS.iter().any(|variant| jsonschema::__private::cmp::equal(variant, instance))
        }
        });
    }

    // Arrays and objects: use jsonschema-aware comparison
    let has_arrays = !array_variants.is_empty();
    let has_objects = !object_variants.is_empty();
    if has_arrays || has_objects {
        let complex: Vec<Value> = array_variants
            .iter()
            .chain(object_variants.iter())
            .map(|variant| (*variant).clone())
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
                COMPLEX_VARIANTS.iter().any(|variant| jsonschema::__private::cmp::equal(variant, instance))
            }
        });
    }

    // Default: fast rejection for any type not present in the enum
    match_arms.push(quote! { _ => false });

    let is_valid = quote! {
        match instance {
            #(#match_arms),*
        }
    };
    CompiledExpr::with_validate_blocks(
        is_valid.clone(),
        quote! {
            if !(#is_valid) {
                static ENUM_OPTIONS: std::sync::LazyLock<serde_json::Value> =
                    std::sync::LazyLock::new(|| {
                        serde_json::from_str(#enum_json).expect("Failed to parse enum options")
                    });
                return Some(jsonschema::__private::error::enumeration(
                    #schema_path, __path.into(), instance, &*ENUM_OPTIONS,
                ));
            }
        },
    )
}
