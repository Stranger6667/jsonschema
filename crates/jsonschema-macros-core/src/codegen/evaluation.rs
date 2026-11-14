use proc_macro2::TokenStream;
use quote::quote;
use serde_json::Value;

use super::emit::ValueEmitter;
use crate::context::CompileContext;

/// The body of the generated `evaluate`: builds the root node into `__builder`.
pub(super) fn compile<E: ValueEmitter>(
    ctx: &CompileContext<'_, E>,
    schema: &Value,
) -> syn::Result<TokenStream> {
    let base = ctx.config.base_uri.as_str();
    // Without a base of its own the schema gets a synthetic one, which the output leaves out.
    let absolute = if base.starts_with("json-schema:///") {
        quote! { None }
    } else {
        let uri = format!("{}#", base.trim_end_matches('#'));
        quote! { Some(#uri) }
    };
    let errors = match schema {
        Value::Bool(true) => quote! { &[] },
        Value::Bool(false) => {
            let err_instance = E::err_instance(quote! { instance });
            quote! { &[__err::false_schema("", __paths::Location::new(), #err_instance)] }
        }
        Value::Object(keywords) => {
            if let Some(keyword) = keywords.keys().next() {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!("`evaluate` does not support the `{keyword}` keyword yet"),
                ));
            }
            quote! { &[] }
        }
        _ => {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "`evaluate` needs a schema that is a boolean or an object",
            ));
        }
    };
    Ok(quote! {
        static __LOCATION: ::std::sync::LazyLock<__eval::NodeLocation> =
            ::std::sync::LazyLock::new(|| __eval::NodeLocation::new("", #absolute));
        let mut __builder = __eval::Builder::new();
        let __root = __builder.node(&__LOCATION, #errors);
        __builder.finish(__root)
    })
}
