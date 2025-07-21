use heck::ToSnakeCase;

use super::loader::TestCase;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

pub(crate) fn generate_modules(
    suite: &[TestCase],
    xfail: &[String],
    draft: &str,
    draft_idx: usize,
) -> TokenStream {
    let current_path = vec![draft.to_string()];
    let modules = suite
        .iter()
        .enumerate()
        .map(|(case_idx, TestCase { name, case })| {
            let module_name = testsuite_common::sanitize_name(name.to_snake_case());
            let mut new_path = current_path.clone();
            new_path.push(module_name.clone());
            let module_ident = format_ident!("{}", module_name);

            let registry = serde_json::to_string(&case.registry).expect("Can't serialize JSON");

            let test_functions = case.tests.iter().enumerate().map(|(idx, test)| {
                let test_name = format!("test_{idx}_{draft_idx}_{case_idx}");
                let test_ident = format_ident!("{test_name}");
                let mut case_path = new_path.clone();
                case_path.push(test_name.clone());
                let full_test_path = case_path.join("::");
                let should_ignore = xfail.iter().any(|x| full_test_path.starts_with(x));
                let ignore_attr = if should_ignore {
                    quote! { #[ignore = "xfail"] }
                } else {
                    quote! {}
                };
                case_path.pop().expect("Empty path");

                let base_uri = if let Some(base_uri) = test.base_uri.as_ref() {
                    quote! {Some(#base_uri)}
                } else {
                    quote! {None}
                };
                let reference = &test.reference;
                let target = serde_json::to_string(&test.target).expect("Can't serialize JSON");
                let then = if let Some(then) = &test.then {
                    serde_json::to_string(then).expect("Can't serialize JSON")
                } else {
                    "null".to_string()
                };
                let error = if let Some(error) = test.error {
                    quote! {Some(#error)}
                } else {
                    quote! {None}
                };
                quote! {
                    #ignore_attr
                    #[test]
                    fn #test_ident() {
                        let test = referencing_testsuite::Test {
                            registry: serde_json::from_str(#registry).expect("Failed to load JSON"),
                            base_uri: #base_uri,
                            reference: #reference,
                            target: serde_json::from_str(#target).expect("Failed to load JSON"),
                            then: serde_json::from_str(#then).expect("Failed to load JSON"),
                            error: #error,
                        };
                        inner_test(#draft, test);
                    }
                }
            });

            quote! {
                mod #module_ident {
                    use super::*;

                    #(#test_functions)*
                }
            }
        });

    quote! {
        #(#modules)*
    }
}
