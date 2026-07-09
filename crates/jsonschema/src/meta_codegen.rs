//! Compile-time meta-schema validators for the bundled drafts, dispatched from
//! `meta::validator_for_draft` under the `macros` feature.

use crate::{Draft, ErrorIterator, ValidationError};
use serde_json::Value;

pub(crate) type ValidateFn = for<'i> fn(&'i Value) -> Result<(), ValidationError<'i>>;
pub(crate) type IterErrorsFn = for<'i> fn(&'i Value) -> ErrorIterator<'i>;

#[jsonschema_macros::validator(path = "metaschemas/draft4.json", draft = Draft4)]
struct MetaDraft4;

#[jsonschema_macros::validator(path = "metaschemas/draft6.json", draft = Draft6)]
struct MetaDraft6;

#[jsonschema_macros::validator(path = "metaschemas/draft7.json", draft = Draft7)]
struct MetaDraft7;

#[jsonschema_macros::validator(
    path = "metaschemas/draft2019-09/schema.json",
    draft = Draft201909,
    resources = {
        "https://json-schema.org/draft/2019-09/meta/core" => { path = "metaschemas/draft2019-09/meta/core.json" },
        "https://json-schema.org/draft/2019-09/meta/applicator" => { path = "metaschemas/draft2019-09/meta/applicator.json" },
        "https://json-schema.org/draft/2019-09/meta/validation" => { path = "metaschemas/draft2019-09/meta/validation.json" },
        "https://json-schema.org/draft/2019-09/meta/meta-data" => { path = "metaschemas/draft2019-09/meta/meta-data.json" },
        "https://json-schema.org/draft/2019-09/meta/format" => { path = "metaschemas/draft2019-09/meta/format.json" },
        "https://json-schema.org/draft/2019-09/meta/content" => { path = "metaschemas/draft2019-09/meta/content.json" },
    }
)]
struct MetaDraft201909;

#[jsonschema_macros::validator(
    path = "metaschemas/draft2020-12/schema.json",
    draft = Draft202012,
    resources = {
        "https://json-schema.org/draft/2020-12/meta/core" => { path = "metaschemas/draft2020-12/meta/core.json" },
        "https://json-schema.org/draft/2020-12/meta/applicator" => { path = "metaschemas/draft2020-12/meta/applicator.json" },
        "https://json-schema.org/draft/2020-12/meta/unevaluated" => { path = "metaschemas/draft2020-12/meta/unevaluated.json" },
        "https://json-schema.org/draft/2020-12/meta/validation" => { path = "metaschemas/draft2020-12/meta/validation.json" },
        "https://json-schema.org/draft/2020-12/meta/meta-data" => { path = "metaschemas/draft2020-12/meta/meta-data.json" },
        "https://json-schema.org/draft/2020-12/meta/format-annotation" => { path = "metaschemas/draft2020-12/meta/format-annotation.json" },
        "https://json-schema.org/draft/2020-12/meta/content" => { path = "metaschemas/draft2020-12/meta/content.json" },
    }
)]
struct MetaDraft202012;

// `Draft::Unknown` and future drafts fall through to Draft 2020-12, matching `validator_for_draft`.
pub(crate) fn is_valid_fn(draft: Draft) -> fn(&Value) -> bool {
    match draft {
        Draft::Draft4 => MetaDraft4::is_valid,
        Draft::Draft6 => MetaDraft6::is_valid,
        Draft::Draft7 => MetaDraft7::is_valid,
        Draft::Draft201909 => MetaDraft201909::is_valid,
        _ => MetaDraft202012::is_valid,
    }
}

pub(crate) fn validate_fn(draft: Draft) -> ValidateFn {
    match draft {
        Draft::Draft4 => MetaDraft4::validate,
        Draft::Draft6 => MetaDraft6::validate,
        Draft::Draft7 => MetaDraft7::validate,
        Draft::Draft201909 => MetaDraft201909::validate,
        _ => MetaDraft202012::validate,
    }
}

pub(crate) fn iter_errors_fn(draft: Draft) -> IterErrorsFn {
    match draft {
        Draft::Draft4 => MetaDraft4::iter_errors,
        Draft::Draft6 => MetaDraft6::iter_errors,
        Draft::Draft7 => MetaDraft7::iter_errors,
        Draft::Draft201909 => MetaDraft201909::iter_errors,
        _ => MetaDraft202012::iter_errors,
    }
}
