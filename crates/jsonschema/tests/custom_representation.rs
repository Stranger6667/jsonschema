// `tokio::test` needs a runtime unavailable on wasm targets.
#![cfg(all(feature = "resolve-async", not(target_arch = "wasm32")))]
use std::borrow::Cow;

use jsonschema::{
    json::{Array, Json, JsonNumber, Node, NodeIdentity, Object},
    AsyncRetrieve, JsonType, Keyword, Uri, ValidationError,
};
use serde_json::{json, Value};

#[derive(Default)]
enum ToyValue {
    #[default]
    Null,
    Boolean(bool),
    Number(f64),
    String(String),
    Array(Vec<ToyValue>),
    Object(Vec<(String, ToyValue)>),
}

struct ToyJson;

impl Json for ToyJson {
    type Node<'a> = &'a ToyValue;
    type PreparedKey = String;
    type StringBuffer = ToyValue;

    fn prepare_key(key: &str) -> String {
        key.to_owned()
    }

    fn with_string_node<T>(
        buffer: &mut ToyValue,
        string: &str,
        f: impl FnOnce(&ToyValue) -> T,
    ) -> T {
        *buffer = ToyValue::String(string.to_owned());
        f(buffer)
    }
}

struct ToyNumber(f64);

impl JsonNumber for ToyNumber {
    // Guarded by the fract/sign checks; out-of-range floats do not occur in these tests.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn as_u64(&self) -> Option<u64> {
        (self.0.fract() == 0.0 && self.0 >= 0.0).then_some(self.0 as u64)
    }
    #[allow(clippy::cast_possible_truncation)]
    fn as_i64(&self) -> Option<i64> {
        (self.0.fract() == 0.0).then_some(self.0 as i64)
    }
    fn as_f64(&self) -> Option<f64> {
        Some(self.0)
    }
    fn as_str(&self) -> Cow<'_, str> {
        Cow::Owned(self.0.to_string())
    }
    fn to_number(&self) -> Cow<'_, serde_json::Number> {
        Cow::Owned(serde_json::Number::from_f64(self.0).expect("finite"))
    }
}

impl<'a> Node<'a, ToyJson> for &'a ToyValue {
    type Object = &'a [(String, ToyValue)];
    type Array = &'a [ToyValue];
    type Number = ToyNumber;

    fn as_object(&self) -> Option<&'a [(String, ToyValue)]> {
        match self {
            ToyValue::Object(members) => Some(members),
            _ => None,
        }
    }
    fn as_array(&self) -> Option<&'a [ToyValue]> {
        match self {
            ToyValue::Array(items) => Some(items),
            _ => None,
        }
    }
    fn as_string(&self) -> Option<Cow<'a, str>> {
        match self {
            ToyValue::String(string) => Some(Cow::Borrowed(string)),
            _ => None,
        }
    }
    fn as_number(&self) -> Option<ToyNumber> {
        match self {
            ToyValue::Number(number) => Some(ToyNumber(*number)),
            _ => None,
        }
    }
    fn as_boolean(&self) -> Option<bool> {
        match self {
            ToyValue::Boolean(boolean) => Some(*boolean),
            _ => None,
        }
    }
    fn is_null(&self) -> bool {
        matches!(self, ToyValue::Null)
    }
    fn json_type(&self) -> JsonType {
        match self {
            ToyValue::Null => JsonType::Null,
            ToyValue::Boolean(_) => JsonType::Boolean,
            ToyValue::Number(_) => JsonType::Number,
            ToyValue::String(_) => JsonType::String,
            ToyValue::Array(_) => JsonType::Array,
            ToyValue::Object(_) => JsonType::Object,
        }
    }
    fn to_value(&self) -> Cow<'a, Value> {
        Cow::Owned(match self {
            ToyValue::Null => Value::Null,
            ToyValue::Boolean(boolean) => Value::Bool(*boolean),
            ToyValue::Number(number) => serde_json::Number::from_f64(*number)
                .map(Value::Number)
                .expect("finite"),
            ToyValue::String(string) => Value::String(string.clone()),
            ToyValue::Array(items) => Value::Array(
                items
                    .iter()
                    .map(|item| item.to_value().into_owned())
                    .collect(),
            ),
            ToyValue::Object(members) => Value::Object(
                members
                    .iter()
                    .map(|(name, value)| (name.clone(), value.to_value().into_owned()))
                    .collect(),
            ),
        })
    }
    fn identity(&self) -> Option<NodeIdentity> {
        Some(NodeIdentity::new(
            std::ptr::from_ref::<ToyValue>(*self) as usize
        ))
    }
}

impl<'a> Object<'a, ToyJson> for &'a [(String, ToyValue)] {
    type Node = &'a ToyValue;
    type MemberName = &'a str;
    type MembersIter = ToyMembersIter<'a>;

    fn len(&self) -> usize {
        <[(String, ToyValue)]>::len(self)
    }
    fn get(&self, key: &String) -> Option<&'a ToyValue> {
        self.iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }
    fn members(&self) -> ToyMembersIter<'a> {
        ToyMembersIter(self.iter())
    }
}

struct ToyMembersIter<'a>(std::slice::Iter<'a, (String, ToyValue)>);

impl<'a> Iterator for ToyMembersIter<'a> {
    type Item = (&'a str, &'a ToyValue);
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|(name, value)| (name.as_str(), value))
    }
}

impl<'a> Array<'a, ToyJson> for &'a [ToyValue] {
    type Node = &'a ToyValue;
    type ElementsIter = std::slice::Iter<'a, ToyValue>;

    fn len(&self) -> usize {
        <[ToyValue]>::len(self)
    }
    fn elements(&self) -> std::slice::Iter<'a, ToyValue> {
        self.iter()
    }
}

struct ToyRetriever;

#[cfg_attr(target_family = "wasm", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_family = "wasm"), async_trait::async_trait)]
impl AsyncRetrieve for ToyRetriever {
    async fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        assert_eq!(uri.as_str(), "https://example.com/name.json");
        Ok(json!({"type": "string", "minLength": 2}))
    }
}

#[tokio::test]
async fn async_build_validates_custom_representation() {
    let schema = json!({
        "type": "object",
        "properties": {"name": {"$ref": "https://example.com/name.json"}},
        "required": ["name"]
    });
    let validator = jsonschema::async_options_for::<ToyJson>()
        .with_retriever(ToyRetriever)
        .build(&schema)
        .await
        .expect("valid schema");

    let valid = ToyValue::Object(vec![("name".into(), ToyValue::String("bob".into()))]);
    let short = ToyValue::Object(vec![("name".into(), ToyValue::String("b".into()))]);
    let wrong_type = ToyValue::Object(vec![("name".into(), ToyValue::Number(1.0))]);
    let missing = ToyValue::Object(vec![]);

    assert!(validator.is_valid(&valid));
    assert!(!validator.is_valid(&short));
    assert!(!validator.is_valid(&wrong_type));
    let error = validator.validate(&missing).expect_err("missing required");
    assert_eq!(error.to_string(), "\"name\" is a required property");
}

struct NonEmptyObject;

impl<'i> Keyword<'i, ToyJson> for NonEmptyObject {
    fn validate(&self, instance: &'i ToyValue) -> Result<(), ValidationError<'i>> {
        if self.is_valid(instance) {
            Ok(())
        } else {
            Err(ValidationError::custom("object is empty"))
        }
    }
    fn is_valid(&self, instance: &'i ToyValue) -> bool {
        instance
            .as_object()
            .is_none_or(|members| !members.is_empty())
    }
}

#[test]
fn custom_keyword_validates_custom_representation() {
    let schema = json!({"type": "object", "nonEmpty": true});
    let validator = jsonschema::options_for::<ToyJson>()
        .with_keyword("nonEmpty", |_, _, _| Ok(Box::new(NonEmptyObject)))
        .build(&schema)
        .expect("valid schema");

    assert!(validator.is_valid(&ToyValue::Object(vec![("a".into(), ToyValue::Null)])));
    let empty = ToyValue::Object(vec![]);
    assert!(!validator.is_valid(&empty));
    let error = validator.validate(&empty).expect_err("empty object");
    assert_eq!(error.to_string(), "object is empty");
}

// The `propertyNames` subschema receives property names as string nodes of the same
// representation, so representation-specific keywords work there too.
#[test]
fn custom_keyword_inside_property_names_validates_names() {
    struct ShortName;

    impl<'i> Keyword<'i, ToyJson> for ShortName {
        fn validate(&self, instance: &'i ToyValue) -> Result<(), ValidationError<'i>> {
            if self.is_valid(instance) {
                Ok(())
            } else {
                Err(ValidationError::custom("name too long"))
            }
        }
        fn is_valid(&self, instance: &'i ToyValue) -> bool {
            instance.as_string().is_none_or(|name| name.len() <= 3)
        }
    }

    let schema = json!({"propertyNames": {"shortName": true}});
    let validator = jsonschema::options_for::<ToyJson>()
        .with_keyword("shortName", |_, _, _| Ok(Box::new(ShortName)))
        .build(&schema)
        .expect("valid schema");

    let short = ToyValue::Object(vec![("abc".into(), ToyValue::Null)]);
    let long = ToyValue::Object(vec![("abcdef".into(), ToyValue::Null)]);
    assert!(validator.is_valid(&short));
    assert!(!validator.is_valid(&long));
    let error = validator.validate(&long).expect_err("name too long");
    assert_eq!(error.to_string(), "name too long");
}

#[tokio::test]
async fn async_build_map_validates_custom_representation() {
    let schema = json!({
        "$defs": {
            "name": {"type": "string", "minLength": 2},
            "point": {"type": "array", "items": {"type": "number"}, "minItems": 2}
        }
    });
    let validators = jsonschema::async_options_for::<ToyJson>()
        .build_map(&schema)
        .await
        .expect("valid schema");

    let name = &validators["#/$defs/name"];
    assert!(name.is_valid(&ToyValue::String("bob".into())));
    assert!(!name.is_valid(&ToyValue::String("b".into())));

    let point = &validators["#/$defs/point"];
    assert!(point.is_valid(&ToyValue::Array(vec![
        ToyValue::Number(1.5),
        ToyValue::Number(2.0),
    ])));
    assert!(!point.is_valid(&ToyValue::Array(vec![ToyValue::Number(1.5)])));
    assert!(!point.is_valid(&ToyValue::Array(vec![
        ToyValue::Boolean(true),
        ToyValue::Null,
    ])));
}

// Property names run through the toy representation's own string nodes.
#[test]
fn property_names_validate_custom_representation() {
    let schema = json!({"propertyNames": {"minLength": 2, "pattern": "^[a-z]+$"}});
    let validator = jsonschema::options_for::<ToyJson>()
        .build(&schema)
        .expect("valid schema");

    let valid = ToyValue::Object(vec![("abc".into(), ToyValue::Null)]);
    let short = ToyValue::Object(vec![("a".into(), ToyValue::Null)]);
    let digits = ToyValue::Object(vec![("123".into(), ToyValue::Null)]);

    assert!(validator.is_valid(&valid));
    assert!(!validator.is_valid(&short));
    assert!(!validator.is_valid(&digits));
    let error = validator.validate(&short).expect_err("short name");
    assert_eq!(error.to_string(), "\"a\" is shorter than 2 characters");
}
