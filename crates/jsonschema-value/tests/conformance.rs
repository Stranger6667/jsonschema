#![cfg(feature = "conformance")]

use std::borrow::Cow;

use jsonschema_value::{conformance, types::JsonType, Array, Json, Node, NodeIdentity, Object};
use serde_json::{Number, Value};

// A second representation, owning its data instead of borrowing `serde_json`. It implements only
// the methods without a default, so the defaults stay exercised.
#[derive(Default)]
enum Simple {
    #[default]
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
    type StringBuffer = Simple;

    fn prepare_key(key: &str) -> String {
        key.to_owned()
    }

    fn with_string_node<T>(buffer: &mut Simple, string: &str, f: impl FnOnce(&Simple) -> T) -> T {
        *buffer = Simple::String(string.to_owned());
        f(buffer)
    }
}

impl<'a> Node<'a, SimpleJson> for &'a Simple {
    type Object = &'a [(String, Simple)];
    type Array = &'a [Simple];
    type Number = &'a Number;

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

    fn as_number(&self) -> Option<&'a Number> {
        match self {
            Simple::Number(number) => Some(number),
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

    fn identity(&self) -> Option<NodeIdentity> {
        Some(NodeIdentity::new(
            std::ptr::from_ref::<Simple>(self) as usize
        ))
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

// A third representation, keeping its nodes in one arena addressed by index: handles are `Copy`
// stack temporaries, so no node has an address of its own.
#[derive(Default)]
struct Arena {
    slots: Vec<Slot>,
}

enum Slot {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<u32>),
    Object(Vec<(String, u32)>),
}

impl Arena {
    fn from_value(value: &Value) -> (Self, u32) {
        let mut arena = Arena::default();
        let root = arena.push(value);
        (arena, root)
    }

    fn push(&mut self, value: &Value) -> u32 {
        let slot = match value {
            Value::Null => Slot::Null,
            Value::Bool(boolean) => Slot::Bool(*boolean),
            Value::Number(number) => Slot::Number(number.clone()),
            Value::String(string) => Slot::String(string.clone()),
            Value::Array(items) => Slot::Array(items.iter().map(|item| self.push(item)).collect()),
            Value::Object(members) => Slot::Object(
                members
                    .iter()
                    .map(|(key, value)| (key.clone(), self.push(value)))
                    .collect(),
            ),
        };
        let index = u32::try_from(self.slots.len()).expect("arena index fits in u32");
        self.slots.push(slot);
        index
    }
}

#[derive(Clone, Copy)]
struct ArenaRef<'a> {
    arena: &'a Arena,
    index: u32,
}

impl<'a> ArenaRef<'a> {
    fn slot(self) -> &'a Slot {
        &self.arena.slots[self.index as usize]
    }

    fn to_json(self) -> Value {
        match self.slot() {
            Slot::Null => Value::Null,
            Slot::Bool(boolean) => Value::Bool(*boolean),
            Slot::Number(number) => Value::Number(number.clone()),
            Slot::String(string) => Value::String(string.clone()),
            Slot::Array(items) => Value::Array(
                items
                    .iter()
                    .map(|&index| self.sibling(index).to_json())
                    .collect(),
            ),
            Slot::Object(members) => Value::Object(
                members
                    .iter()
                    .map(|(key, index)| (key.clone(), self.sibling(*index).to_json()))
                    .collect(),
            ),
        }
    }

    fn sibling(self, index: u32) -> ArenaRef<'a> {
        ArenaRef {
            arena: self.arena,
            index,
        }
    }
}

struct ArenaJson;

impl Json for ArenaJson {
    type Node<'a> = ArenaRef<'a>;
    type PreparedKey = String;
    type StringBuffer = Arena;

    fn prepare_key(key: &str) -> String {
        key.to_owned()
    }

    fn with_string_node<T>(
        buffer: &mut Arena,
        string: &str,
        f: impl FnOnce(ArenaRef<'_>) -> T,
    ) -> T {
        buffer.slots.clear();
        buffer.slots.push(Slot::String(string.to_owned()));
        f(ArenaRef {
            arena: buffer,
            index: 0,
        })
    }
}

impl<'a> Node<'a, ArenaJson> for ArenaRef<'a> {
    type Object = ArenaMembers<'a>;
    type Array = ArenaItems<'a>;
    type Number = &'a Number;

    fn as_object(&self) -> Option<ArenaMembers<'a>> {
        match self.slot() {
            Slot::Object(members) => Some(ArenaMembers {
                arena: self.arena,
                members,
            }),
            _ => None,
        }
    }

    fn as_array(&self) -> Option<ArenaItems<'a>> {
        match self.slot() {
            Slot::Array(items) => Some(ArenaItems {
                arena: self.arena,
                items,
            }),
            _ => None,
        }
    }

    fn as_string(&self) -> Option<Cow<'a, str>> {
        match self.slot() {
            Slot::String(string) => Some(Cow::Borrowed(string)),
            _ => None,
        }
    }

    fn as_number(&self) -> Option<&'a Number> {
        match self.slot() {
            Slot::Number(number) => Some(number),
            _ => None,
        }
    }

    fn as_boolean(&self) -> Option<bool> {
        match self.slot() {
            Slot::Bool(boolean) => Some(*boolean),
            _ => None,
        }
    }

    fn is_null(&self) -> bool {
        matches!(self.slot(), Slot::Null)
    }

    fn json_type(&self) -> JsonType {
        match self.slot() {
            Slot::Null => JsonType::Null,
            Slot::Bool(_) => JsonType::Boolean,
            Slot::Number(_) => JsonType::Number,
            Slot::String(_) => JsonType::String,
            Slot::Array(_) => JsonType::Array,
            Slot::Object(_) => JsonType::Object,
        }
    }

    fn to_value(&self) -> Cow<'a, Value> {
        Cow::Owned(ArenaRef::to_json(*self))
    }

    fn identity(&self) -> Option<NodeIdentity> {
        Some(NodeIdentity::tagged(
            std::ptr::from_ref::<Arena>(self.arena) as usize,
            self.index,
        ))
    }
}

#[derive(Clone, Copy)]
struct ArenaMembers<'a> {
    arena: &'a Arena,
    members: &'a [(String, u32)],
}

impl<'a> Object<'a, ArenaJson> for ArenaMembers<'a> {
    type Node = ArenaRef<'a>;
    type MemberName = &'a str;
    type MembersIter = ArenaMembersIter<'a>;

    fn len(&self) -> usize {
        self.members.len()
    }

    fn get(&self, key: &String) -> Option<ArenaRef<'a>> {
        self.members
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, index)| ArenaRef {
                arena: self.arena,
                index: *index,
            })
    }

    fn members(&self) -> ArenaMembersIter<'a> {
        ArenaMembersIter {
            arena: self.arena,
            inner: self.members.iter(),
        }
    }
}

struct ArenaMembersIter<'a> {
    arena: &'a Arena,
    inner: std::slice::Iter<'a, (String, u32)>,
}

impl<'a> Iterator for ArenaMembersIter<'a> {
    type Item = (&'a str, ArenaRef<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(name, index)| {
            (
                name.as_str(),
                ArenaRef {
                    arena: self.arena,
                    index: *index,
                },
            )
        })
    }
}

#[derive(Clone, Copy)]
struct ArenaItems<'a> {
    arena: &'a Arena,
    items: &'a [u32],
}

impl<'a> Array<'a, ArenaJson> for ArenaItems<'a> {
    type Node = ArenaRef<'a>;
    type ElementsIter = ArenaItemsIter<'a>;

    fn len(&self) -> usize {
        self.items.len()
    }

    fn elements(&self) -> ArenaItemsIter<'a> {
        ArenaItemsIter {
            arena: self.arena,
            inner: self.items.iter(),
        }
    }
}

struct ArenaItemsIter<'a> {
    arena: &'a Arena,
    inner: std::slice::Iter<'a, u32>,
}

impl<'a> Iterator for ArenaItemsIter<'a> {
    type Item = ArenaRef<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|&index| ArenaRef {
            arena: self.arena,
            index,
        })
    }
}

#[test]
fn arena_representation_conforms() {
    let (arena, root) = Arena::from_value(&conformance::document());
    conformance::assert_conformance::<ArenaJson>(&ArenaRef {
        arena: &arena,
        index: root,
    });
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
