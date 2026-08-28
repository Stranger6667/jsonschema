use std::{
    borrow::Cow,
    sync::{
        atomic::{AtomicUsize, Ordering},
        OnceLock,
    },
};

use jsonschema::{
    json::{Array, Json, Node, NodeIdentity, Object},
    JsonType,
};
use jsonschema_value::LazyInstance;
use serde_json::{json, Number, Value};

// A node holding pre-serialized bytes rather than a live tree. The only representation here that
// opts into `Deferred`, so it is what exercises that branch.
struct DeferredSlot {
    bytes: Vec<u8>,
    make: fn(&[u8], u32) -> Value,
    shape: DeferredShape,
}

enum DeferredShape {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<u32>),
    Object(Vec<(String, u32)>),
}

#[derive(Default)]
struct DeferredDoc {
    slots: Vec<DeferredSlot>,
}

impl DeferredDoc {
    fn from_value(value: &Value) -> (Self, u32) {
        Self::from_value_with_make(value, deferred_make)
    }

    // Tests pass their own `make` so one test's decode count cannot see another's.
    fn from_value_with_make(value: &Value, make: fn(&[u8], u32) -> Value) -> (Self, u32) {
        let mut doc = DeferredDoc::default();
        let root = doc.push(value, make);
        (doc, root)
    }

    fn push(&mut self, value: &Value, make: fn(&[u8], u32) -> Value) -> u32 {
        let shape = match value {
            Value::Null => DeferredShape::Null,
            Value::Bool(boolean) => DeferredShape::Bool(*boolean),
            Value::Number(number) => DeferredShape::Number(number.clone()),
            Value::String(string) => DeferredShape::String(string.clone()),
            Value::Array(items) => {
                DeferredShape::Array(items.iter().map(|item| self.push(item, make)).collect())
            }
            Value::Object(members) => DeferredShape::Object(
                members
                    .iter()
                    .map(|(key, value)| (key.clone(), self.push(value, make)))
                    .collect(),
            ),
        };
        let bytes = serde_json::to_vec(value).expect("serializable");
        let index = u32::try_from(self.slots.len()).expect("fits in u32");
        self.slots.push(DeferredSlot { bytes, make, shape });
        index
    }
}

fn deferred_make(bytes: &[u8], _tag: u32) -> Value {
    serde_json::from_slice(bytes).expect("precomputed bytes are valid JSON")
}

#[derive(Clone, Copy)]
struct DeferredRef<'a> {
    doc: &'a DeferredDoc,
    index: u32,
}

impl<'a> DeferredRef<'a> {
    fn slot(self) -> &'a DeferredSlot {
        &self.doc.slots[self.index as usize]
    }
}

struct DeferredJson;

impl Json for DeferredJson {
    type Node<'a> = DeferredRef<'a>;
    type PreparedKey = String;
    type StringBuffer = DeferredDoc;

    fn prepare_key(key: &str) -> String {
        key.to_owned()
    }

    fn with_string_node<T>(
        buffer: &mut DeferredDoc,
        string: &str,
        f: impl FnOnce(DeferredRef<'_>) -> T,
    ) -> T {
        buffer.slots.clear();
        buffer.slots.push(DeferredSlot {
            bytes: serde_json::to_vec(string).expect("serializable"),
            make: deferred_make,
            shape: DeferredShape::String(string.to_owned()),
        });
        f(DeferredRef {
            doc: buffer,
            index: 0,
        })
    }
}

impl<'a> Node<'a, DeferredJson> for DeferredRef<'a> {
    type Object = DeferredMembers<'a>;
    type Array = DeferredItems<'a>;
    type Number = &'a Number;

    fn as_object(&self) -> Option<DeferredMembers<'a>> {
        match &self.slot().shape {
            DeferredShape::Object(members) => Some(DeferredMembers {
                doc: self.doc,
                members,
            }),
            _ => None,
        }
    }
    fn as_array(&self) -> Option<DeferredItems<'a>> {
        match &self.slot().shape {
            DeferredShape::Array(items) => Some(DeferredItems {
                doc: self.doc,
                items,
            }),
            _ => None,
        }
    }
    fn as_string(&self) -> Option<Cow<'a, str>> {
        match &self.slot().shape {
            DeferredShape::String(string) => Some(Cow::Borrowed(string)),
            _ => None,
        }
    }
    fn as_number(&self) -> Option<&'a Number> {
        match &self.slot().shape {
            DeferredShape::Number(number) => Some(number),
            _ => None,
        }
    }
    fn as_boolean(&self) -> Option<bool> {
        match &self.slot().shape {
            DeferredShape::Bool(boolean) => Some(*boolean),
            _ => None,
        }
    }
    fn is_null(&self) -> bool {
        matches!(self.slot().shape, DeferredShape::Null)
    }
    fn json_type(&self) -> JsonType {
        match &self.slot().shape {
            DeferredShape::Null => JsonType::Null,
            DeferredShape::Bool(_) => JsonType::Boolean,
            DeferredShape::Number(_) => JsonType::Number,
            DeferredShape::String(_) => JsonType::String,
            DeferredShape::Array(_) => JsonType::Array,
            DeferredShape::Object(_) => JsonType::Object,
        }
    }
    fn to_value(&self) -> Cow<'a, Value> {
        let slot = self.slot();
        Cow::Owned((slot.make)(&slot.bytes, 0))
    }
    fn identity(&self) -> Option<NodeIdentity> {
        Some(NodeIdentity::tagged(
            std::ptr::from_ref::<DeferredDoc>(self.doc) as usize,
            self.index,
        ))
    }
    // The opt-in under test: hand over the bytes instead of building the value now.
    fn lazy_value(&self) -> LazyInstance<'a> {
        let slot = self.slot();
        LazyInstance::Deferred {
            bytes: &slot.bytes,
            tag: 0,
            make: slot.make,
            cell: OnceLock::new(),
        }
    }
}

#[derive(Clone, Copy)]
struct DeferredMembers<'a> {
    doc: &'a DeferredDoc,
    members: &'a [(String, u32)],
}

impl<'a> Object<'a, DeferredJson> for DeferredMembers<'a> {
    type Node = DeferredRef<'a>;
    type MemberName = &'a str;
    type MembersIter = DeferredMembersIter<'a>;

    fn len(&self) -> usize {
        self.members.len()
    }
    fn get(&self, key: &String) -> Option<DeferredRef<'a>> {
        self.members
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, index)| DeferredRef {
                doc: self.doc,
                index: *index,
            })
    }
    fn members(&self) -> DeferredMembersIter<'a> {
        DeferredMembersIter {
            doc: self.doc,
            inner: self.members.iter(),
        }
    }
}

struct DeferredMembersIter<'a> {
    doc: &'a DeferredDoc,
    inner: std::slice::Iter<'a, (String, u32)>,
}

impl<'a> Iterator for DeferredMembersIter<'a> {
    type Item = (&'a str, DeferredRef<'a>);
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(name, index)| {
            (
                name.as_str(),
                DeferredRef {
                    doc: self.doc,
                    index: *index,
                },
            )
        })
    }
}

#[derive(Clone, Copy)]
struct DeferredItems<'a> {
    doc: &'a DeferredDoc,
    items: &'a [u32],
}

impl<'a> Array<'a, DeferredJson> for DeferredItems<'a> {
    type Node = DeferredRef<'a>;
    type ElementsIter = DeferredItemsIter<'a>;

    fn len(&self) -> usize {
        self.items.len()
    }
    fn elements(&self) -> DeferredItemsIter<'a> {
        DeferredItemsIter {
            doc: self.doc,
            inner: self.items.iter(),
        }
    }
}

struct DeferredItemsIter<'a> {
    doc: &'a DeferredDoc,
    inner: std::slice::Iter<'a, u32>,
}

impl<'a> Iterator for DeferredItemsIter<'a> {
    type Item = DeferredRef<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|&index| DeferredRef {
            doc: self.doc,
            index,
        })
    }
}

#[test]
fn deferred_instance_equals_eager_equivalent() {
    let schema = json!({"type": "object", "required": ["name"]});
    let instance = json!({"extra": 1});

    let validator = jsonschema::options_for::<DeferredJson>()
        .build(&schema)
        .expect("valid schema");
    let (doc, root) = DeferredDoc::from_value(&instance);
    let error = validator
        .validate(DeferredRef {
            doc: &doc,
            index: root,
        })
        .expect_err("missing required property");

    assert_eq!(error.instance().as_ref(), &instance);
}

// Decoding happens zero times until `instance()` is read, then exactly once however often after.
#[test]
fn deferred_instance_is_built_once_on_first_read() {
    static DECODE_CALLS: AtomicUsize = AtomicUsize::new(0);

    fn counting_make(bytes: &[u8], _tag: u32) -> Value {
        DECODE_CALLS.fetch_add(1, Ordering::SeqCst);
        serde_json::from_slice(bytes).expect("precomputed bytes are valid JSON")
    }

    let schema = json!({"type": "object", "required": ["name"]});
    let instance = json!({"extra": 1});

    let validator = jsonschema::options_for::<DeferredJson>()
        .build(&schema)
        .expect("valid schema");
    let (doc, root) = DeferredDoc::from_value_with_make(&instance, counting_make);
    let error = validator
        .validate(DeferredRef {
            doc: &doc,
            index: root,
        })
        .expect_err("missing required property");

    // `Required`'s `Display` and the path accessors never touch the instance.
    assert_eq!(DECODE_CALLS.load(Ordering::SeqCst), 0);
    assert_eq!(error.to_string(), "\"name\" is a required property");
    assert!(matches!(
        error.kind(),
        jsonschema::error::ValidationErrorKind::Required { property }
            if property == &Value::String("name".to_string())
    ));
    assert_eq!(error.instance_path().as_str(), "");
    assert_eq!(error.schema_path().as_str(), "/required");
    assert_eq!(DECODE_CALLS.load(Ordering::SeqCst), 0);

    let first = error.instance().clone().into_owned();
    assert_eq!(DECODE_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(first, instance);

    let second = error.instance().clone().into_owned();
    assert_eq!(DECODE_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(second, first);
}

#[test]
fn deferred_errors_resolve_instance_after_being_collected() {
    let schema = json!({"type": "object", "required": ["name", "age"]});
    let instance = json!({});

    let validator = jsonschema::options_for::<DeferredJson>()
        .build(&schema)
        .expect("valid schema");
    let (doc, root) = DeferredDoc::from_value(&instance);
    let errors: Vec<_> = validator
        .iter_errors(DeferredRef {
            doc: &doc,
            index: root,
        })
        .collect();

    assert_eq!(errors.len(), 2);
    for error in &errors {
        assert_eq!(error.instance().as_ref(), &instance);
    }
}

// `ValidationError` must stay `Send + Sync`; that is why `LazyInstance` uses `OnceLock`.
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<jsonschema::ValidationError<'static>>();
};
