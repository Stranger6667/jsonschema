use hegel::{generators as gs, TestCase};
use serde_json::{json, Number, Value};

use super::{
    fraction::{excluded_divisors, Fraction},
    pool::arbitrary_instance,
    size_ceiling, size_floor, MAX_SIZE,
};

/// Smallest `i64` at or above the bound, or `None` when every `i64` sits below it.
fn i64_at_or_above(bound: &Number) -> Option<i64> {
    if let Some(int) = bound.as_i64() {
        return Some(int);
    }
    let float = bound.as_f64()?;
    if float >= i64::MAX as f64 {
        return None;
    }
    if float < i64::MIN as f64 {
        return Some(i64::MIN);
    }
    Some(float.ceil() as i64)
}

/// Largest `i64` at or below the bound, or `None` when every `i64` sits above it.
fn i64_at_or_below(bound: &Number) -> Option<i64> {
    if let Some(int) = bound.as_i64() {
        return Some(int);
    }
    let float = bound.as_f64()?;
    if float < i64::MIN as f64 {
        return None;
    }
    if float > i64::MAX as f64 {
        return Some(i64::MAX);
    }
    Some(float.floor() as i64)
}

pub(crate) fn draw_integer(
    tc: &TestCase,
    minimum: Option<&Number>,
    maximum: Option<&Number>,
    multiple_of: &[Number],
    not_multiple_of: &[Number],
) -> Option<Value> {
    let low = match minimum {
        Some(bound) => Some(i64_at_or_above(bound)?),
        None => None,
    };
    let high = match maximum {
        Some(bound) => Some(i64_at_or_below(bound)?),
        None => None,
    };
    if let (Some(low), Some(high)) = (low, high) {
        if low > high {
            return None;
        }
    }
    // Integer multiples of a reduced `p/q` grid are exactly the multiples of `p`, and several
    // divisors admit the multiples of their least common multiple.
    let mut stride = Fraction::ONE;
    for divisor in multiple_of {
        let each = Fraction::divisor(divisor)?;
        stride = stride.lcm(Fraction::integer(each.numerator))?;
    }
    let barred = excluded_divisors(not_multiple_of)?;
    if barred.iter().any(|divisor| stride.is_multiple_of(*divisor)) {
        // Every grid point is barred.
        return None;
    }
    let stride = i64::try_from(stride.numerator).ok()?;
    let ceil_step =
        |value: i64| value.div_euclid(stride) + i64::from(value.rem_euclid(stride) != 0);
    let floor_step = |value: i64| value.div_euclid(stride);
    // The window defaults live in step space, so a sparse grid keeps its points.
    let (low_step, high_step) = match (low, high) {
        (Some(low), Some(high)) => (ceil_step(low), floor_step(high)),
        (Some(low), None) => {
            let step = ceil_step(low);
            (step, step.saturating_add(16))
        }
        (None, Some(high)) => {
            let step = floor_step(high);
            (step.saturating_sub(16), step)
        }
        (None, None) => (-8, 8),
    };
    if low_step > high_step {
        return None;
    }
    let high_step = high_step.min(low_step.saturating_add(16));
    let step = tc.draw(
        gs::integers::<i64>()
            .min_value(low_step)
            .max_value(high_step),
    );
    let value = step.checked_mul(stride)?;
    if barred
        .iter()
        .any(|divisor| Fraction::integer(i128::from(value)).is_multiple_of(*divisor))
    {
        return None;
    }
    Some(json!(value))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_number(
    tc: &TestCase,
    minimum: Option<&Number>,
    exclusive_minimum: bool,
    maximum: Option<&Number>,
    exclusive_maximum: bool,
    multiple_of: &[Number],
    not_multiple_of: &[Number],
    excludes_integers: bool,
) -> Option<Value> {
    let low = match minimum {
        Some(bound) => Some(bound.as_f64()?),
        None => None,
    };
    let high = match maximum {
        Some(bound) => Some(bound.as_f64()?),
        None => None,
    };
    if let (Some(low), Some(high)) = (low, high) {
        if low > high || (low == high && (exclusive_minimum || exclusive_maximum)) {
            return None;
        }
    }
    let barred = excluded_divisors(not_multiple_of)?;
    // Several divisors admit exactly the multiples of their least common multiple.
    let mut combined: Option<Fraction> = None;
    for divisor in multiple_of {
        let each = Fraction::divisor(divisor)?;
        combined = Some(match combined {
            None => each,
            Some(current) => current.lcm(each)?,
        });
    }
    if let Some(grid) = combined {
        if barred.iter().any(|divisor| grid.is_multiple_of(*divisor)) {
            return None;
        }
        let step_size = grid.to_f64();
        let ceil_step = |bound: f64, exclusive: bool| {
            let mut step = (bound / step_size).ceil();
            if exclusive && step * step_size <= bound {
                step += 1.0;
            }
            step
        };
        let floor_step = |bound: f64, exclusive: bool| {
            let mut step = (bound / step_size).floor();
            if exclusive && step * step_size >= bound {
                step -= 1.0;
            }
            step
        };
        // The window defaults live in step space, so a sparse grid keeps its points.
        let (low_step, high_step) = match (low, high) {
            (Some(low), Some(high)) => (
                ceil_step(low, exclusive_minimum),
                floor_step(high, exclusive_maximum),
            ),
            (Some(low), None) => {
                let step = ceil_step(low, exclusive_minimum);
                (step, step + 16.0)
            }
            (None, Some(high)) => {
                let step = floor_step(high, exclusive_maximum);
                (step - 16.0, step)
            }
            (None, None) => (-8.0, 8.0),
        };
        if low_step > high_step {
            return None;
        }
        let high_step = high_step.min(low_step + 16.0);
        let step = tc.draw(
            gs::integers::<i64>()
                .min_value(low_step as i64)
                .max_value(high_step as i64),
        );
        let scaled = Fraction {
            numerator: i128::from(step).checked_mul(grid.numerator)?,
            denominator: grid.denominator,
        };
        if excludes_integers && scaled.is_integer() {
            return None;
        }
        if barred.iter().any(|divisor| scaled.is_multiple_of(*divisor)) {
            return None;
        }
        return Some(json!(step as f64 * step_size));
    }
    let mut low_value = low.unwrap_or_else(|| high.map_or(-8.0, |high| high - 16.0));
    let mut high_value = high.unwrap_or_else(|| (low_value + 16.0).max(8.0));
    // Stepping one float inward settles an exclusive bound before the draw engine sees it; a
    // window that is empty at float resolution declines here.
    if exclusive_minimum {
        low_value = low_value.next_up();
    }
    if exclusive_maximum {
        high_value = high_value.next_down();
    }
    if low_value > high_value {
        return None;
    }
    let mut value = tc.draw(
        gs::floats::<f64>()
            .min_value(low_value)
            .max_value(high_value),
    );
    if excludes_integers && value.fract() == 0.0 {
        let shifted = value + 0.5;
        if shifted > high_value {
            return None;
        }
        value = shifted;
    }
    if !barred.is_empty() {
        let fraction = Number::from_f64(value)
            .as_ref()
            .and_then(Fraction::from_number)?;
        if barred
            .iter()
            .any(|divisor| fraction.is_multiple_of(*divisor))
        {
            return None;
        }
    }
    Some(json!(value))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_string(
    tc: &TestCase,
    min_length: Option<&Number>,
    max_length: Option<&Number>,
    patterns: &[String],
    formats: &[String],
    excluded: &[String],
    content_media_types: &[String],
    content_encodings: &[String],
) -> Option<Value> {
    let floor = size_floor(min_length);
    if floor > MAX_SIZE {
        return None;
    }
    let ceiling = size_ceiling(max_length);
    if ceiling < floor {
        return None;
    }
    let admitted = |value: String| -> Option<Value> {
        if excluded.contains(&value) {
            return None;
        }
        Some(json!(value))
    };
    if !content_media_types.is_empty() || !content_encodings.is_empty() {
        let mut value = if content_media_types
            .iter()
            .any(|media| media == "application/json")
        {
            let mut decoded = tc.draw(arbitrary_instance());
            let mut serialized = serde_json::to_string(&decoded).expect("JSON serializes");
            // Nesting grows the serialized text up to the length floor.
            while (serialized.chars().count() as u64) < floor {
                decoded = Value::Array(vec![decoded]);
                serialized = serde_json::to_string(&decoded).expect("JSON serializes");
            }
            serialized
        } else {
            // A media type with no checker behind it constrains nothing.
            tc.draw(
                gs::text()
                    .min_size(floor as usize)
                    .max_size((floor + 6) as usize),
            )
        };
        if content_encodings
            .iter()
            .any(|encoding| encoding == "base64")
        {
            value = data_encoding::BASE64.encode(value.as_bytes());
        }
        let length = value.chars().count() as u64;
        if length < floor || length > ceiling {
            return None;
        }
        return admitted(value);
    }
    // A format with a generator behind it drives the draw; one without a checker constrains
    // nothing and falls through. Remaining facets are the wrapper's net.
    for format in formats {
        match format.as_str() {
            "email" | "idn-email" => return admitted(tc.draw(gs::emails())),
            "date" => {
                return admitted(
                    tc.draw(gs::sampled_from(vec![
                        "2024-01-15",
                        "1999-12-31",
                        "2000-02-29",
                    ]))
                    .to_owned(),
                );
            }
            "date-time" => {
                return admitted(
                    tc.draw(gs::sampled_from(vec![
                        "2024-01-15T10:30:00Z",
                        "1999-12-31T23:59:59.999Z",
                    ]))
                    .to_owned(),
                );
            }
            "time" => {
                return admitted(
                    tc.draw(gs::sampled_from(vec!["10:30:00Z", "23:59:59+01:00"]))
                        .to_owned(),
                );
            }
            "uuid" => {
                return admitted(
                    tc.draw(gs::sampled_from(vec![
                        "550e8400-e29b-41d4-a716-446655440000",
                        "00000000-0000-0000-0000-000000000000",
                    ]))
                    .to_owned(),
                );
            }
            "ipv4" => {
                return admitted(
                    tc.draw(gs::sampled_from(vec!["127.0.0.1", "255.255.255.255"]))
                        .to_owned(),
                );
            }
            "ipv6" => {
                return admitted(
                    tc.draw(gs::sampled_from(vec!["::1", "2001:db8::8a2e:370:7334"]))
                        .to_owned(),
                );
            }
            "hostname" | "idn-hostname" => {
                return admitted(
                    tc.draw(gs::sampled_from(vec!["example.com", "localhost"]))
                        .to_owned(),
                );
            }
            "uri" | "iri" => {
                return admitted(
                    tc.draw(gs::sampled_from(vec!["https://example.com/a", "urn:x:y"]))
                        .to_owned(),
                );
            }
            "json-pointer" => {
                return admitted(tc.draw(gs::sampled_from(vec!["", "/a/0"])).to_owned());
            }
            "regex" => {
                return admitted(tc.draw(gs::sampled_from(vec!["^a", "[0-9]+"])).to_owned());
            }
            _ => {}
        }
    }
    if !patterns.is_empty() {
        // One pattern drives the draw; the length window and the remaining patterns are the
        // wrapper's net. The validator's ECMA engine accepts patterns the draw engine cannot
        // parse, and those decline.
        let index = tc.draw(
            gs::integers::<usize>()
                .min_value(0)
                .max_value(patterns.len() - 1),
        );
        let pattern = &patterns[index];
        if regex::Regex::new(pattern).is_err() {
            return None;
        }
        return admitted(tc.draw(gs::from_regex(pattern)));
    }
    let length = tc.draw(
        gs::integers::<u64>()
            .min_value(floor)
            .max_value(ceiling.min(floor + 3)),
    );
    admitted(
        tc.draw(
            gs::text()
                .min_size(length as usize)
                .max_size(length as usize),
        ),
    )
}
