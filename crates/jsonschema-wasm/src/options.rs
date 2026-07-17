use jsonschema::Draft;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct Options {
    pub(crate) draft: Option<String>,
    pub(crate) format_assertions: bool,
    pub(crate) ignore_unknown_formats: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            draft: None,
            format_assertions: false,
            ignore_unknown_formats: true,
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) const DRAFTS: [(&str, &str); 5] = [
    ("draft2020-12", "Draft 2020-12"),
    ("draft2019-09", "Draft 2019-09"),
    ("draft7", "Draft 7"),
    ("draft6", "Draft 6"),
    ("draft4", "Draft 4"),
];

pub(crate) fn draft_from_id(id: &str) -> Option<Draft> {
    Some(match id {
        "draft2020-12" => Draft::Draft202012,
        "draft2019-09" => Draft::Draft201909,
        "draft7" => Draft::Draft7,
        "draft6" => Draft::Draft6,
        "draft4" => Draft::Draft4,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_case::test_case;

    #[test_case("draft2020-12", Some(Draft::Draft202012))]
    #[test_case("draft2019-09", Some(Draft::Draft201909))]
    #[test_case("draft7", Some(Draft::Draft7))]
    #[test_case("draft6", Some(Draft::Draft6))]
    #[test_case("draft4", Some(Draft::Draft4))]
    #[test_case("nope", None)]
    fn draft_id_resolves(id: &str, expected: Option<Draft>) {
        assert_eq!(draft_from_id(id), expected);
    }

    #[test]
    fn options_default_ignores_unknown_formats() {
        let options: Options = serde_json::from_str("{}").unwrap();
        assert!(!options.format_assertions);
        assert!(options.ignore_unknown_formats);
    }

    #[test]
    fn options_parse_camel_case() {
        let options: Options = serde_json::from_str(
            r#"{"draft":"draft7","formatAssertions":true,"ignoreUnknownFormats":false}"#,
        )
        .unwrap();
        assert_eq!(options.draft.as_deref(), Some("draft7"));
        assert!(options.format_assertions);
        assert!(!options.ignore_unknown_formats);
    }
}
