use crate::{idents, loader};
use heck::ToSnakeCase;
use std::collections::HashSet;

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

pub(crate) fn generate_modules(
    tree: &loader::TestCaseTree,
    functions: &mut HashSet<String>,
    xfail: &[String],
    draft: &str,
) -> TokenStream {
    let root_path = vec![draft.to_string()];
    generate_nested_structure(tree, functions, &root_path, xfail, draft)
}

fn generate_nested_structure(
    tree: &loader::TestCaseTree,
    functions: &mut HashSet<String>,
    current_path: &[String],
    xfail: &[String],
    draft: &str,
) -> TokenStream {
    let modules = tree.iter().map(|(name, node)| {
        let module_name = testsuite::sanitize_name(name.to_snake_case());
        let module_ident = format_ident!("{}", module_name);
        let mut new_path = current_path.to_owned();
        new_path.push(module_name.clone());

        match node {
            loader::TestCaseNode::Submodule(subtree) => {
                let submodules =
                    generate_nested_structure(subtree, functions, &new_path, xfail, draft);
                quote! {
                    mod #module_ident {
                        use super::*;

                        #submodules
                    }
                }
            }
            loader::TestCaseNode::TestFile(cases) => {
                let mut modules = HashSet::with_capacity(cases.len());
                let case_modules = cases.iter().map(|case| {
                    let base_module_name = testsuite::sanitize_name(case.description.to_snake_case());
                    let module_name = idents::get_unique(&base_module_name, &mut modules);
                    let module_ident = format_ident!("{}", module_name);
                    let mut case_path = new_path.clone();
                    case_path.push(module_name);

                    let schema = serde_json::to_string(&case.schema).expect("Can't serialize JSON");
                    let case_description = &case.description;

                    let test_functions = case.tests.iter().map(|test| {
                        let base_test_name = testsuite::sanitize_name(test.description.to_snake_case());
                        let test_name = idents::get_unique(&base_test_name, functions);
                        let test_ident = format_ident!("test_{}", test_name);
                        case_path.push(test_name.clone());

                        let full_test_path = case_path.join("::");
                        let is_optional = case_path.iter().any(|segment| segment == "optional");
                        let should_ignore = xfail.iter().any(|x| full_test_path.starts_with(x));
                        let ignore_attr = if should_ignore {
                            quote! { #[ignore = "known failure listed in xfail"] }
                        } else {
                            quote! {}
                        };
                        case_path.pop().expect("Empty path");

                        let test_description = &test.description;
                        let data = serde_json::to_string(&test.data).expect("Can't serialize JSON");
                        let valid = test.valid;

                        quote! {
                            #ignore_attr
                            #[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
                            #[cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), wasm_bindgen_test::wasm_bindgen_test)]
                            fn #test_ident() {
                                let test = testsuite::Test {
                                    draft: #draft,
                                    schema: serde_json::from_str(#schema).expect("Failed to load JSON"),
                                    is_optional: #is_optional,
                                    case: #case_description,
                                    description: #test_description,
                                    data: serde_json::from_str(#data).expect("Failed to load JSON"),
                                    valid: #valid,
                                };
                                inner_test(&test);
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
                    mod #module_ident {
                        use super::*;

                        #(#case_modules)*
                    }
                }
            }
        }
    });

    quote! {
        #(#modules)*
    }
}
