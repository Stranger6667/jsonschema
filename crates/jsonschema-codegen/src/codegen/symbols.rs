use proc_macro2::TokenStream;
use quote::quote;

/// Symbol table used by emitters to avoid hard-coding concrete runtime paths.
///
/// The current implementation targets `serde_json`, but this indirection keeps
/// emission sites ready for future non-serde backends.
#[derive(Clone)]
pub(crate) struct EmitSymbols {
    value: TokenStream,
    map: TokenStream,
    value_slice: TokenStream,
}

impl EmitSymbols {
    pub(crate) fn serde_json() -> Self {
        Self {
            value: quote! { serde_json::Value },
            map: quote! { serde_json::Map<String, serde_json::Value> },
            value_slice: quote! { [serde_json::Value] },
        }
    }

    pub(crate) fn value_ty(&self) -> TokenStream {
        self.value.clone()
    }

    pub(crate) fn map_ty(&self) -> TokenStream {
        self.map.clone()
    }

    pub(crate) fn value_slice_ty(&self) -> TokenStream {
        self.value_slice.clone()
    }
}
