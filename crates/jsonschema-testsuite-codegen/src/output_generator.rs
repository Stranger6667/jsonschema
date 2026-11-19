use crate::{idents, output_loader};
use heck::ToSnakeCase;
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};
use std::collections::HashSet;

pub(crate) fn generate_modules(
    tree: &output_loader::OutputCaseTree,
    functions: &mut HashSet<String>,
    xfail: &[String],
    version: &str,
    remotes_ident: &Ident,
) -> TokenStream {
    let root_path = vec![version.to_string()];
    generate_nested_structure(tree, functions, &root_path, xfail, version, remotes_ident)
}

fn generate_nested_structure(
    tree: &output_loader::OutputCaseTree,
    functions: &mut HashSet<String>,
    current_path: &[String],
    xfail: &[String],
    version: &str,
    remotes_ident: &Ident,
) -> TokenStream {
    let modules = tree.iter().map(|(name, node)| {
        let module_name = testsuite::sanitize_name(name.to_snake_case());
        let module_ident = format_ident!("{}", module_name);
        let mut new_path = current_path.to_owned();
        new_path.push(module_name.clone());

        match node {
            output_loader::OutputCaseNode::Submodule(subtree) => {
                let submodules = generate_nested_structure(
                    subtree,
                    functions,
                    &new_path,
                    xfail,
                    version,
                    remotes_ident,
                );
                quote! {
                    mod #module_ident {
                        use super::*;

                        #submodules
                    }
                }
            }
            output_loader::OutputCaseNode::TestFile(file) => {
                let mut modules = HashSet::with_capacity(file.cases.len());
                let file_display = format!("{version}/content/{}", file.relative_path);
                let case_modules = file.cases.iter().map(|case| {
                    let base_module_name =
                        testsuite::sanitize_name(case.description.to_snake_case());
                    let module_name = idents::get_unique(&base_module_name, &mut modules);
                    let module_ident = format_ident!("{}", module_name);
                    let mut case_path = new_path.clone();
                    case_path.push(module_name.clone());

                    let schema =
                        serde_json::to_string(&case.schema).expect("Can't serialize JSON");
                    let case_description = &case.description;
                    let generated = generated_evaluate(&case.schema, version);

                    let test_functions = case.tests.iter().map(|test| {
                        let base_test_name =
                            testsuite::sanitize_name(test.description.to_snake_case());
                        let test_name = idents::get_unique(&base_test_name, functions);
                        let test_ident = format_ident!("test_{}", test_name);
                        case_path.push(test_name.clone());

                        let full_test_path = case_path.join("::");
                        let should_ignore = xfail.iter().any(|x| full_test_path.starts_with(x));
                        let ignore_attr = if should_ignore {
                            quote! { #[ignore] }
                        } else {
                            quote! {}
                        };
                        case_path.pop().expect("Empty path");

                        let test_description = &test.description;
                        let data =
                            serde_json::to_string(&test.data).expect("Can't serialize JSON");
                        let outputs = test.output.iter().map(|(format_name, schema)| {
                            let schema_json =
                                serde_json::to_string(schema).expect("Can't serialize JSON");
                            quote! {
                                testsuite::OutputFormat {
                                    format: #format_name,
                                    schema: serde_json::from_str(#schema_json)
                                        .expect("Failed to load JSON"),
                                }
                            }
                        });

                        quote! {
                            #ignore_attr
                            #[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
                            #[cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), wasm_bindgen_test::wasm_bindgen_test)]
                            fn #test_ident() {
                                let test = testsuite::OutputTest {
                                    version: #version,
                                    file: #file_display,
                                    schema: serde_json::from_str(#schema)
                                        .expect("Failed to load JSON"),
                                    case: #case_description,
                                    description: #test_description,
                                    data: serde_json::from_str(#data)
                                        .expect("Failed to load JSON"),
                                    outputs: vec![#(#outputs),*],
                                    remotes: #remotes_ident,
                                    evaluate: GENERATED,
                                };
                                inner_test(test);
                            }
                        }
                    });

                    quote! {
                        mod #module_ident {
                            use super::*;

                            #generated

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

/// The validator generated for `schema` as the version's tests build it, with `GENERATED` set to
/// its `evaluate`.
fn generated_evaluate(schema: &serde_json::Value, version: &str) -> TokenStream {
    // Version 1 cases run under Draft 2020-12, whatever their `$schema` says.
    let (schema, draft_attr) = if version == "v1" || version.starts_with("v1-") {
        let mut schema = schema.clone();
        if let Some(keywords) = schema.as_object_mut() {
            keywords.remove("$schema");
        }
        (schema, quote! { , draft = Draft202012 })
    } else {
        (schema.clone(), quote! {})
    };
    let schema = serde_json::to_string(&schema).expect("Can't serialize JSON");
    quote! {
        #[cfg(feature = "macros")]
        #[jsonschema::validator(schema = #schema #draft_attr)]
        struct Validator;

        #[cfg(feature = "macros")]
        fn generated(instance: &serde_json::Value) -> testsuite::EvaluateOutputs {
            let evaluation = Validator::evaluate(instance);
            testsuite::EvaluateOutputs {
                list: serde_json::to_value(evaluation.list()).expect("list output serializable"),
                hierarchical: serde_json::to_value(evaluation.hierarchical())
                    .expect("hierarchical output serializable"),
            }
        }

        #[cfg(feature = "macros")]
        const GENERATED: Option<fn(&serde_json::Value) -> testsuite::EvaluateOutputs> =
            Some(generated);
        #[cfg(not(feature = "macros"))]
        const GENERATED: Option<fn(&serde_json::Value) -> testsuite::EvaluateOutputs> = None;
    }
}
