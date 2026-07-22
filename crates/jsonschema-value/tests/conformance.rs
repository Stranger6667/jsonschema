use std::borrow::Cow;

use jsonschema_value::{conformance, types::JsonType, Array, Json, Node, Object};
use serde_json::{Number, Value};

// A second representation, owning its data instead of borrowing `serde_json`. It implements only
// the methods without a default, so the defaults stay exercised.
enum Simple {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<Simple>),
    Object(Vec<(String, Simple)>),
}

impl Simple {
    fn from_value(value: &Value) -> Self {
        match value {
            Value::Null => Simple::Null,
            Value::Bool(boolean) => Simple::Bool(*boolean),
            Value::Number(number) => Simple::Number(number.clone()),
            Value::String(string) => Simple::String(string.clone()),
            Value::Array(items) => Simple::Array(items.iter().map(Simple::from_value).collect()),
            Value::Object(members) => Simple::Object(
                members
                    .iter()
                    .map(|(key, value)| (key.clone(), Simple::from_value(value)))
                    .collect(),
            ),
        }
    }

    fn to_json(&self) -> Value {
        match self {
            Simple::Null => Value::Null,
            Simple::Bool(boolean) => Value::Bool(*boolean),
            Simple::Number(number) => Value::Number(number.clone()),
            Simple::String(string) => Value::String(string.clone()),
            Simple::Array(items) => Value::Array(items.iter().map(Simple::to_json).collect()),
            Simple::Object(members) => Value::Object(
                members
                    .iter()
                    .map(|(key, value)| (key.clone(), value.to_json()))
                    .collect(),
            ),
        }
    }
}

struct SimpleJson;

impl Json for SimpleJson {
    type Node<'a> = &'a Simple;
    type PreparedKey = String;

    fn prepare_key(key: &str) -> String {
        key.to_owned()
    }
}

impl<'a> Node<'a, SimpleJson> for &'a Simple {
    type Object = &'a [(String, Simple)];
    type Array = &'a [Simple];

    fn as_object(&self) -> Option<&'a [(String, Simple)]> {
        match self {
            Simple::Object(members) => Some(members),
            _ => None,
        }
    }

    fn as_array(&self) -> Option<&'a [Simple]> {
        match self {
            Simple::Array(items) => Some(items),
            _ => None,
        }
    }

    fn as_string(&self) -> Option<Cow<'a, str>> {
        match self {
            Simple::String(string) => Some(Cow::Borrowed(string)),
            _ => None,
        }
    }

    fn as_number(&self) -> Option<Cow<'a, Number>> {
        match self {
            Simple::Number(number) => Some(Cow::Borrowed(number)),
            _ => None,
        }
    }

    fn as_boolean(&self) -> Option<bool> {
        match self {
            Simple::Bool(boolean) => Some(*boolean),
            _ => None,
        }
    }

    fn is_null(&self) -> bool {
        matches!(self, Simple::Null)
    }

    fn json_type(&self) -> JsonType {
        match self {
            Simple::Null => JsonType::Null,
            Simple::Bool(_) => JsonType::Boolean,
            Simple::Number(_) => JsonType::Number,
            Simple::String(_) => JsonType::String,
            Simple::Array(_) => JsonType::Array,
            Simple::Object(_) => JsonType::Object,
        }
    }

    fn to_value(&self) -> Cow<'a, Value> {
        Cow::Owned(Simple::to_json(self))
    }

    fn cache_key(&self) -> Option<usize> {
        Some(std::ptr::from_ref::<Simple>(self) as usize)
    }
}

impl<'a> Object<'a, SimpleJson> for &'a [(String, Simple)] {
    type Node = &'a Simple;
    type MemberName = &'a str;
    type MembersIter = SimpleMembers<'a>;

    fn len(&self) -> usize {
        <[(String, Simple)]>::len(self)
    }

    fn get(&self, key: &String) -> Option<&'a Simple> {
        self.iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }

    fn members(&self) -> SimpleMembers<'a> {
        SimpleMembers(self.iter())
    }
}

struct SimpleMembers<'a>(std::slice::Iter<'a, (String, Simple)>);

impl<'a> Iterator for SimpleMembers<'a> {
    type Item = (&'a str, &'a Simple);

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|(name, value)| (name.as_str(), value))
    }
}

impl<'a> Array<'a, SimpleJson> for &'a [Simple] {
    type Node = &'a Simple;
    type ElementsIter = std::slice::Iter<'a, Simple>;

    fn len(&self) -> usize {
        <[Simple]>::len(self)
    }

    fn elements(&self) -> std::slice::Iter<'a, Simple> {
        (*self).iter()
    }
}

#[test]
fn simple_representation_conforms() {
    let document = Simple::from_value(&conformance::document());
    conformance::assert_conformance::<SimpleJson>(&&document);
}

#[test]
fn serde_json_representation_conforms() {
    let document = conformance::document();
    conformance::assert_conformance::<jsonschema_value::SerdeJson>(&&document);
}
