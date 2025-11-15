use proc_macro::TokenStream;
use quote::{format_ident, quote};
use std::collections::HashSet;
use syn::{parse_macro_input, ItemFn};

mod generator;
mod idents;
mod loader;
mod output_generator;
mod output_loader;
mod remotes;

/// A procedural macro that generates tests from
/// [JSON-Schema-Test-Suite](https://github.com/json-schema-org/JSON-Schema-Test-Suite).
#[proc_macro_attribute]
pub fn suite(args: TokenStream, input: TokenStream) -> TokenStream {
    let config = parse_macro_input!(args as testsuite::SuiteConfig);
    let test_func = parse_macro_input!(input as ItemFn);
    let test_func_ident = &test_func.sig.ident;

    let remotes = match remotes::generate(&config.path) {
        Ok(remotes) => remotes,
        Err(e) => return compile_error_ts(e.to_string()),
    };

    let mut output = quote! {
        #test_func

        #remotes

        struct TestsuiteRetriever;

        impl jsonschema::Retrieve for TestsuiteRetriever {
            fn retrieve(
                &self,
                uri: &jsonschema::Uri<String>,
            ) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
                static REMOTE_MAP: std::sync::LazyLock<std::collections::HashMap<&'static str, &'static str>> =
                    std::sync::LazyLock::new(|| {
                        let mut map = std::collections::HashMap::with_capacity(REMOTE_DOCUMENTS.len());
                        for (uri, contents) in REMOTE_DOCUMENTS {
                            map.insert(*uri, *contents);
                        }
                        map
                    });
                match REMOTE_MAP.get(uri.as_str()) {
                    Some(contents) => Ok(serde_json::from_str(contents)
                        .expect("Failed to parse remote schema")),
                    None => Err(format!("Unknown remote: {uri}").into()),
                }
            }
        }

        fn testsuite_retriever() -> TestsuiteRetriever {
            TestsuiteRetriever
        }

        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        use wasm_bindgen_test::wasm_bindgen_test;
    };
    // There are a lot of tests in the test suite
    let mut functions = HashSet::with_capacity(7200);
    for draft in &config.drafts {
        let suite_tree = match loader::load_suite(&config.path, draft) {
            Ok(tree) => tree,
            Err(e) => return compile_error_ts(e.to_string()),
        };
        let modules =
            generator::generate_modules(&suite_tree, &mut functions, &config.xfail, draft);
        let module_ident = format_ident!("{}", &draft.replace('-', "_"));
        output = quote! {
            #output

            mod #module_ident {
                use testsuite::Test;
                use super::#test_func_ident;

                #[inline]
                fn inner_test(test: &Test) {
                    #test_func_ident(test);
                }
                #modules
            }
        }
    }
    output.into()
}

/// A procedural macro that generates tests for the structured output test suite.
#[proc_macro_attribute]
pub fn output_suite(args: TokenStream, input: TokenStream) -> TokenStream {
    let config = parse_macro_input!(args as testsuite::SuiteConfig);
    let test_func = parse_macro_input!(input as ItemFn);
    let test_func_ident = &test_func.sig.ident;

    let mut output = quote! {
        #test_func

        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        use wasm_bindgen_test::wasm_bindgen_test;
    };

    let mut functions = HashSet::new();
    for version in &config.drafts {
        let suite_tree = match output_loader::load_cases(&config.path, version) {
            Ok(tree) => tree,
            Err(e) => return compile_error_ts(e.to_string()),
        };
        let docs = match output_loader::load_output_schema(&config.path, version) {
            Ok(docs) => docs,
            Err(e) => return compile_error_ts(e.to_string()),
        };
        let schema_literal = docs.schema;
        let remote_entries = docs.uris.iter().map(|uri| {
            quote! {
                testsuite::OutputRemote { uri: #uri, contents: OUTPUT_SCHEMA_JSON }
            }
        });
        let module_ident = format_ident!("{}", version.replace('-', "_"));
        let remotes_ident = format_ident!("OUTPUT_REMOTES");
        let modules = output_generator::generate_modules(
            &suite_tree,
            &mut functions,
            &config.xfail,
            version,
            &remotes_ident,
        );
        output = quote! {
            #output

            mod #module_ident {
                use testsuite::OutputTest;
                use super::#test_func_ident;

                const OUTPUT_SCHEMA_JSON: &str = #schema_literal;
                const #remotes_ident: &[testsuite::OutputRemote] = &[
                    #(#remote_entries),*
                ];

                #[inline]
                fn inner_test(test: OutputTest) {
                    #test_func_ident(test);
                }

                #modules
            }
        };
    }

    output.into()
}

fn compile_error_ts(err: impl quote::ToTokens) -> TokenStream {
    TokenStream::from(quote! {
        compile_error!(#err);
    })
}
