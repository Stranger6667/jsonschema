use std::collections::HashSet;

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{parse_macro_input, ItemFn};
mod generator;
mod idents;
mod loader;
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
        Err(e) => {
            let err = e.to_string();
            return TokenStream::from(quote! {
                compile_error!(#err);
            });
        }
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
                    None => Err(format!("Unknown remote: {}", uri).into()),
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
            Err(e) => {
                let err = e.to_string();
                return TokenStream::from(quote! {
                    compile_error!(#err);
                });
            }
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
