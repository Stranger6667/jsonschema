use proc_macro2::TokenStream;
use quote::quote;
use serde_json::Value;

use super::emit::ValueEmitter;
use crate::context::CompileContext;

/// Keywords whose units `evaluate` reports, in the runtime's priority order.
const LEAF_KEYWORDS: [&str; 3] = ["type", "minLength", "maxLength"];

/// The body of the generated `evaluate`. Keyword units take their errors from one run of the
/// schema's errors function, by schema location, so no keyword's error code is emitted twice.
pub(super) fn compile<E: ValueEmitter>(
    ctx: &CompileContext<'_, E>,
    schema: &Value,
) -> syn::Result<TokenStream> {
    if let Some(message) = unsupported(schema) {
        return Err(syn::Error::new(proc_macro2::Span::call_site(), message));
    }
    // The schema's own unit comes first.
    let mut pointers = vec![String::new()];
    if let Value::Object(keywords) = schema {
        pointers.extend(
            LEAF_KEYWORDS
                .iter()
                .filter(|keyword| keywords.contains_key(**keyword))
                .map(|keyword| format!("/{keyword}")),
        );
    }
    let base = ctx.config.base_uri.as_str();
    // Without a base of its own the schema gets a synthetic one, which the output leaves out.
    let base = if base.starts_with("json-schema:///") {
        quote! { None }
    } else {
        let base = base.trim_end_matches('#');
        quote! { Some(#base) }
    };
    Ok(quote! {
        static __LOCATIONS: __eval::Locations = __eval::Locations::new(&[#(#pointers),*], #base);
        let mut __errors = __err::Errors::new(false);
        let _ = errors(instance, &__paths::LazyLocation::new(), &mut __errors);
        __LOCATIONS.evaluate(&__errors.into_vec())
    })
}

/// Why the generated `evaluate` cannot handle `schema` yet, if it cannot.
pub(crate) fn unsupported(schema: &Value) -> Option<String> {
    match schema {
        Value::Bool(_) => None,
        Value::Object(keywords) => keywords
            .keys()
            .find(|keyword| !LEAF_KEYWORDS.contains(&keyword.as_str()))
            .map(|keyword| format!("`evaluate` does not support the `{keyword}` keyword yet")),
        _ => Some("`evaluate` needs a schema that is a boolean or an object".to_string()),
    }
}
