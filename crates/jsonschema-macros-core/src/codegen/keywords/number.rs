use super::{
    super::{CompileContext, CompiledExpr},
    minmax::{self, ComparisonOp},
    multiple_of,
};
use proc_macro2::TokenStream;
use quote::quote;
use referencing::Draft;
use serde_json::{Map, Value};

/// Compile all number-specific keywords.
pub(in super::super) fn compile(
    ctx: &CompileContext<'_>,
    schema: &Map<String, Value>,
) -> CompiledExpr {
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

        if exclusive_min || exclusive_max {
            if let Some(value) = schema.get("minimum") {
                if exclusive_min {
                    items.push(compile_exclusive_min(ctx, value, "exclusiveMinimum"));
                } else {
                    items.push(compile_min(ctx, value));
                }
            }
            if let Some(value) = schema.get("maximum") {
                if exclusive_max {
                    items.push(compile_exclusive_max(ctx, value, "exclusiveMaximum"));
                } else {
                    items.push(compile_max(ctx, value));
                }
            }
        } else {
            compile_inclusive_min_max(
                ctx,
                schema.get("minimum"),
                schema.get("maximum"),
                &mut items,
            );
        }
    } else {
        compile_inclusive_min_max(
            ctx,
            schema.get("minimum"),
            schema.get("maximum"),
            &mut items,
        );
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

fn compile_inclusive_min_max(
    ctx: &CompileContext<'_>,
    minimum: Option<&Value>,
    maximum: Option<&Value>,
    items: &mut Vec<CompiledExpr>,
) {
    if let (Some(min_value), Some(max_value)) = (minimum, maximum) {
        if let Some(folded) = compile_equal_bounds(ctx, min_value, max_value) {
            items.push(folded);
            return;
        }
    }
    if let Some(value) = minimum {
        items.push(compile_min(ctx, value));
    }
    if let Some(value) = maximum {
        items.push(compile_max(ctx, value));
    }
}

fn compile_equal_bounds(
    ctx: &CompileContext<'_>,
    min_value: &Value,
    max_value: &Value,
) -> Option<CompiledExpr> {
    if min_value != max_value {
        return None;
    }
    if min_value.as_u64().is_none() && min_value.as_i64().is_none() {
        return None;
    }
    let min_path = ctx.schema_path_for_keyword("minimum");
    let max_path = ctx.schema_path_for_keyword("maximum");
    let value_json = serde_json::to_string(min_value).unwrap();
    let limit_expr = build_limit_value_expr(min_value, &value_json);
    let eq_check = equal_bound_check(min_value);
    let below_check = minmax::generate_numeric_check(ComparisonOp::Lt, min_value);
    let above_check = minmax::generate_numeric_check(ComparisonOp::Gt, min_value);
    Some(CompiledExpr::with_validate_blocks(
        eq_check,
        quote! {
            if #below_check {
                return Some(__err::minimum(
                    #min_path, __path.into(), instance, #limit_expr,
                ));
            }
            if #above_check {
                return Some(__err::maximum(
                    #max_path, __path.into(), instance, #limit_expr,
                ));
            }
        },
    ))
}

fn equal_bound_check(value: &Value) -> TokenStream {
    if let Some(unsigned) = value.as_u64() {
        quote! { jsonschema::__private::numeric::eq(n, #unsigned) }
    } else {
        let signed = value
            .as_i64()
            .expect("equal-bounds fold is restricted to integer limits");
        quote! { jsonschema::__private::numeric::eq(n, #signed) }
    }
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
        return CompiledExpr::from_error(minmax::generate_numeric_check(op, value));
    }

    let check = minmax::generate_numeric_check(op, value);
    let schema_path = ctx.schema_path_for_keyword(keyword);
    let value_json = serde_json::to_string(value).unwrap();

    let error_fn: TokenStream = match op {
        ComparisonOp::Gt => quote! { __err::exclusive_minimum },
        ComparisonOp::Lt => quote! { __err::exclusive_maximum },
        ComparisonOp::Gte => quote! { __err::minimum },
        ComparisonOp::Lte => quote! { __err::maximum },
    };

    let limit_expr = build_limit_value_expr(value, &value_json);

    CompiledExpr::from_check_and_error(
        check,
        quote! { #error_fn(#schema_path, __path.into(), instance, #limit_expr) },
    )
}

/// Build an expression evaluating to the limit as a `serde_json::Value`.
fn build_limit_value_expr(value: &Value, value_json: &str) -> TokenStream {
    if let Some(unsigned) = value.as_u64() {
        quote! { serde_json::Value::Number(serde_json::Number::from(#unsigned)) }
    } else if let Some(signed) = value.as_i64() {
        quote! { serde_json::Value::Number(serde_json::Number::from(#signed)) }
    } else {
        // Float or arbitrary-precision: use LazyLock to parse at runtime.
        quote! {
            {
                static LIMIT: __Lazy<serde_json::Value> =
                    __Lazy::new(|| serde_json::from_str(#value_json).expect("limit"));
                LIMIT.clone()
            }
        }
    }
}
