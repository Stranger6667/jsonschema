use std::collections::HashSet;

use proc_macro::TokenStream;

use quote::{format_ident, quote};
use syn::{parse_macro_input, ItemFn};
mod generator;
mod idents;
mod loader;
mod mocks;

/// A procedural macro that generates tests from
/// [JSON-Schema-Test-Suite](https://github.com/json-schema-org/JSON-Schema-Test-Suite).
#[proc_macro_attribute]
pub fn suite(args: TokenStream, input: TokenStream) -> TokenStream {
    let config = parse_macro_input!(args as testsuite::SuiteConfig);
    let test_func = parse_macro_input!(input as ItemFn);
    let test_func_ident = &test_func.sig.ident;

    let mocks = match mocks::generate(&config.path) {
        Ok(mocks) => mocks,
        Err(e) => {
            let err = e.to_string();
            return TokenStream::from(quote! {
                compile_error!(#err);
            });
        }
    };

    let mut output = quote! {
        #test_func

        static MOCK: once_cell::sync::Lazy<mockito::Server> = once_cell::sync::Lazy::new(|| {
            #mocks
        });
    };
    // There are a lot of tests in the test suite
    let mut functions = HashSet::with_capacity(7200);

    for draft in config.drafts {
        let suite_tree = match loader::load_suite(&config.path, &draft) {
            Ok(tree) => tree,
            Err(e) => {
                let err = e.to_string();
                return TokenStream::from(quote! {
                    compile_error!(#err);
                });
            }
        };
        let modules =
            generator::generate_modules(&suite_tree, &mut functions, &config.xfail, &draft);
        let draft = format_ident!("{}", &draft.replace('-', "_"));
        output = quote! {
            #output

            mod #draft {
                use testsuite::Test;
                use super::{#test_func_ident, MOCK};

                #[inline]
                fn inner_test(test: Test) {
                    let _ = &*MOCK;
                    #test_func_ident(test);
                }
                #modules
            }
        }
    }
    output.into()
}
