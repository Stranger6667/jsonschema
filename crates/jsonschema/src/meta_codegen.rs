//! Compile-time meta-schema validators for the bundled drafts, dispatched from
//! `meta::validator_for_draft` under the `macros` feature.

use crate::{Draft, Validator};

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

static DRAFT4_META_VALIDATOR: Validator = Validator::generated(
    Draft::Draft4,
    MetaDraft4::is_valid,
    MetaDraft4::validate,
    MetaDraft4::iter_errors,
    MetaDraft4::evaluate,
);
static DRAFT6_META_VALIDATOR: Validator = Validator::generated(
    Draft::Draft6,
    MetaDraft6::is_valid,
    MetaDraft6::validate,
    MetaDraft6::iter_errors,
    MetaDraft6::evaluate,
);
static DRAFT7_META_VALIDATOR: Validator = Validator::generated(
    Draft::Draft7,
    MetaDraft7::is_valid,
    MetaDraft7::validate,
    MetaDraft7::iter_errors,
    MetaDraft7::evaluate,
);
static DRAFT201909_META_VALIDATOR: Validator = Validator::generated(
    Draft::Draft201909,
    MetaDraft201909::is_valid,
    MetaDraft201909::validate,
    MetaDraft201909::iter_errors,
    MetaDraft201909::evaluate,
);
static DRAFT202012_META_VALIDATOR: Validator = Validator::generated(
    Draft::Draft202012,
    MetaDraft202012::is_valid,
    MetaDraft202012::validate,
    MetaDraft202012::iter_errors,
    MetaDraft202012::evaluate,
);

pub(crate) fn validator_for_draft(draft: Draft) -> &'static Validator {
    match draft {
        Draft::Draft4 => &DRAFT4_META_VALIDATOR,
        Draft::Draft6 => &DRAFT6_META_VALIDATOR,
        Draft::Draft7 => &DRAFT7_META_VALIDATOR,
        Draft::Draft201909 => &DRAFT201909_META_VALIDATOR,
        _ => &DRAFT202012_META_VALIDATOR,
    }
}
