use std::{collections::HashSet, fs, path::Path};

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use serde_json::Value;

use crate::{files, loader};

/// Cases of the annotation suite run under the newest dialect this crate implements.
const LATEST_DIALECT: u32 = 2020;

/// `lookup`: a function returning the generated `evaluate` of the annotation suite case with a
/// file name and description.
pub(crate) fn generate(path: &str, lookup: &syn::Ident) -> Result<TokenStream, String> {
    let mut modules = Vec::new();
    let mut arms = Vec::new();
    let mut seen = HashSet::new();
    for file in files::json_files(Path::new(path))? {
        let absolute_path = fs::canonicalize(&file)
            .map_err(|e| format!("Cannot canonicalize {}: {e}", file.display()))?;
        let absolute_path = absolute_path
            .to_str()
            .ok_or_else(|| format!("Non-UTF-8 path: {}", absolute_path.display()))?
            .to_string();
        let contents = fs::read_to_string(&file)
            .map_err(|e| format!("Cannot read {}: {e}", file.display()))?;
        let document: Value = serde_json::from_str(&contents)
            .map_err(|e| format!("Invalid JSON in {}: {e}", file.display()))?;
        let file_name = file
            .file_name()
            .expect("json file has a name")
            .to_string_lossy()
            .to_string();
        modules.push(quote! { const _: &str = include_str!(#absolute_path); });
        for case in document["suite"].as_array().into_iter().flatten() {
            let description = case["description"].as_str().unwrap_or_default().to_string();
            let compatible = case["compatibility"]
                .as_str()
                .is_none_or(|compatibility| loader::is_compatible(compatibility, LATEST_DIALECT));
            let schema = &case["schema"];
            if !compatible || !seen.insert((file_name.clone(), description.clone())) {
                continue;
            }
            let module_ident = format_ident!("case_{}", modules.len());
            let schema = serde_json::to_string(schema).expect("Can't serialize JSON");
            modules.push(quote! {
                #[cfg(feature = "macros")]
                mod #module_ident {
                    #[jsonschema::validator(schema = #schema)]
                    struct Validator;

                    pub(super) fn evaluate(instance: &serde_json::Value) -> testsuite::EvaluateOutputs {
                        let evaluation = Validator::evaluate(instance);
                        testsuite::EvaluateOutputs {
                            list: serde_json::to_value(evaluation.list())
                                .expect("list output serializable"),
                            hierarchical: serde_json::to_value(evaluation.hierarchical())
                                .expect("hierarchical output serializable"),
                        }
                    }
                }
            });
            arms.push(quote! {
                #[cfg(feature = "macros")]
                (#file_name, #description) => Some(#module_ident::evaluate),
            });
        }
    }
    Ok(quote! {
        #(#modules)*

        fn #lookup(
            file: &str,
            case: &str,
        ) -> Option<fn(&serde_json::Value) -> testsuite::EvaluateOutputs> {
            match (file, case) {
                #(#arms)*
                _ => None,
            }
        }
    })
}
