use std::{collections::HashSet, fs, path::Path};

use heck::ToSnakeCase;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use serde_json::Value;

use crate::idents;

/// Generate one module per `*.json` file under `path`, and one `#[test]` per
/// case object. Each case must carry a string `description`; everything else is
/// embedded verbatim and decoded by the runner.
pub(crate) fn generate(path: &str, runner: &syn::Ident) -> Result<TokenStream, String> {
    let dir = Path::new(path);
    if !dir.exists() {
        return Err(format!(
            "Canonical suite path does not exist: {}",
            dir.display()
        ));
    }

    let mut files: Vec<_> = fs::read_dir(dir)
        .map_err(|e| format!("Cannot read {}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();

    let mut functions = HashSet::new();
    let mut modules = Vec::with_capacity(files.len());

    for file in files {
        let stem = file
            .file_stem()
            .expect("json file has a stem")
            .to_string_lossy()
            .to_snake_case();
        let module_ident = format_ident!("{}", testsuite::sanitize_name(stem));

        let abs_path = fs::canonicalize(&file)
            .map_err(|e| format!("Cannot canonicalize {}: {e}", file.display()))?;
        let abs_path_str = abs_path
            .to_str()
            .ok_or_else(|| format!("Non-UTF-8 path: {}", abs_path.display()))?;

        let text = fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        // Embed each case's source text verbatim: re-serializing through the host `serde_json`
        // (no `arbitrary_precision`) rewrites past-u64 literals through f64.
        let cases: Vec<&serde_json::value::RawValue> =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;

        let mut tests = Vec::with_capacity(cases.len());
        for case_raw in &cases {
            let case: Value = serde_json::from_str(case_raw.get())
                .map_err(|e| format!("{}: {e}", file.display()))?;
            let description = case
                .get("description")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    format!(
                        "{}: a case is missing a string `description`",
                        file.display()
                    )
                })?;
            let base = testsuite::sanitize_name(description.to_snake_case());
            let name = idents::get_unique(&base, &mut functions);
            let test_ident = format_ident!("{}", name);
            let json = case_raw.get();

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

                const _: &[u8] = include_bytes!(#abs_path_str);

                #(#tests)*
            }
        });
    }

    Ok(quote! { #(#modules)* })
}

#[cfg(test)]
mod tests {
    use proc_macro2::{TokenStream, TokenTree};

    fn string_literals(stream: TokenStream, out: &mut Vec<String>) {
        for tree in stream {
            match tree {
                TokenTree::Group(group) => string_literals(group.stream(), out),
                TokenTree::Literal(literal) => {
                    if let syn::Lit::Str(text) = syn::Lit::new(literal) {
                        out.push(text.value());
                    }
                }
                _ => {}
            }
        }
    }

    // The host serde_json rewrites past-u64 literals through f64; cases must embed verbatim so
    // the generated tests exercise the declared numbers.
    #[test]
    fn embeds_case_text_verbatim() {
        let dir = std::env::temp_dir().join(format!("canonical-codegen-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let case_text = r#"{"description":"bignum","schema":{"const":18446744073709551617}}"#;
        std::fs::write(dir.join("cases.json"), format!("[{case_text}]")).expect("write case file");
        let runner = quote::format_ident!("runner");
        let generated = super::generate(dir.to_str().expect("utf-8 path"), &runner);
        std::fs::remove_dir_all(&dir).expect("remove temp dir");
        let mut literals = Vec::new();
        string_literals(generated.expect("generate"), &mut literals);
        assert!(
            literals.iter().any(|text| text == case_text),
            "expected verbatim case text {case_text}, got literals: {literals:?}"
        );
    }
}
