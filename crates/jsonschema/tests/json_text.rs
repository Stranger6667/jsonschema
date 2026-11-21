#![cfg(feature = "json-text")]
use jsonschema::{
    json::{JsonText, JsonTextNode, Node, Object},
    Keyword, ValidationError,
};
use serde_json::json;
use test_case::test_case;

struct EvenLength;

impl<'i> Keyword<'i, JsonText> for EvenLength {
    fn validate(&self, instance: JsonTextNode<'i>) -> Result<(), ValidationError<'i>> {
        if self.is_valid(instance) {
            Ok(())
        } else {
            Err(ValidationError::custom("odd number of members"))
        }
    }

    fn is_valid(&self, instance: JsonTextNode<'i>) -> bool {
        instance
            .as_object()
            .is_none_or(|object| object.len() % 2 == 0)
    }
}

#[test_case(r#"{"a": 1, "b": 2}"#, true ; "even")]
#[test_case(r#"{"a": 1}"#, false ; "odd")]
#[test_case(r#"{"a": 1, "a": 2}"#, false ; "a repeated key counts once")]
#[test_case("[1]", true ; "not an object")]
fn custom_keywords_read_json_text(text: &str, valid: bool) {
    let validator = jsonschema::options_for::<JsonText>()
        .with_keyword("evenLength", |_, _, _| Ok(Box::new(EvenLength)))
        .build(&json!({"evenLength": true}))
        .expect("valid schema");
    let document = JsonText::parse(text.as_bytes()).expect("valid JSON");
    assert_eq!(validator.is_valid(document.root()), valid);
    assert_eq!(validator.validate(document.root()).is_ok(), valid);
    let errors: Vec<String> = validator
        .iter_errors(document.root())
        .map(|error| error.to_string())
        .collect();
    let expected: Vec<String> = if valid {
        vec![]
    } else {
        vec!["odd number of members".to_string()]
    };
    assert_eq!(errors, expected);
}
