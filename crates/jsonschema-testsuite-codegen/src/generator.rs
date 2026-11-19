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
            loader::TestCaseNode::TestFile(file) => {
                let cases = &file.cases;
                let relative = &file.relative;
                let mut modules = HashSet::with_capacity(cases.len());
                let case_modules = cases.iter().map(|case| {
                    let base_module_name = testsuite::sanitize_name(case.description.to_snake_case());
                    let module_name = idents::get_unique(&base_module_name, &mut modules);
                    let module_ident = format_ident!("{}", module_name);
                    let mut case_path = new_path.clone();
                    case_path.push(module_name);

                    let schema = serde_json::to_string(&case.schema).expect("Can't serialize JSON");
                    let case_description = &case.description;

                    let externals: Vec<(&String, String)> = case
                        .external_schemas
                        .iter()
                        .map(|(uri, schema)| {
                            (uri, serde_json::to_string(schema).expect("Can't serialize JSON"))
                        })
                        .collect();
                    let resources_attr = if externals.is_empty() {
                        quote! {}
                    } else {
                        let entries = externals
                            .iter()
                            .map(|(uri, contents)| quote! { #uri => { schema = #contents } });
                        quote! { , resources = { #(#entries),* } }
                    };
                    let external_schemas = externals.iter().map(|(uri, contents)| {
                        quote! { (#uri, serde_json::from_str(#contents).expect("Failed to load JSON")) }
                    });
                    let external_schemas = quote! { vec![#(#external_schemas),*] };

                    let (validate_formats_attr, validate_formats) =
                        match loader::validate_formats(case) {
                            Some(value) => (
                                quote! { , validate_formats = #value },
                                quote! { Some(#value) },
                            ),
                            None => (quote! {}, quote! { None }),
                        };

                    let draft_variant = match draft {
                        "draft4" => quote! { referencing::Draft::Draft4 },
                        "draft6" => quote! { referencing::Draft::Draft6 },
                        "draft7" => quote! { referencing::Draft::Draft7 },
                        "draft2019-09" => quote! { referencing::Draft::Draft201909 },
                        _ => quote! { referencing::Draft::Draft202012 },
                    };

                    let test_functions = case.tests.iter().map(|test| {
                        let base_test_name = testsuite::sanitize_name(test.description.to_snake_case());
                        let test_name = idents::get_unique(&base_test_name, functions);
                        let test_ident = format_ident!("test_{}", test_name);
                        case_path.push(test_name.clone());

                        let full_test_path = case_path.join("::");
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
                                    file: #relative,
                                    schema: serde_json::from_str(#schema).expect("Failed to load JSON"),
                                    external_schemas: #external_schemas,
                                    validate_formats: #validate_formats,
                                    case: #case_description,
                                    description: #test_description,
                                    data: serde_json::from_str(#data).expect("Failed to load JSON"),
                                    valid: #valid,
                                };

                                #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
                                let validator: Option<Box<dyn testsuite::CodegenValidator>> = None;

                                #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
                                let validator: Option<Box<dyn testsuite::CodegenValidator>> =
                                    Some(Box::new(Validator));

                                inner_test(&test, validator);
                            }
                        }
                    });

                    quote! {
                        mod #module_ident {
                            use super::*;

                            // The generated validators are more than rustc can hold on a hosted
                            // runner for this target; the generated code is target-independent
                            // and compared on every other one.
                            #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
                            #[jsonschema::validator(
                                schema = #schema,
                                draft = #draft_variant
                                #resources_attr
                                #validate_formats_attr
                            )]
                            struct Validator;

                            #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
                            impl testsuite::CodegenValidator for Validator {
                                fn is_valid(&self, instance: &serde_json::Value) -> bool {
                                    Self::is_valid(instance)
                                }
                                fn validate(
                                    &self,
                                    instance: &serde_json::Value,
                                ) -> Result<(), Box<dyn std::any::Any + Send + Sync>> {
                                    Self::validate(instance).map_err(|e| {
                                        Box::new(e.to_owned())
                                            as Box<dyn std::any::Any + Send + Sync>
                                    })
                                }
                                fn iter_errors(
                                    &self,
                                    instance: &serde_json::Value,
                                ) -> Vec<testsuite::ErrorReport> {
                                    Self::iter_errors(instance)
                                        .map(|e| testsuite::ErrorReport {
                                            message: e.to_string(),
                                            schema_path: e.schema_path().to_string(),
                                            evaluation_path: e.evaluation_path().to_string(),
                                            absolute_keyword_location: e
                                                .absolute_keyword_location()
                                                .map(|uri| uri.as_str().to_string()),
                                            instance_path: e.instance_path().to_string(),
                                        })
                                        .collect()
                                }
                                fn evaluate(
                                    &self,
                                    instance: &serde_json::Value,
                                ) -> (serde_json::Value, serde_json::Value) {
                                    let evaluation = Self::evaluate(instance);
                                    (
                                        serde_json::to_value(evaluation.list())
                                            .expect("List output should serialize"),
                                        serde_json::to_value(evaluation.hierarchical())
                                            .expect("Hierarchical output should serialize"),
                                    )
                                }
                            }

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
