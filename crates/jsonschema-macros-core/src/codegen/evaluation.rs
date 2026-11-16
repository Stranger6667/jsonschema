use proc_macro2::TokenStream;
use quote::quote;
use referencing::Draft;
use serde_json::Value;

use super::emit::ValueEmitter;
use crate::context::CompileContext;

/// Keywords whose units `evaluate` reports, in the runtime's priority order.
const LEAF_KEYWORDS: [&str; 17] = [
    "type",
    "const",
    "enum",
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "multipleOf",
    "minLength",
    "maxLength",
    "minItems",
    "maxItems",
    "minProperties",
    "maxProperties",
    "required",
    "pattern",
    "uniqueItems",
];

/// The body of the generated `evaluate`. Keyword units take their errors from one run of the
/// schema's errors function, by schema location, so no keyword's error code is emitted twice.
pub(super) fn compile<E: ValueEmitter>(
    ctx: &CompileContext<'_, E>,
    schema: &Value,
) -> syn::Result<TokenStream> {
    if let Some(message) = unsupported(schema, ctx.draft) {
        return Err(syn::Error::new(proc_macro2::Span::call_site(), message));
    }
    // The schema's own unit comes first.
    let mut units = vec![vec![String::new()]];
    if let Value::Object(keywords) = schema {
        units.extend(
            LEAF_KEYWORDS
                .iter()
                .filter_map(|keyword| unit(keyword, keywords.get(*keyword)?, ctx.draft)),
        );
    }
    let units = units.iter().map(|paths| quote! { &[#(#paths),*] });
    let base = ctx.config.base_uri.as_str();
    // Without a base of its own the schema gets a synthetic one, which the output leaves out.
    let base = if base.starts_with("json-schema:///") {
        quote! { None }
    } else {
        let base = base.trim_end_matches('#');
        quote! { Some(#base) }
    };
    Ok(quote! {
        static __LOCATIONS: __eval::Locations = __eval::Locations::new(&[#(#units),*], #base);
        let mut __errors = __err::Errors::new(false);
        let _ = errors(instance, &__paths::LazyLocation::new(), &mut __errors);
        __LOCATIONS.evaluate(&__errors.into_vec())
    })
}

/// A keyword's unit: its location, then any other location its errors are reported at. `None`
/// when the runtime gives the keyword no unit.
fn unit(keyword: &str, value: &Value, draft: Draft) -> Option<Vec<String>> {
    let location = format!("/{keyword}");
    match (draft, keyword) {
        // Draft 4's boolean flags make a bound exclusive; the bound's unit reports their errors.
        (Draft::Draft4, "exclusiveMinimum" | "exclusiveMaximum") => None,
        (Draft::Draft4, "minimum") => Some(vec![location, "/exclusiveMinimum".to_string()]),
        (Draft::Draft4, "maximum") => Some(vec![location, "/exclusiveMaximum".to_string()]),
        (_, "uniqueItems") if value == &Value::Bool(false) => None,
        _ => Some(vec![location]),
    }
}

/// Why the generated `evaluate` cannot handle `schema` yet, if it cannot.
pub(crate) fn unsupported(schema: &Value, draft: Draft) -> Option<String> {
    match schema {
        Value::Bool(_) => None,
        Value::Object(keywords) => keywords
            .iter()
            .find(|(keyword, value)| !supports(keyword, value, draft))
            .map(|(keyword, _)| format!("`evaluate` does not support the `{keyword}` keyword yet")),
        _ => Some("`evaluate` needs a schema that is a boolean or an object".to_string()),
    }
}

fn supports(keyword: &str, value: &Value, draft: Draft) -> bool {
    match keyword {
        // Not a Draft 4 keyword, so the runtime reports it as an annotation.
        "const" => draft != Draft::Draft4,
        // The runtime skips a divisor beyond `f64`.
        "multipleOf" => value.as_f64().is_some_and(f64::is_finite),
        _ => LEAF_KEYWORDS.contains(&keyword),
    }
}
