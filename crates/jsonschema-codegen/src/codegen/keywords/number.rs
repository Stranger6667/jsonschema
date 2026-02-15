use super::{
    super::{draft::has_vocabulary, CompileContext, CompiledExpr},
    minmax::{self, ComparisonOp},
    multiple_of,
};
use proc_macro2::TokenStream;
use quote::quote;
use referencing::{Draft, Vocabulary};
use serde_json::{Map, Value};

/// Compile all number-specific keywords.
pub(in super::super) fn compile(
    ctx: &CompileContext<'_>,
    schema: &Map<String, Value>,
) -> CompiledExpr {
    if !has_vocabulary(ctx, &Vocabulary::Validation) {
        return CompiledExpr::always_true();
    }

    let mut items: Vec<CompiledExpr> = Vec::new();

    if matches!(ctx.draft, Draft::Draft4) {
        let exclusive_min = schema
            .get("exclusiveMinimum")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let exclusive_max = schema
            .get("exclusiveMaximum")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        if let Some(value) = schema.get("minimum") {
            if exclusive_min {
                // Draft 4 uses exclusiveMinimum:true as a boolean modifier of minimum.
                // The dynamic validator reports errors at /exclusiveMinimum, not /minimum.
                items.push(compile_exclusive_min(ctx, value, "exclusiveMinimum"));
            } else {
                items.push(compile_min(ctx, value));
            }
        }
        if let Some(value) = schema.get("maximum") {
            if exclusive_max {
                // Draft 4 uses exclusiveMaximum:true as a boolean modifier of maximum.
                // The dynamic validator reports errors at /exclusiveMaximum, not /maximum.
                items.push(compile_exclusive_max(ctx, value, "exclusiveMaximum"));
            } else {
                items.push(compile_max(ctx, value));
            }
        }
    } else {
        if let Some(value) = schema.get("minimum") {
            items.push(compile_min(ctx, value));
        }
        if let Some(value) = schema.get("maximum") {
            items.push(compile_max(ctx, value));
        }
        if let Some(value) = schema.get("exclusiveMinimum") {
            items.push(compile_exclusive_min(ctx, value, "exclusiveMinimum"));
        }
        if let Some(value) = schema.get("exclusiveMaximum") {
            items.push(compile_exclusive_max(ctx, value, "exclusiveMaximum"));
        }
    }

    if let Some(value) = schema.get("multipleOf") {
        items.push(multiple_of::compile(ctx, value));
    }

    CompiledExpr::combine_and(items)
}

fn compile_min(ctx: &CompileContext<'_>, value: &Value) -> CompiledExpr {
    compile_bound(ctx, value, ComparisonOp::Gte, "minimum")
}

fn compile_max(ctx: &CompileContext<'_>, value: &Value) -> CompiledExpr {
    compile_bound(ctx, value, ComparisonOp::Lte, "maximum")
}

/// `keyword` is the schema keyword whose path is emitted (either "minimum"/"maximum" for draft4
/// exclusive flags, or "exclusiveMinimum"/"exclusiveMaximum" for draft6+).
fn compile_exclusive_min(ctx: &CompileContext<'_>, value: &Value, keyword: &str) -> CompiledExpr {
    compile_bound(ctx, value, ComparisonOp::Gt, keyword)
}

fn compile_exclusive_max(ctx: &CompileContext<'_>, value: &Value, keyword: &str) -> CompiledExpr {
    compile_bound(ctx, value, ComparisonOp::Lt, keyword)
}

fn compile_bound(
    ctx: &CompileContext<'_>,
    value: &Value,
    op: ComparisonOp,
    keyword: &str,
) -> CompiledExpr {
    if !value.is_number() {
        return CompiledExpr::from(minmax::generate_numeric_check(op, value));
    }

    let check_ts = minmax::generate_numeric_check(op, value);
    let schema_path = ctx.schema_path_for_keyword(keyword);
    let value_json = serde_json::to_string(value).unwrap();

    let err_fn: TokenStream = match op {
        ComparisonOp::Gt => quote! { jsonschema::keywords_helpers::error::exclusive_minimum },
        ComparisonOp::Lt => quote! { jsonschema::keywords_helpers::error::exclusive_maximum },
        ComparisonOp::Gte => quote! { jsonschema::keywords_helpers::error::minimum },
        ComparisonOp::Lte => quote! { jsonschema::keywords_helpers::error::maximum },
    };

    let limit_expr = build_limit_value_expr(value, &value_json);

    CompiledExpr::with_validate_blocks(
        check_ts.clone(),
        quote! {
            if !(#check_ts) {
                return Some(#err_fn(#schema_path, __path.clone(), instance, #limit_expr));
            }
        },
        quote! {
            if !(#check_ts) {
                __errors.push(#err_fn(#schema_path, __path.clone(), instance, #limit_expr));
            }
        },
    )
}

/// Build an expression that evaluates to the limit as a `serde_json::Value`.
/// For values that fit into u64/i64, inline them directly.
/// For float-only values, use a `LazyLock`.
fn build_limit_value_expr(value: &Value, value_json: &str) -> TokenStream {
    if let Some(u) = value.as_u64() {
        quote! { serde_json::Value::Number(serde_json::Number::from(#u as u64)) }
    } else if let Some(i) = value.as_i64() {
        quote! { serde_json::Value::Number(serde_json::Number::from(#i as i64)) }
    } else {
        // Float or arbitrary-precision: use LazyLock to parse at runtime.
        quote! {
            {
                static LIMIT: std::sync::LazyLock<serde_json::Value> =
                    std::sync::LazyLock::new(|| serde_json::from_str(#value_json).expect("limit"));
                LIMIT.clone()
            }
        }
    }
}
