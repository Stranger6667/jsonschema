use super::super::{
    generate_multiple_of_check, generate_numeric_check, has_vocabulary, ComparisonOp,
    CompileContext,
};
use proc_macro2::TokenStream;
use quote::quote;
use referencing::{Draft, Vocabulary};
use serde_json::{Map, Value};

/// Compile all number-specific keywords.
pub(in super::super) fn compile(
    ctx: &mut CompileContext<'_>,
    schema: &Map<String, Value>,
) -> TokenStream {
    if !has_vocabulary(ctx, &Vocabulary::Validation) {
        return quote! { true };
    }

    let mut items = Vec::new();
    if matches!(ctx.draft, Draft::Draft4) {
        // Draft 4: exclusiveMinimum/Maximum are boolean modifiers
        let exclusive_min = schema
            .get("exclusiveMinimum")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let exclusive_max = schema
            .get("exclusiveMaximum")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        if let Some(value) = schema.get("minimum") {
            let op = if exclusive_min {
                ComparisonOp::Gt
            } else {
                ComparisonOp::Gte
            };
            items.push(generate_numeric_check(op, value));
        }

        if let Some(value) = schema.get("maximum") {
            let op = if exclusive_max {
                ComparisonOp::Lt
            } else {
                ComparisonOp::Lte
            };
            items.push(generate_numeric_check(op, value));
        }
    } else {
        // Draft 6+: standalone numeric values
        if let Some(value) = schema.get("minimum") {
            items.push(generate_numeric_check(ComparisonOp::Gte, value));
        }
        if let Some(value) = schema.get("maximum") {
            items.push(generate_numeric_check(ComparisonOp::Lte, value));
        }
        if let Some(value) = schema.get("exclusiveMinimum") {
            items.push(generate_numeric_check(ComparisonOp::Gt, value));
        }
        if let Some(value) = schema.get("exclusiveMaximum") {
            items.push(generate_numeric_check(ComparisonOp::Lt, value));
        }
    }

    if let Some(value) = schema.get("multipleOf") {
        items.push(generate_multiple_of_check(value));
    }

    if items.is_empty() {
        quote! { true }
    } else {
        quote! { ( #(#items)&&* ) }
    }
}
