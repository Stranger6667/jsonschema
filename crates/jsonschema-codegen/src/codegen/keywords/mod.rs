pub(super) mod additional_items;
pub(super) mod additional_properties;
pub(super) mod all_of;
pub(super) mod any_of;
pub(super) mod array;
pub(super) mod const_;
pub(super) mod contains;
pub(super) mod content;
pub(super) mod dependencies;
pub(super) mod enum_;
pub(super) mod format;
pub(super) mod if_;
pub(super) mod items;
pub(super) mod max_items;
pub(super) mod max_length;
pub(super) mod max_properties;
pub(super) mod min_items;
pub(super) mod min_length;
pub(super) mod min_properties;
pub(super) mod minmax;
pub(super) mod multiple_of;
pub(super) mod not;
pub(super) mod number;
pub(super) mod object;
pub(super) mod one_of;
pub(super) mod pattern;
pub(super) mod pattern_properties;
pub(super) mod prefix_items;
pub(super) mod properties;
pub(super) mod property_names;
pub(super) mod ref_;
pub(super) mod required;
pub(super) mod string;
pub(super) mod type_;
pub(super) mod unevaluated;
pub(super) mod unevaluated_items;
pub(super) mod unevaluated_properties;
pub(super) mod unique_items;

use proc_macro2::TokenStream;
use quote::quote;

/// Combine a list of boolean `TokenStream` expressions with `||`.
/// Returns `false` for an empty list.
pub(super) fn combine_or(parts: Vec<TokenStream>) -> TokenStream {
    match parts.len() {
        0 => quote! { false },
        1 => parts.into_iter().next().unwrap_or_else(|| quote! { false }),
        _ => quote! { (#(#parts)||*) },
    }
}
