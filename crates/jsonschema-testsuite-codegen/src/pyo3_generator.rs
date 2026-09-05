use std::{fs, path::Path};

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use testsuite_internal::Case;

use crate::{files, loader};

pub(crate) struct SuiteCase {
    pub(crate) definition: TokenStream,
    pub(crate) entry: TokenStream,
}

pub(crate) fn generate_cases(
    suite_path: &str,
    draft: &str,
    remote_resources: &[(String, String)],
    next_index: &mut usize,
) -> Result<Vec<SuiteCase>, Box<dyn std::error::Error>> {
    let root = Path::new(suite_path).join("tests").join(draft);
    let draft_variant = match draft {
        "draft4" => quote! { referencing::Draft::Draft4 },
        "draft6" => quote! { referencing::Draft::Draft6 },
        "draft7" => quote! { referencing::Draft::Draft7 },
        "draft2019-09" => quote! { referencing::Draft::Draft201909 },
        _ => quote! { referencing::Draft::Draft202012 },
    };

    let mut cases = Vec::new();
    for path in files::json_files(&root)? {
        let relative = path
            .strip_prefix(&root)?
            .to_str()
            .ok_or("Invalid filename")?
            .replace('\\', "/");
        let contents = fs::read_to_string(&path)?;
        let parsed: Vec<Case> = serde_json::from_str(&loader::sanitize_lone_surrogates(&contents))?;
        let is_optional = relative.starts_with("optional/");

        for (index, case) in parsed.iter().enumerate() {
            let schema = serde_json::to_string(&case.schema).expect("Can't serialize JSON");
            let ident = format_ident!("Validator{}", *next_index);
            *next_index += 1;

            let resources_attr = if schema.contains("localhost:1234") {
                let entries = remote_resources
                    .iter()
                    .map(|(uri, contents)| quote! { #uri => { schema = #contents } });
                quote! { , resources = { #(#entries),* } }
            } else {
                quote! {}
            };
            let validate_formats_attr = if is_optional {
                quote! { , validate_formats = true }
            } else {
                quote! {}
            };

            let id = format!("{draft}|{relative}|{index}");
            cases.push(SuiteCase {
                definition: quote! {
                    #[jsonschema::validator(
                        schema = #schema,
                        draft = #draft_variant,
                        backend = Pyo3
                        #resources_attr
                        #validate_formats_attr
                    )]
                    struct #ident;
                },
                entry: quote! {
                    SuiteEntry {
                        id: #id,
                        is_valid: |instance| #ident::is_valid(instance),
                        validate: |instance| {
                            Ok(#ident::validate(instance)?.err().map(|error| error.to_string()))
                        },
                        iter_errors: |instance| {
                            Ok(#ident::iter_errors(instance)?
                                .map(|error| {
                                    (
                                        error.to_string(),
                                        error.schema_path().to_string(),
                                        error.instance_path().to_string(),
                                    )
                                })
                                .collect())
                        },
                    }
                },
            });
        }
    }
    Ok(cases)
}
