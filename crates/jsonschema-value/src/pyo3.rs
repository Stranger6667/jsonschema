//! Validating native Python objects.
//!
//! Nodes are `Borrowed` handles anchored at the root instance, valid for the whole validation call.
#![allow(unsafe_code)]
// FFI size/index values cross `isize`/`usize`/`u64`; the conversions are bounded by the container.
#![allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::elidable_lifetime_names
)]

use std::{
    borrow::Cow,
    cell::RefCell,
    hash::{Hash, Hasher},
    str::FromStr,
    sync::atomic::{AtomicPtr, Ordering},
};

use ahash::{AHashSet, AHasher};

use pyo3::{
    exceptions::PyValueError,
    ffi,
    prelude::*,
    sync::PyOnceLock,
    types::{PyDict, PyString},
    Borrowed,
};
use serde_json::{Map, Number, Value};

use crate::{cmp, types::JsonType, Array, Json, JsonNumber, Node, Object};

type PyNode<'py> = Borrowed<'py, 'py, PyAny>;

pub struct Pyo3;

impl Json for Pyo3 {
    type Node<'py> = PyNode<'py>;
    type PreparedKey = Py<PyString>;

    fn prepare_key(key: &str) -> Py<PyString> {
        Python::attach(|py| PyString::intern(py, key).unbind())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ObjType {
    Str,
    Int,
    Bool,
    Null,
    Float,
    List,
    Dict,
    Tuple,
    Enum,
    Decimal,
    Unknown,
}

struct TypePtrs {
    str_: *mut ffi::PyTypeObject,
    int: *mut ffi::PyTypeObject,
    bool_: *mut ffi::PyTypeObject,
    float: *mut ffi::PyTypeObject,
    list: *mut ffi::PyTypeObject,
    dict: *mut ffi::PyTypeObject,
    tuple: *mut ffi::PyTypeObject,
    enum_base: *mut ffi::PyTypeObject,
    decimal: *mut ffi::PyTypeObject,
    none: *mut ffi::PyObject,
    true_: *mut ffi::PyObject,
}

// Raw type pointers are process-stable for the interpreter's lifetime.
unsafe impl Send for TypePtrs {}
unsafe impl Sync for TypePtrs {}

static TYPES: PyOnceLock<TypePtrs> = PyOnceLock::new();
// Acquire/Release publish the initialized `TypePtrs`; free-threaded builds run this concurrently.
static TYPES_PTR: AtomicPtr<TypePtrs> = AtomicPtr::new(std::ptr::null_mut());

#[inline]
fn types(py: Python<'_>) -> &'static TypePtrs {
    let cached = TYPES_PTR.load(Ordering::Acquire);
    if !cached.is_null() {
        return unsafe { &*cached };
    }
    types_init(py)
}

#[cold]
fn types_init(py: Python<'_>) -> &'static TypePtrs {
    let t = TYPES.get_or_init(py, || {
        let ptr = |import: &str, attribute: &str| {
            py.import(import)
                .and_then(|m| m.getattr(attribute))
                .map_or(std::ptr::null_mut(), |c| {
                    c.as_ptr().cast::<ffi::PyTypeObject>()
                })
        };
        TypePtrs {
            str_: py.get_type::<PyString>().as_type_ptr(),
            int: ptr("builtins", "int"),
            bool_: ptr("builtins", "bool"),
            float: ptr("builtins", "float"),
            list: ptr("builtins", "list"),
            dict: py.get_type::<PyDict>().as_type_ptr(),
            tuple: ptr("builtins", "tuple"),
            enum_base: ptr("enum", "Enum"),
            decimal: ptr("decimal", "Decimal"),
            none: unsafe { ffi::Py_None() },
            true_: unsafe { ffi::Py_True() },
        }
    });
    TYPES_PTR.store(std::ptr::from_ref(t).cast_mut(), Ordering::Release);
    t
}

fn object_type(node: Borrowed<'_, '_, PyAny>) -> ObjType {
    let ptr = node.as_ptr();
    let t = types(node.py());
    if ptr == t.none {
        return ObjType::Null;
    }
    let ty = unsafe { ffi::Py_TYPE(ptr) };
    // Dict & str dominate real-world JSON.
    if ty == t.dict {
        ObjType::Dict
    } else if ty == t.str_ {
        ObjType::Str
    } else if ty == t.list {
        ObjType::List
    } else if ty == t.int {
        ObjType::Int
    } else if ty == t.bool_ {
        ObjType::Bool
    } else if ty == t.float {
        ObjType::Float
    } else if ty == t.tuple {
        ObjType::Tuple
    } else if !t.decimal.is_null() && ty == t.decimal {
        ObjType::Decimal
    } else if is_subtype(ty, t.dict) {
        ObjType::Dict
    } else if is_subtype(ty, t.list) {
        ObjType::List
    } else if !t.enum_base.is_null() && is_subtype(ty, t.enum_base) {
        ObjType::Enum
    } else {
        ObjType::Unknown
    }
}

fn is_subtype(ty: *mut ffi::PyTypeObject, base: *mut ffi::PyTypeObject) -> bool {
    !base.is_null() && unsafe { ffi::PyType_IsSubtype(ty, base) != 0 }
}

// Unwrap an `Enum` member to its `.value`, which its parent's attribute slot holds, so the borrow
// outlives the owned handles dropped here.
fn resolved<'py>(node: PyNode<'py>) -> PyNode<'py> {
    let mut current = node.to_owned();
    let mut seen: Vec<*mut ffi::PyObject> = Vec::new();
    while object_type(current.as_borrowed()) == ObjType::Enum {
        // A `value` property may hand back a member already on the chain.
        if seen.contains(&current.as_ptr()) {
            record_value_error("Enum value resolves to itself");
            return inert(current.py());
        }
        seen.push(current.as_ptr());
        match current.getattr("value") {
            Ok(value) => current = value,
            Err(error) => {
                record_value_error(&format!("Failed to access enum value: {error}"));
                return inert(current.py());
            }
        }
    }
    unsafe { Borrowed::from_ptr(current.py(), current.as_ptr()) }
}

// Stand-in for a node that could not be read; an error is pending, so the result is discarded.
fn inert(py: Python<'_>) -> PyNode<'_> {
    unsafe { Borrowed::from_ptr(py, ffi::Py_None()) }
}

thread_local! {
    // First error raised while inspecting an instance; the binding clears it before a call and
    // raises it after.
    static PENDING_ERROR: RefCell<Option<PyErr>> = const { RefCell::new(None) };
}

fn record_error(error: PyErr) {
    PENDING_ERROR.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            *slot = Some(error);
        }
    });
}

fn record_value_error(message: &str) {
    record_error(PyValueError::new_err(message.to_owned()));
}

/// Take the error recorded while inspecting an instance, if any.
#[must_use]
pub fn take_pending_error() -> Option<PyErr> {
    PENDING_ERROR.with(|slot| slot.borrow_mut().take())
}

/// RAII scope giving each call its own error slot and restoring the caller's on exit, so a keyword
/// that re-enters validation cannot consume the outer call's error.
pub struct PendingErrorScope {
    saved: Option<PyErr>,
}

impl PendingErrorScope {
    #[must_use]
    pub fn enter() -> Self {
        Self {
            saved: take_pending_error(),
        }
    }
}

impl Drop for PendingErrorScope {
    fn drop(&mut self) {
        let saved = self.saved.take();
        PENDING_ERROR.with(|slot| *slot.borrow_mut() = saved);
    }
}

/// Record the errors reachable from `node` itself.
///
/// Nested ones surface only when a keyword reads that value; scanning the whole instance would cost
/// a full traversal per call.
pub fn probe_root(node: Borrowed<'_, '_, PyAny>) {
    match object_type(node) {
        ObjType::Str => drop(str_ref(node)),
        ObjType::Enum => drop(resolved(node)),
        ObjType::Unknown => record_unsupported(node),
        // Key types only.
        ObjType::Dict => {
            let mut position: ffi::Py_ssize_t = 0;
            let mut key: *mut ffi::PyObject = std::ptr::null_mut();
            let mut value: *mut ffi::PyObject = std::ptr::null_mut();
            while unsafe {
                ffi::PyDict_Next(
                    node.as_ptr(),
                    &raw mut position,
                    &raw mut key,
                    &raw mut value,
                ) != 0
            } {
                let key = unsafe { Borrowed::from_ptr(node.py(), key) };
                if !key_is_usable(key) {
                    record_unusable_key(key);
                    break;
                }
            }
        }
        _ => {}
    }
}

// The string content of a `str` object, borrowed for `'py`. Sound when the object outlives `'py`.
unsafe fn str_ref_ptr<'py>(object: *mut ffi::PyObject) -> Option<&'py str> {
    let mut size: ffi::Py_ssize_t = 0;
    let ptr = ffi::PyUnicode_AsUTF8AndSize(object, &raw mut size);
    if ptr.is_null() {
        record_error(PyErr::fetch(Python::assume_attached()));
        return None;
    }
    let slice = std::slice::from_raw_parts(ptr.cast::<u8>(), size as usize);
    // `PyUnicode_AsUTF8AndSize` yields CPython-validated UTF-8; lone surrogates fail to null above.
    let text = std::str::from_utf8_unchecked(slice);
    Some(std::mem::transmute::<&str, &'py str>(text))
}

fn str_ref<'py>(node: PyNode<'py>) -> Option<&'py str> {
    unsafe { str_ref_ptr(node.as_ptr()) }
}

fn obj_str_number(node: Borrowed<'_, '_, PyAny>) -> Option<Number> {
    let text = node.str().ok()?;
    Number::from_str(text.to_str().ok()?).ok()
}

fn number_of(node: PyNode<'_>) -> Option<Number> {
    match object_type(node) {
        ObjType::Int => {
            let value = unsafe { ffi::PyLong_AsLongLong(node.as_ptr()) };
            if value == -1 && unsafe { !ffi::PyErr_Occurred().is_null() } {
                unsafe { ffi::PyErr_Clear() };
                obj_str_number(node)
            } else {
                Some(Number::from(value))
            }
        }
        ObjType::Float => {
            let value = unsafe { ffi::PyFloat_AsDouble(node.as_ptr()) };
            Number::from_f64(value)
        }
        ObjType::Decimal => obj_str_number(node),
        ObjType::Enum => number_of(resolved(node)),
        ObjType::Unknown => {
            record_unsupported(node);
            None
        }
        _ => None,
    }
}

fn py_to_value(node: PyNode<'_>) -> Value {
    match object_type(node) {
        ObjType::Null => Value::Null,
        ObjType::Unknown => {
            record_unsupported(node);
            Value::Null
        }
        ObjType::Bool => Value::Bool(node.as_ptr() == types(node.py()).true_),
        ObjType::Int | ObjType::Float | ObjType::Decimal => {
            number_of(node).map_or(Value::Null, Value::Number)
        }
        ObjType::Str => Value::String(str_ref(node).unwrap_or_default().to_owned()),
        ty @ (ObjType::List | ObjType::Tuple) => {
            let array = PyArray::new(node, ty == ObjType::Tuple);
            Value::Array(array.elements().map(py_to_value).collect())
        }
        ObjType::Dict => {
            let dict = unsafe { node.cast_unchecked::<PyDict>() };
            let mut map = Map::new();
            for (key, value) in dict.members() {
                map.insert(key.into_owned(), py_to_value(value));
            }
            Value::Object(map)
        }
        ObjType::Enum => py_to_value(resolved(node)),
    }
}

fn key_to_string(key: PyNode<'_>) -> String {
    match object_type(key) {
        ObjType::Str => str_ref(key).unwrap_or_default().to_owned(),
        ObjType::Enum if is_str_instance(key) => {
            let value = resolved(key);
            if object_type(value) == ObjType::Str {
                return str_ref(value).unwrap_or_default().to_owned();
            }
            record_unusable_key(key);
            String::new()
        }
        _ => {
            record_unusable_key(key);
            String::new()
        }
    }
}

fn record_unsupported(node: PyNode<'_>) {
    record_value_error(&format!("Unsupported type: '{}'", type_name(node)));
}

fn record_unusable_key(key: PyNode<'_>) {
    record_value_error(&format!(
        "Dict key must be str or str enum. Got '{}'",
        type_name(key)
    ));
}

fn type_name(node: PyNode<'_>) -> String {
    node.get_type()
        .name()
        .map(|name| name.to_string())
        .unwrap_or_default()
}

fn is_str_instance(node: PyNode<'_>) -> bool {
    let ty = unsafe { ffi::Py_TYPE(node.as_ptr()) };
    ty == types(node.py()).str_ || is_subtype(ty, types(node.py()).str_)
}

// A key usable as a property name: a string, or a string-valued enum.
fn key_is_usable(key: PyNode<'_>) -> bool {
    match object_type(key) {
        ObjType::Str => true,
        ObjType::Enum => is_str_instance(key),
        _ => false,
    }
}

impl<'py> Node<'py, Pyo3> for PyNode<'py> {
    type Object = Borrowed<'py, 'py, PyDict>;
    type Array = PyArray<'py>;
    type Number = PyNumber<'py>;

    fn as_object(&self) -> Option<Self::Object> {
        match object_type(*self) {
            ObjType::Dict => Some(unsafe { self.cast_unchecked::<PyDict>() }),
            ObjType::Enum => resolved(*self).as_object(),
            ObjType::Unknown => {
                record_unsupported(*self);
                None
            }
            _ => None,
        }
    }

    fn as_array(&self) -> Option<PyArray<'py>> {
        match object_type(*self) {
            ObjType::List => Some(PyArray::new(*self, false)),
            ObjType::Tuple => Some(PyArray::new(*self, true)),
            ObjType::Enum => resolved(*self).as_array(),
            ObjType::Unknown => {
                record_unsupported(*self);
                None
            }
            _ => None,
        }
    }

    fn as_string(&self) -> Option<Cow<'py, str>> {
        match object_type(*self) {
            ObjType::Str => str_ref(*self).map(Cow::Borrowed),
            ObjType::Enum => resolved(*self).as_string(),
            ObjType::Unknown => {
                record_unsupported(*self);
                None
            }
            _ => None,
        }
    }

    fn as_number(&self) -> Option<PyNumber<'py>> {
        let kind = object_type(*self);
        match kind {
            ObjType::Int => Some(PyNumber { node: *self, kind }),
            // Non-finite floats and `Decimal("nan")` are not JSON numbers.
            ObjType::Float => unsafe { ffi::PyFloat_AsDouble(self.as_ptr()) }
                .is_finite()
                .then_some(PyNumber { node: *self, kind }),
            ObjType::Decimal => number_of(*self)
                .is_some()
                .then_some(PyNumber { node: *self, kind }),
            ObjType::Enum => resolved(*self).as_number(),
            ObjType::Unknown => {
                record_unsupported(*self);
                None
            }
            _ => None,
        }
    }

    fn is_number(&self) -> bool {
        let ptr = self.as_ptr();
        let t = types(self.py());
        let ty = unsafe { ffi::Py_TYPE(ptr) };
        // Numbers dominate this hot path (`type: number` over data); check them before the full ladder.
        if ty == t.float {
            // A non-finite float is not a JSON number, matching `Number::from_f64`.
            return unsafe { ffi::PyFloat_AsDouble(ptr) }.is_finite();
        }
        if ty == t.int {
            let value = unsafe { ffi::PyLong_AsLongLong(ptr) };
            // Fits `i64` -> a number; only oversized ints need the string fallback.
            if value == -1 && unsafe { !ffi::PyErr_Occurred().is_null() } {
                unsafe { ffi::PyErr_Clear() };
                return number_of(*self).is_some();
            }
            return true;
        }
        match object_type(*self) {
            ObjType::Decimal => number_of(*self).is_some(),
            ObjType::Enum => resolved(*self).is_number(),
            _ => false,
        }
    }

    fn as_boolean(&self) -> Option<bool> {
        match object_type(*self) {
            ObjType::Bool => Some(self.as_ptr() == types(self.py()).true_),
            ObjType::Enum => resolved(*self).as_boolean(),
            ObjType::Unknown => {
                record_unsupported(*self);
                None
            }
            _ => None,
        }
    }

    fn is_null(&self) -> bool {
        if self.as_ptr() == types(self.py()).none {
            return true;
        }
        match object_type(*self) {
            ObjType::Enum => resolved(*self).is_null(),
            ObjType::Unknown => {
                record_unsupported(*self);
                false
            }
            _ => false,
        }
    }

    fn json_type(&self) -> JsonType {
        match object_type(*self) {
            ObjType::Null => JsonType::Null,
            ObjType::Bool => JsonType::Boolean,
            ObjType::Int | ObjType::Float | ObjType::Decimal => JsonType::Number,
            ObjType::Str => JsonType::String,
            ObjType::List | ObjType::Tuple => JsonType::Array,
            ObjType::Dict => JsonType::Object,
            ObjType::Unknown => {
                record_unsupported(*self);
                JsonType::Object
            }
            ObjType::Enum => resolved(*self).json_type(),
        }
    }

    fn string_length(&self) -> Option<u64> {
        match object_type(*self) {
            ObjType::Str => Some(unsafe { ffi::PyUnicode_GetLength(self.as_ptr()) }.max(0) as u64),
            ObjType::Enum => resolved(*self).string_length(),
            _ => None,
        }
    }

    fn equals_value(&self, expected: &Value) -> bool {
        match expected {
            Value::Null => self.is_null(),
            Value::Bool(b) => self.as_boolean() == Some(*b),
            Value::Number(n) => self
                .as_number()
                .is_some_and(|got| cmp::equal_numbers(&got, n)),
            Value::String(s) => self
                .as_string()
                .is_some_and(|got| got.as_ref() == s.as_str()),
            Value::Array(items) => self.as_array().is_some_and(|arr| {
                arr.len() == items.len()
                    && arr
                        .elements()
                        .zip(items)
                        .all(|(element, expected)| element.equals_value(expected))
            }),
            Value::Object(map) => self.as_object().is_some_and(|object| {
                object.len() == map.len()
                    && object.members().all(|(key, value)| {
                        map.get(key.as_ref()).is_some_and(|e| value.equals_value(e))
                    })
            }),
        }
    }

    fn to_value(&self) -> Cow<'py, Value> {
        Cow::Owned(py_to_value(*self))
    }

    fn cache_key(&self) -> Option<usize> {
        Some(self.as_ptr() as usize)
    }
}

/// A Python number read in place; `kind` is the classification already done by `as_number`.
pub struct PyNumber<'py> {
    node: PyNode<'py>,
    kind: ObjType,
}

impl JsonNumber for PyNumber<'_> {
    fn as_u64(&self) -> Option<u64> {
        match self.kind {
            ObjType::Int => {
                let value = unsafe { ffi::PyLong_AsUnsignedLongLong(self.node.as_ptr()) };
                if value == u64::MAX && unsafe { !ffi::PyErr_Occurred().is_null() } {
                    unsafe { ffi::PyErr_Clear() };
                    return None;
                }
                Some(value)
            }
            _ => self.to_number().as_u64(),
        }
    }

    fn as_i64(&self) -> Option<i64> {
        match self.kind {
            ObjType::Int => {
                let value = unsafe { ffi::PyLong_AsLongLong(self.node.as_ptr()) };
                if value == -1 && unsafe { !ffi::PyErr_Occurred().is_null() } {
                    unsafe { ffi::PyErr_Clear() };
                    return None;
                }
                Some(value)
            }
            _ => self.to_number().as_i64(),
        }
    }

    fn as_f64(&self) -> Option<f64> {
        match self.kind {
            ObjType::Float => Some(unsafe { ffi::PyFloat_AsDouble(self.node.as_ptr()) }),
            _ => self.to_number().as_f64(),
        }
    }

    fn as_str(&self) -> Cow<'_, str> {
        self.node
            .str()
            .ok()
            .and_then(|text| text.to_str().map(|text| Cow::Owned(text.to_owned())).ok())
            .unwrap_or(Cow::Borrowed(""))
    }

    fn to_number(&self) -> Cow<'_, Number> {
        number_of(self.node).map_or(Cow::Owned(Number::from(0)), Cow::Owned)
    }
}

// Lazy `PyDict_Next` iteration; keys and values are borrowed.
pub struct PyMembers<'py> {
    dict: Borrowed<'py, 'py, PyDict>,
    pos: ffi::Py_ssize_t,
}

impl<'py> Iterator for PyMembers<'py> {
    type Item = (Cow<'py, str>, PyNode<'py>);

    fn next(&mut self) -> Option<Self::Item> {
        let mut key: *mut ffi::PyObject = std::ptr::null_mut();
        let mut value: *mut ffi::PyObject = std::ptr::null_mut();
        if unsafe {
            ffi::PyDict_Next(
                self.dict.as_ptr(),
                &raw mut self.pos,
                &raw mut key,
                &raw mut value,
            )
        } == 0
        {
            return None;
        }
        let py = self.dict.py();
        let name = if unsafe { ffi::Py_TYPE(key) } == types(py).str_ {
            Cow::Borrowed(unsafe { str_ref_ptr(key) }.unwrap_or(""))
        } else {
            Cow::Owned(key_to_string(unsafe { Borrowed::from_ptr(py, key) }))
        };
        let value = unsafe { Borrowed::from_ptr(py, value) };
        Some((name, value))
    }
}

impl<'py> Object<'py, Pyo3> for Borrowed<'py, 'py, PyDict> {
    type Node = PyNode<'py>;
    type MemberName = Cow<'py, str>;
    type MembersIter = PyMembers<'py>;

    fn len(&self) -> usize {
        unsafe { ffi::PyDict_Size(self.as_ptr()) }.max(0) as usize
    }

    fn get(&self, key: &Py<PyString>) -> Option<PyNode<'py>> {
        let py = self.py();
        // `PyDict_GetItem` returns a borrowed reference and never raises.
        let value = unsafe { ffi::PyDict_GetItem(self.as_ptr(), key.as_ptr()) };
        if value.is_null() {
            None
        } else {
            Some(unsafe { Borrowed::from_ptr(py, value) })
        }
    }

    fn members(&self) -> PyMembers<'py> {
        PyMembers {
            dict: *self,
            pos: 0,
        }
    }
}

// Array over a Python `list` or `tuple`.
pub struct PyArray<'py> {
    sequence: PyNode<'py>,
    is_tuple: bool,
    len: usize,
}

impl<'py> PyArray<'py> {
    fn new(sequence: PyNode<'py>, is_tuple: bool) -> Self {
        let ptr = sequence.as_ptr();
        let len = if is_tuple {
            unsafe { ffi::PyTuple_Size(ptr) }
        } else {
            unsafe { ffi::PyList_Size(ptr) }
        };
        PyArray {
            sequence,
            is_tuple,
            len: len.max(0) as usize,
        }
    }
}

impl<'py> Array<'py, Pyo3> for PyArray<'py> {
    type Node = PyNode<'py>;
    type ElementsIter = PyElements<'py>;

    fn len(&self) -> usize {
        self.len
    }

    fn elements(&self) -> PyElements<'py> {
        PyElements {
            sequence: self.sequence,
            is_tuple: self.is_tuple,
            index: 0,
            len: self.len,
        }
    }

    fn is_unique(&self) -> bool {
        let size = self.len;
        if size <= 1 {
            return true;
        }
        let items: Vec<PyNode<'py>> = self.elements().collect();
        if size <= crate::unique::ITEMS_SIZE_THRESHOLD {
            // Pairwise beats hashing for small arrays, even at O(N^2).
            let mut idx = 0;
            while idx < size {
                let mut inner_idx = idx + 1;
                while inner_idx < size {
                    if equal_nodes(items[idx], items[inner_idx]) {
                        return false;
                    }
                    inner_idx += 1;
                }
                idx += 1;
            }
            true
        } else {
            let mut seen = AHashSet::with_capacity(size);
            items.into_iter().all(|item| seen.insert(HashedNode(item)))
        }
    }
}

/// JSON equality between two Python nodes: numbers compare mathematically, objects ignore key order.
fn equal_nodes(left: PyNode<'_>, right: PyNode<'_>) -> bool {
    let (left_type, right_type) = (object_type(left), object_type(right));
    match (left_type, right_type) {
        (ObjType::Enum, _) => equal_nodes(resolved(left), right),
        (_, ObjType::Enum) => equal_nodes(left, resolved(right)),
        (ObjType::Unknown, _) => {
            record_unsupported(left);
            false
        }
        (_, ObjType::Unknown) => {
            record_unsupported(right);
            false
        }
        (ObjType::Null, ObjType::Null) => true,
        (ObjType::Bool, ObjType::Bool) => left.as_ptr() == right.as_ptr(),
        (
            ObjType::Int | ObjType::Float | ObjType::Decimal,
            ObjType::Int | ObjType::Float | ObjType::Decimal,
        ) => match (number_of(left), number_of(right)) {
            (Some(left), Some(right)) => cmp::equal_numbers(&left, &right),
            _ => false,
        },
        (ObjType::Str, ObjType::Str) => match (str_ref(left), str_ref(right)) {
            (Some(left), Some(right)) => left == right,
            _ => false,
        },
        (ObjType::List | ObjType::Tuple, ObjType::List | ObjType::Tuple) => {
            let left = PyArray::new(left, left_type == ObjType::Tuple);
            let right = PyArray::new(right, right_type == ObjType::Tuple);
            left.len() == right.len()
                && left
                    .elements()
                    .zip(right.elements())
                    .all(|(left, right)| equal_nodes(left, right))
        }
        (ObjType::Dict, ObjType::Dict) => {
            let (left_ptr, right_ptr) = (left.as_ptr(), right.as_ptr());
            if unsafe { ffi::PyDict_Size(left_ptr) != ffi::PyDict_Size(right_ptr) } {
                return false;
            }
            let mut position: ffi::Py_ssize_t = 0;
            let mut key: *mut ffi::PyObject = std::ptr::null_mut();
            let mut value: *mut ffi::PyObject = std::ptr::null_mut();
            while unsafe {
                ffi::PyDict_Next(left_ptr, &raw mut position, &raw mut key, &raw mut value) != 0
            } {
                let key_node = unsafe { Borrowed::from_ptr(left.py(), key) };
                // Legal keys hash as their JSON name, so lookup agrees with `key_to_string`.
                if !key_is_usable(key_node) {
                    record_unusable_key(key_node);
                    return false;
                }
                // Borrowed, never raises; a missing key means the objects differ.
                let other = unsafe { ffi::PyDict_GetItem(right_ptr, key) };
                if other.is_null() {
                    return false;
                }
                let (value, other) = unsafe {
                    (
                        Borrowed::from_ptr(left.py(), value),
                        Borrowed::from_ptr(left.py(), other),
                    )
                };
                if !equal_nodes(value, other) {
                    return false;
                }
            }
            true
        }
        _ => false,
    }
}

/// Mirrors `unique::HashedValue` so equal nodes always hash equal.
fn hash_node<H: Hasher>(node: PyNode<'_>, state: &mut H) {
    match object_type(node) {
        ObjType::Enum => hash_node(resolved(node), state),
        ObjType::Unknown => record_unsupported(node),
        ObjType::Null => state.write_u32(3_221_225_473),
        ObjType::Bool => (node.as_ptr() == types(node.py()).true_).hash(state),
        ObjType::Int | ObjType::Float | ObjType::Decimal => {
            if let Some(number) = number_of(node) {
                if let Some(number) = number.as_f64() {
                    number.to_bits().hash(state);
                } else if let Some(number) = number.as_u64() {
                    number.hash(state);
                } else if let Some(number) = number.as_i64() {
                    number.hash(state);
                }
            }
        }
        ObjType::Str => str_ref(node).unwrap_or_default().hash(state),
        ty @ (ObjType::List | ObjType::Tuple) => {
            let array = PyArray::new(node, ty == ObjType::Tuple);
            for item in array.elements() {
                hash_node(item, state);
            }
        }
        ObjType::Dict => {
            let dict = unsafe { node.cast_unchecked::<PyDict>() };
            let mut hash = 0;
            for (key, value) in dict.members() {
                let mut item_hasher = AHasher::default();
                key.as_ref().hash(&mut item_hasher);
                hash_node(value, &mut item_hasher);
                hash ^= item_hasher.finish();
            }
            state.write_u64(hash);
        }
    }
}

struct HashedNode<'py>(PyNode<'py>);

impl PartialEq for HashedNode<'_> {
    fn eq(&self, other: &Self) -> bool {
        equal_nodes(self.0, other.0)
    }
}

impl Eq for HashedNode<'_> {}

impl Hash for HashedNode<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_node(self.0, state);
    }
}

pub struct PyElements<'py> {
    sequence: PyNode<'py>,
    is_tuple: bool,
    index: usize,
    len: usize,
}

impl<'py> Iterator for PyElements<'py> {
    type Item = PyNode<'py>;

    fn next(&mut self) -> Option<PyNode<'py>> {
        if self.index >= self.len {
            return None;
        }
        let index = self.index as ffi::Py_ssize_t;
        self.index += 1;
        let ptr = self.sequence.as_ptr();
        // `*_GetItem` return borrowed references; keep them borrowed for `'py`.
        let borrowed = unsafe {
            if self.is_tuple {
                ffi::PyTuple_GetItem(ptr, index)
            } else {
                ffi::PyList_GetItem(ptr, index)
            }
        };
        if borrowed.is_null() {
            // Resized by a callback since `len` was read; clear the IndexError CPython just set.
            unsafe { ffi::PyErr_Clear() };
            record_value_error("Sequence changed size during validation");
            self.index = self.len;
            return None;
        }
        Some(unsafe { Borrowed::from_ptr(self.sequence.py(), borrowed) })
    }
}

// Needs a real interpreter, which only `pyo3-tests` links in.
#[cfg(all(test, feature = "pyo3-tests"))]
mod tests {
    use super::*;
    use serde_json::json;
    use test_case::test_case;

    fn to_py<'py>(py: Python<'py>, value: &Value) -> Bound<'py, PyAny> {
        let json = py.import("json").expect("json module");
        let text = serde_json::to_string(value).expect("serialize");
        json.call_method1("loads", (text,)).expect("loads")
    }

    #[cfg(feature = "conformance")]
    #[test]
    fn conforms_to_contract() {
        Python::attach(|py| {
            let document = crate::conformance::document();
            let owned = to_py(py, &document);
            let node = unsafe { Borrowed::from_ptr(py, owned.as_ptr()) };

            crate::conformance::assert_conformance::<Pyo3>(&node);
            assert_eq!(node.to_value().into_owned(), document);
        });
    }

    #[test]
    fn tuple_is_array() {
        Python::attach(|py| {
            let owned = pyo3::types::PyTuple::new(py, [1, 2, 3])
                .expect("tuple")
                .into_any();
            let tuple = unsafe { Borrowed::from_ptr(py, owned.as_ptr()) };
            assert_eq!(tuple.json_type(), JsonType::Array);
            let values: Vec<Option<u64>> = tuple
                .as_array()
                .expect("array")
                .elements()
                .map(|item| item.as_number().and_then(|n| n.as_u64()))
                .collect();
            assert_eq!(values, [Some(1), Some(2), Some(3)]);
            assert_eq!(tuple.to_value().into_owned(), json!([1, 2, 3]));
        });
    }

    #[test]
    fn enum_resolves_to_value() {
        Python::attach(|py| {
            let module = pyo3::types::PyModule::from_code(
                py,
                std::ffi::CString::new(
                    "import enum\nclass Color(enum.IntEnum):\n    RED = 1\nclass Name(enum.StrEnum):\n    A = 'abc'\n",
                )
                .unwrap()
                .as_c_str(),
                std::ffi::CString::new("m.py").unwrap().as_c_str(),
                std::ffi::CString::new("m").unwrap().as_c_str(),
            )
            .expect("module");
            let color = module.getattr("Color").unwrap().getattr("RED").unwrap();
            let color = unsafe { Borrowed::from_ptr(py, color.as_ptr()) };
            assert_eq!(color.json_type(), JsonType::Number);
            assert_eq!(color.as_number().expect("number").as_u64(), Some(1));

            let name = module.getattr("Name").unwrap().getattr("A").unwrap();
            let name = unsafe { Borrowed::from_ptr(py, name.as_ptr()) };
            assert_eq!(name.json_type(), JsonType::String);
            assert_eq!(name.as_string().as_deref(), Some("abc"));
        });
    }

    // `is_number` must agree with `as_number().is_some()`: non-finite floats are not JSON numbers.
    #[test_case("1", true; "int")]
    #[test_case("1.5", true; "float")]
    #[test_case("-0.0", true; "negative zero")]
    #[test_case("10**40", true; "int beyond i64")]
    #[test_case("float('nan')", false; "nan")]
    #[test_case("float('inf')", false; "inf")]
    #[test_case("float('-inf')", false; "negative inf")]
    #[test_case("True", false; "bool")]
    #[test_case("'x'", false; "string")]
    #[test_case("None", false; "none")]
    #[test_case("[1]", false; "list")]
    fn is_number_matches_as_number(expr: &str, want: bool) {
        Python::attach(|py| {
            let owned = py
                .eval(std::ffi::CString::new(expr).unwrap().as_c_str(), None, None)
                .expect("eval");
            let node = unsafe { Borrowed::from_ptr(py, owned.as_ptr()) };
            assert_eq!(node.is_number(), want);
            assert_eq!(node.is_number(), node.as_number().is_some());
        });
    }

    #[test_case(&json!(42), &json!(42.0), true; "int eq float")]
    #[test_case(&json!(42.0), &json!(42), true; "float eq int")]
    #[test_case(&json!(1), &json!(2), false; "diff numbers")]
    #[test_case(&json!("abc"), &json!("abc"), true; "string eq")]
    #[test_case(&json!("abc"), &json!("xyz"), false; "string neq")]
    #[test_case(&json!(true), &json!(true), true; "bool eq")]
    #[test_case(&json!(true), &json!(1), false; "bool is not number")]
    #[test_case(&json!(1), &json!(true), false; "number is not bool")]
    #[test_case(&json!(null), &json!(null), true; "null eq")]
    #[test_case(&json!(null), &json!(0), false; "null neq number")]
    #[test_case(&json!([1, 2]), &json!([1, 2]), true; "array eq")]
    #[test_case(&json!([1, 2]), &json!([1, 2, 3]), false; "array len diff")]
    #[test_case(&json!([1]), &json!([1.0]), true; "array cross-type number")]
    #[test_case(&json!([[1], [2]]), &json!([[1], [2]]), true; "nested array")]
    #[test_case(&json!({"a": 1}), &json!({"a": 1}), true; "object eq")]
    #[test_case(&json!({"a": 1}), &json!({"a": 1, "b": 2}), false; "expected superset")]
    #[test_case(&json!({"a": 1, "b": 2}), &json!({"a": 1}), false; "instance superset")]
    #[test_case(&json!({"a": 1}), &json!({"a": 2}), false; "object value diff")]
    #[test_case(&json!({"a": 1}), &json!({"b": 1}), false; "object key diff")]
    #[test_case(&json!({"a": {"b": 1}}), &json!({"a": {"b": 1}}), true; "nested object")]
    #[test_case(&json!({"a": 1}), &json!({"a": 1.0}), true; "object cross-type number")]
    #[test_case(&json!("1"), &json!(1), false; "string is not number")]
    #[test_case(&json!([]), &json!({}), false; "array is not object")]
    fn equals_value_semantics(instance: &Value, expected: &Value, want: bool) {
        Python::attach(|py| {
            let owned = to_py(py, instance);
            let node = unsafe { Borrowed::from_ptr(py, owned.as_ptr()) };
            assert_eq!(node.equals_value(expected), want);
        });
    }

    #[test]
    fn equals_value_enum_member() {
        Python::attach(|py| {
            let module = pyo3::types::PyModule::from_code(
                py,
                std::ffi::CString::new(
                    "import enum\nclass Color(enum.IntEnum):\n    RED = 1\nclass Name(enum.StrEnum):\n    A = 'abc'\n",
                )
                .unwrap()
                .as_c_str(),
                std::ffi::CString::new("m.py").unwrap().as_c_str(),
                std::ffi::CString::new("m").unwrap().as_c_str(),
            )
            .expect("module");
            let red = module.getattr("Color").unwrap().getattr("RED").unwrap();
            let red = unsafe { Borrowed::from_ptr(py, red.as_ptr()) };
            assert!(red.equals_value(&json!(1)));
            assert!(red.equals_value(&json!(1.0)));
            assert!(!red.equals_value(&json!(2)));
            let a = module.getattr("Name").unwrap().getattr("A").unwrap();
            let a = unsafe { Borrowed::from_ptr(py, a.as_ptr()) };
            assert!(a.equals_value(&json!("abc")));
            assert!(!a.equals_value(&json!("xyz")));
        });
    }

    #[test_case("[1, 2, 3]", true; "unique ints")]
    #[test_case("[1, 1]", false; "duplicate ints")]
    #[test_case("[1, 1.0]", false; "int equals float")]
    #[test_case("[1, True]", true; "bool is not number")]
    #[test_case("['a', 'b']", true; "unique strings")]
    #[test_case("['a', 'a']", false; "duplicate strings")]
    #[test_case("[None, None]", false; "duplicate nulls")]
    #[test_case("[None, False]", true; "null is not bool")]
    #[test_case("[[1, 2], [1, 2]]", false; "duplicate arrays")]
    #[test_case("[[1, 2], [2, 1]]", true; "arrays differ by order")]
    #[test_case("[{'a': 1, 'b': 2}, {'b': 2, 'a': 1}]", false; "objects differ by key order only")]
    #[test_case("[{'a': 1}, {'a': 2}]", true; "objects differ by value")]
    #[test_case("[{'a': 1}, {'b': 1}]", true; "objects differ by key")]
    #[test_case("[{'a': 1}, {'a': 1, 'b': 2}]", true; "object is a subset")]
    #[test_case("list(range(20))", true; "large unique")]
    #[test_case("list(range(20)) + [0]", false; "large duplicate")]
    #[test_case("[{'a': [1, {'b': 2}]}, {'a': [1, {'b': 2}]}]", false; "nested duplicate")]
    #[test_case("(1, 2, 1)", false; "tuple duplicate")]
    fn array_is_unique(expr: &str, want: bool) {
        Python::attach(|py| {
            let owned = py
                .eval(std::ffi::CString::new(expr).unwrap().as_c_str(), None, None)
                .expect("eval");
            let node = unsafe { Borrowed::from_ptr(py, owned.as_ptr()) };
            let array = node.as_array().expect("array");
            assert_eq!(array.is_unique(), want);
        });
    }

    #[test]
    fn surrogate_string_records_error() {
        Python::attach(|py| {
            let eval = |src: &str| {
                py.eval(std::ffi::CString::new(src).unwrap().as_c_str(), None, None)
                    .expect("eval")
            };
            let _ = take_pending_error();
            let owned = eval("'\\ud800'");
            let node = unsafe { Borrowed::from_ptr(py, owned.as_ptr()) };
            assert!(node.as_string().is_none());
            let error = take_pending_error().expect("recorded");
            assert_eq!(
                error.to_string(),
                "UnicodeEncodeError: 'utf-8' codec can't encode character '\\ud800' in position 0: surrogates not allowed"
            );
            let owned = eval("'abc'");
            let node = unsafe { Borrowed::from_ptr(py, owned.as_ptr()) };
            assert_eq!(node.as_string().as_deref(), Some("abc"));
            assert!(take_pending_error().is_none());
        });
    }

    #[test]
    fn decimal_is_number() {
        Python::attach(|py| {
            let owned = py
                .import("decimal")
                .unwrap()
                .getattr("Decimal")
                .unwrap()
                .call1(("1.5",))
                .unwrap();
            let decimal = unsafe { Borrowed::from_ptr(py, owned.as_ptr()) };
            assert_eq!(decimal.json_type(), JsonType::Number);
            assert_eq!(decimal.as_number().expect("number").as_f64(), Some(1.5));
        });
    }
}
