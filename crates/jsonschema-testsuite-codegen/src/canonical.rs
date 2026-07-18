use std::{collections::HashSet, fs, path::Path};

use heck::ToSnakeCase;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use serde_json::{value::RawValue, Value};

use crate::{files, idents};

/// Generate one module per `*.json` file under `path`, with one `#[test]` per case.
pub(crate) fn generate(path: &str, runner: &syn::Ident) -> Result<TokenStream, String> {
    let mut functions = HashSet::new();
    let mut modules = Vec::new();

    for file in files::json_files(Path::new(path))? {
        let module_name = file
            .file_stem()
            .expect("json file has a stem")
            .to_string_lossy()
            .to_snake_case();
        let module_ident = format_ident!("{}", testsuite::sanitize_name(module_name));

        let absolute_path = fs::canonicalize(&file)
            .map_err(|e| format!("Cannot canonicalize {}: {e}", file.display()))?;
        let absolute_path = absolute_path
            .to_str()
            .ok_or_else(|| format!("Non-UTF-8 path: {}", file.display()))?;

        let contents = fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        // Embed each case's source text verbatim: re-serializing through the host `serde_json`
        // (no `arbitrary_precision`) rewrites past-u64 literals through f64.
        let cases: Vec<&RawValue> =
            serde_json::from_str(&contents).map_err(|e| format!("{}: {e}", file.display()))?;

        let mut tests = Vec::with_capacity(cases.len());
        for case in &cases {
            let json = case.get();
            let decoded: Value =
                serde_json::from_str(json).map_err(|e| format!("{}: {e}", file.display()))?;
            let description = decoded["description"]
                .as_str()
                .expect("Case description must be a string");
            let test_ident = format_ident!(
                "{}",
                idents::get_unique(
                    &testsuite::sanitize_name(description.to_snake_case()),
                    &mut functions
                )
            );

            tests.push(quote! {
                #[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
                #[cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), wasm_bindgen_test::wasm_bindgen_test)]
                fn #test_ident() {
                    #runner(serde_json::from_str(#json).expect("Failed to load canonical case"));
                }
            });
        }

        modules.push(quote! {
            mod #module_ident {
                use super::*;

                const _: &[u8] = include_bytes!(#absolute_path);

                #(#tests)*
            }
        });
    }

    Ok(quote! { #(#modules)* })
}
