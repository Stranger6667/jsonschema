use heck::ToSnakeCase;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use std::{collections::HashSet, fs};
use syn::ItemFn;

use crate::idents;

pub(crate) fn generate(path: &str, test_func: &ItemFn) -> Result<TokenStream, String> {
    let test_func_ident = &test_func.sig.ident;
    let mut entries = fs::read_dir(path)
        .map_err(|error| format!("Failed to read annotation suite at {path}: {error}"))?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect::<Vec<_>>();
    entries.sort_by_key(std::fs::DirEntry::path);

    let mut used_modules = HashSet::new();
    let modules = entries
        .into_iter()
        .map(|entry| {
            let filename = entry.file_name().to_string_lossy().into_owned();
            let module_name = idents::get_unique(
                &testsuite::sanitize_name(filename.trim_end_matches(".json").to_snake_case()),
                &mut used_modules,
            );
            let module_ident = format_ident!("{module_name}");
            let contents = fs::read_to_string(entry.path())
                .map_err(|error| format!("Failed to read {}: {error}", entry.path().display()))?;
            let document: serde_json::Value = serde_json::from_str(&contents)
                .map_err(|error| format!("Failed to parse {}: {error}", entry.path().display()))?;
            let cases = document["suite"]
                .as_array()
                .ok_or_else(|| format!("Annotation suite {filename} has no suite array"))?;
            let mut used_cases = HashSet::new();
            let case_modules = cases
                .iter()
                .map(|case| {
                    let description = case["description"].as_str().ok_or_else(|| {
                        format!("Annotation suite {filename} has a case without a description")
                    })?;
                    let case_name = idents::get_unique(
                        &testsuite::sanitize_name(description.to_snake_case()),
                        &mut used_cases,
                    );
                    let case_ident = format_ident!("{case_name}");
                    let schema = serde_json::to_string(&case["schema"]).map_err(|error| {
                        format!("Failed to serialize annotation schema: {error}")
                    })?;
                    let tests = case["tests"].as_array().ok_or_else(|| {
                        format!("Annotation case {description} has no tests array")
                    })?;
                    let test_functions = tests.iter().enumerate().map(|(index, test)| {
                        let test_ident = format_ident!("test_{index}");
                        let test_json = serde_json::to_string(test)
                            .expect("annotation test JSON should serialize");
                        let test_id = format!("{filename} / {description} / {index}");
                        quote! {
                            #[test]
                            fn #test_ident() {
                                let test_case: TestCase = serde_json::from_str(#test_json)
                                    .expect("Failed to load annotation test");
                                let evaluation = GeneratedValidator::evaluate(&test_case.instance);
                                super::super::#test_func_ident(&evaluation, &test_case, #test_id);
                            }
                        }
                    });
                    Ok(quote! {
                        mod #case_ident {
                            use super::*;
                            #[jsonschema::validator(
                                schema = #schema,
                                draft = referencing::Draft::Draft202012
                            )]
                            struct GeneratedValidator;
                            #(#test_functions)*
                        }
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(quote! {
                mod #module_ident {
                    use super::*;
                    #(#case_modules)*
                }
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    Ok(quote! {
        #test_func
        #(#modules)*
    })
}
