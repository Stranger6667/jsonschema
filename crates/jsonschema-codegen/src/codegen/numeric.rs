use referencing::Draft;
use serde_json::Value;

fn pow10_u128(exp: u32) -> Option<u128> {
    let mut acc = 1u128;
    for _ in 0..exp {
        acc = acc.checked_mul(10)?;
    }
    Some(acc)
}

fn parse_nonnegative_integer_number_literal(literal: &str) -> Option<u64> {
    let (is_negative, literal) = match literal.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, literal),
    };

    let (mantissa, exponent) = match literal.find(['e', 'E']) {
        Some(idx) => {
            let exponent = literal.get(idx + 1..)?.parse::<i32>().ok()?;
            (&literal[..idx], exponent)
        }
        None => (literal, 0),
    };
    let (int_part, frac_part) = match mantissa.split_once('.') {
        Some((int_part, frac_part)) => (int_part, frac_part),
        None => (mantissa, ""),
    };
    if int_part.is_empty() || !int_part.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if !frac_part.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let mut digits = String::with_capacity(int_part.len() + frac_part.len());
    digits.push_str(int_part);
    digits.push_str(frac_part);
    let trimmed = digits.trim_start_matches('0');
    let mut value = if trimmed.is_empty() {
        0u128
    } else {
        trimmed.parse::<u128>().ok()?
    };

    let frac_len = i32::try_from(frac_part.len()).ok()?;
    let net_scale = frac_len - exponent;
    if net_scale > 0 {
        let divisor = pow10_u128(u32::try_from(net_scale).ok()?)?;
        if value % divisor != 0 {
            return None;
        }
        value /= divisor;
    } else if net_scale < 0 {
        let multiplier = pow10_u128(u32::try_from(-net_scale).ok()?)?;
        value = value.checked_mul(multiplier)?;
    }

    let parsed = u64::try_from(value).ok()?;
    if is_negative && parsed != 0 {
        return None;
    }
    Some(parsed)
}

/// Extract u64 from a JSON value, handling both integers and decimals like 2.0.
/// Draft 6+ allows integer-valued numbers (e.g., 2.0), Draft 4 does not.
pub(super) fn value_as_u64(draft: Draft, value: &Value) -> Option<u64> {
    if let Some(n) = value.as_u64() {
        return Some(n);
    }
    if !matches!(draft, Draft::Draft4) {
        if let Value::Number(number) = value {
            return parse_nonnegative_integer_number_literal(&number.to_string());
        }
    }
    None
}
