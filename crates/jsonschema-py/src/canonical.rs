use pyo3::{exceptions, ffi, prelude::*};
use serde::{
    ser::{self, Serialize, SerializeMap, SerializeSeq, SerializeStruct},
    Serializer,
};
use serde_json::ser::{CompactFormatter, Formatter};
use std::{cell::RefCell, io, mem};

use crate::{
    ser::{
        dict_len, get_object_type, get_object_type_from_object, get_type_name, is_enum_subclass,
        pylist_get_item, pylist_len, pytuple_get_item, pytuple_len, serialize_large_int,
        ObjectType, RECURSION_LIMIT,
    },
    types,
};
use pyo3::ffi::{
    PyLong_AsLongLong, PyObject_GetAttr, PyObject_IsInstance, PyUnicode_AsUTF8AndSize, Py_TYPE,
};

/// A serde_json Formatter that writes integer-valued floats as integers.
///
/// NaN and Infinity are NOT handled here — the Float serialization path
/// checks for those before calling `serialize_f64`, so `write_f64` only
/// receives finite values.
struct CanonicalFormatter {
    default: CompactFormatter,
}

const I64_UPPER_EXCLUSIVE_F64: f64 = 9_223_372_036_854_775_808.0;
const I64_LOWER_INCLUSIVE_F64: f64 = -9_223_372_036_854_775_808.0;
const U64_UPPER_EXCLUSIVE_F64: f64 = 18_446_744_073_709_551_616.0;
const MAX_SCRATCH_POOL_SIZE: usize = 8;
const MAX_SCRATCH_CAPACITY: usize = 16_384;
const SERDE_JSON_NUMBER_TOKEN: &str = "$serde_json::private::Number";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DecimalKind {
    Special,
    Integral,
    Fractional,
}

#[inline]
fn classify_decimal_kind(bytes: &[u8]) -> DecimalKind {
    if bytes.is_empty() {
        return DecimalKind::Fractional;
    }

    let mut idx = 0;
    if matches!(bytes[idx], b'+' | b'-') {
        idx += 1;
        if idx == bytes.len() {
            return DecimalKind::Fractional;
        }
    }

    // Decimal string specials are ascii words: NaN / sNaN / Infinity.
    if bytes[idx].is_ascii_alphabetic() {
        return DecimalKind::Special;
    }

    let mut seen_digit = false;
    let mut seen_dot = false;
    let mut frac_digits: i64 = 0;
    let mut suffix_zeros: i64 = 0;
    let mut all_digits_zero = true;

    while idx < bytes.len() {
        let byte = bytes[idx];
        if byte.is_ascii_digit() {
            seen_digit = true;
            if byte == b'0' {
                suffix_zeros = suffix_zeros.saturating_add(1);
            } else {
                suffix_zeros = 0;
                all_digits_zero = false;
            }
            if seen_dot {
                frac_digits = frac_digits.saturating_add(1);
            }
            idx += 1;
            continue;
        }
        if byte == b'.' && !seen_dot {
            seen_dot = true;
            idx += 1;
            continue;
        }
        break;
    }

    if !seen_digit {
        return DecimalKind::Fractional;
    }

    let mut exponent: i64 = 0;
    if idx < bytes.len() && matches!(bytes[idx], b'e' | b'E') {
        idx += 1;
        let mut negative = false;
        if idx < bytes.len() && matches!(bytes[idx], b'+' | b'-') {
            negative = bytes[idx] == b'-';
            idx += 1;
        }
        let mut has_exp_digit = false;
        while idx < bytes.len() && bytes[idx].is_ascii_digit() {
            has_exp_digit = true;
            exponent = exponent
                .saturating_mul(10)
                .saturating_add(i64::from(bytes[idx] - b'0'));
            idx += 1;
        }
        if !has_exp_digit {
            return DecimalKind::Fractional;
        }
        if negative {
            exponent = -exponent;
        }
    }

    if idx != bytes.len() {
        return DecimalKind::Fractional;
    }

    if exponent >= frac_digits {
        return DecimalKind::Integral;
    }

    let required_zeros = frac_digits.saturating_sub(exponent);
    if all_digits_zero || suffix_zeros >= required_zeros {
        DecimalKind::Integral
    } else {
        DecimalKind::Fractional
    }
}

struct BorrowedNumber<'a>(&'a str);

impl Serialize for BorrowedNumber<'_> {
    #[inline]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Emit an arbitrary-precision number directly without parsing or allocating.
        // This follows serde_json's internal Number serialization token contract.
        let mut s = serializer.serialize_struct(SERDE_JSON_NUMBER_TOKEN, 1)?;
        s.serialize_field(SERDE_JSON_NUMBER_TOKEN, self.0)?;
        s.end()
    }
}

#[cold]
#[inline(never)]
fn serialize_decimal<S>(object: *mut ffi::PyObject, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    // Get string representation of the Decimal
    let str_obj = unsafe { ffi::PyObject_Str(object) };
    if str_obj.is_null() {
        return Err(ser::Error::custom("Failed to convert Decimal to string"));
    }
    let mut str_size: ffi::Py_ssize_t = 0;
    let ptr = unsafe { ffi::PyUnicode_AsUTF8AndSize(str_obj, &raw mut str_size) };
    if ptr.is_null() {
        unsafe { ffi::Py_DECREF(str_obj) };
        return Err(ser::Error::custom("Failed to get UTF-8 representation"));
    }
    let slice = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(
            ptr.cast::<u8>(),
            str_size as usize,
        ))
    };

    // Classify from string representation in one pass:
    // special values -> null, integral values -> integer path.
    let bytes = slice.as_bytes();
    match classify_decimal_kind(bytes) {
        DecimalKind::Special => {
            unsafe { ffi::Py_DECREF(str_obj) };
            serializer.serialize_unit()
        }
        DecimalKind::Integral => {
            let py_int = unsafe { ffi::PyNumber_Long(object) };
            if py_int.is_null() {
                unsafe {
                    ffi::PyErr_Clear();
                    ffi::Py_DECREF(str_obj);
                }
                return Err(ser::Error::custom("Failed to convert Decimal to integer"));
            }
            let result = serialize_large_int(py_int, serializer);
            unsafe {
                ffi::Py_DECREF(py_int);
                ffi::Py_DECREF(str_obj);
            }
            result
        }
        DecimalKind::Fractional => {
            let result = serializer.serialize_some(&BorrowedNumber(slice));
            unsafe { ffi::Py_DECREF(str_obj) };
            result
        }
    }
}

impl Formatter for CanonicalFormatter {
    #[inline]
    fn write_f64<W: io::Write + ?Sized>(&mut self, writer: &mut W, value: f64) -> io::Result<()> {
        if value.fract() == 0.0 {
            if (0.0..U64_UPPER_EXCLUSIVE_F64).contains(&value) {
                // SAFETY: range check above guarantees lossless conversion.
                let int = unsafe { value.to_int_unchecked::<u64>() };
                return self.default.write_u64(writer, int);
            }
            if (I64_LOWER_INCLUSIVE_F64..I64_UPPER_EXCLUSIVE_F64).contains(&value) {
                // SAFETY: range check above guarantees lossless conversion.
                let int = unsafe { value.to_int_unchecked::<i64>() };
                return self.default.write_i64(writer, int);
            }
            // Integer-valued float: convert to integer via Python FFI.
            // The GIL is held because we are always called from within a #[pyfunction].
            unsafe {
                let py_float = ffi::PyFloat_FromDouble(value);
                if py_float.is_null() {
                    return Err(io::Error::other("PyFloat_FromDouble failed"));
                }
                let py_int = ffi::PyNumber_Long(py_float);
                ffi::Py_DECREF(py_float);
                if py_int.is_null() {
                    ffi::PyErr_Clear();
                    return Err(io::Error::other("PyNumber_Long failed"));
                }
                let str_obj = ffi::PyObject_Str(py_int);
                ffi::Py_DECREF(py_int);
                if str_obj.is_null() {
                    return Err(io::Error::other("PyObject_Str failed"));
                }
                let mut str_size: ffi::Py_ssize_t = 0;
                let ptr = ffi::PyUnicode_AsUTF8AndSize(str_obj, &raw mut str_size);
                if ptr.is_null() {
                    ffi::Py_DECREF(str_obj);
                    return Err(io::Error::other("PyUnicode_AsUTF8AndSize failed"));
                }
                let bytes = std::slice::from_raw_parts(ptr.cast::<u8>(), str_size as usize);
                let result = writer.write_all(bytes);
                ffi::Py_DECREF(str_obj);
                result
            }
        } else {
            self.default.write_f64(writer, value)
        }
    }
}

struct CanonicalPyObject<'scratch> {
    object: *mut ffi::PyObject,
    object_type: ObjectType,
    recursion_depth: u8,
    scratch_pool: &'scratch RefCell<Vec<Vec<DictEntry>>>,
}

struct DictEntry {
    key_ptr: *const u8,
    key_len: usize,
    value: *mut ffi::PyObject,
}

#[derive(Default)]
struct OwnedKeyRefs {
    refs: Option<Vec<*mut ffi::PyObject>>,
}

impl OwnedKeyRefs {
    #[inline]
    fn push(&mut self, object: *mut ffi::PyObject) {
        self.refs.get_or_insert_with(Vec::new).push(object);
    }
}

impl Drop for OwnedKeyRefs {
    fn drop(&mut self) {
        if let Some(refs) = self.refs.take() {
            for object in refs {
                unsafe { ffi::Py_DECREF(object) };
            }
        }
    }
}

struct DictEntryScratch<'scratch> {
    entries: Vec<DictEntry>,
    pool: &'scratch RefCell<Vec<Vec<DictEntry>>>,
}

impl<'scratch> DictEntryScratch<'scratch> {
    fn with_capacity(pool: &'scratch RefCell<Vec<Vec<DictEntry>>>, capacity: usize) -> Self {
        let mut entries = pool.borrow_mut().pop().unwrap_or_default();
        entries.clear();
        if entries.capacity() < capacity {
            entries.reserve(capacity - entries.capacity());
        }
        DictEntryScratch { entries, pool }
    }

    fn entries_mut(&mut self) -> &mut Vec<DictEntry> {
        &mut self.entries
    }
}

impl Drop for DictEntryScratch<'_> {
    fn drop(&mut self) {
        self.entries.clear();
        let entries = mem::take(&mut self.entries);
        if entries.capacity() > MAX_SCRATCH_CAPACITY {
            return;
        }
        let mut pool = self.pool.borrow_mut();
        if pool.len() < MAX_SCRATCH_POOL_SIZE {
            pool.push(entries);
            return;
        }
        if let Some((idx, min_capacity)) = pool
            .iter()
            .enumerate()
            .map(|(idx, vec)| (idx, vec.capacity()))
            .min_by_key(|(_, cap)| *cap)
        {
            if entries.capacity() > min_capacity {
                pool[idx] = entries;
            }
        }
    }
}

impl<'scratch> CanonicalPyObject<'scratch> {
    #[inline]
    fn new(
        object: *mut ffi::PyObject,
        recursion_depth: u8,
        scratch_pool: &'scratch RefCell<Vec<Vec<DictEntry>>>,
    ) -> Self {
        CanonicalPyObject {
            object,
            object_type: get_object_type_from_object(object),
            recursion_depth,
            scratch_pool,
        }
    }

    #[inline]
    fn with_obtype(
        object: *mut ffi::PyObject,
        object_type: ObjectType,
        recursion_depth: u8,
        scratch_pool: &'scratch RefCell<Vec<Vec<DictEntry>>>,
    ) -> Self {
        CanonicalPyObject {
            object,
            object_type,
            recursion_depth,
            scratch_pool,
        }
    }
}

macro_rules! tri {
    ($expr:expr) => {
        match $expr {
            Ok(val) => val,
            Err(err) => return Err(err),
        }
    };
}

impl Serialize for CanonicalPyObject<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self.object_type {
            ObjectType::Str => {
                let mut str_size: ffi::Py_ssize_t = 0;
                let ptr = unsafe { PyUnicode_AsUTF8AndSize(self.object, &raw mut str_size) };
                if ptr.is_null() {
                    let py = unsafe { Python::assume_attached() };
                    let py_error = pyo3::PyErr::fetch(py);
                    return Err(ser::Error::custom(format!(
                        "Failed to get UTF-8 representation: {py_error}",
                    )));
                }
                let slice = unsafe {
                    std::str::from_utf8_unchecked(std::slice::from_raw_parts(
                        ptr.cast::<u8>(),
                        str_size as usize,
                    ))
                };
                serializer.serialize_str(slice)
            }
            ObjectType::Int => {
                let value = unsafe { PyLong_AsLongLong(self.object) };
                if value == -1 {
                    #[cfg(Py_3_12)]
                    {
                        let exception = unsafe { ffi::PyErr_GetRaisedException() };
                        if !exception.is_null() {
                            unsafe { ffi::PyErr_Clear() };
                            return serialize_large_int(self.object, serializer);
                        }
                    };
                    #[cfg(not(Py_3_12))]
                    {
                        let mut ptype: *mut ffi::PyObject = std::ptr::null_mut();
                        let mut pvalue: *mut ffi::PyObject = std::ptr::null_mut();
                        let mut ptraceback: *mut ffi::PyObject = std::ptr::null_mut();
                        unsafe {
                            ffi::PyErr_Fetch(&raw mut ptype, &raw mut pvalue, &raw mut ptraceback);
                        }
                        let is_overflow = !pvalue.is_null();
                        if is_overflow {
                            unsafe {
                                if !ptype.is_null() {
                                    ffi::Py_DecRef(ptype);
                                }
                                if !pvalue.is_null() {
                                    ffi::Py_DecRef(pvalue);
                                }
                                if !ptraceback.is_null() {
                                    ffi::Py_DecRef(ptraceback);
                                }
                            };
                            return serialize_large_int(self.object, serializer);
                        }
                    };
                }
                serializer.serialize_i64(value)
            }
            ObjectType::Float => {
                let value = unsafe { crate::ser::pyfloat_as_double(self.object) };
                if value.is_nan() || value.is_infinite() {
                    // JSON has no NaN/Infinity: canonicalize to null
                    serializer.serialize_unit()
                } else {
                    // CanonicalFormatter::write_f64 handles integer-valued conversion
                    serializer.serialize_f64(value)
                }
            }
            ObjectType::Bool => serializer.serialize_bool(self.object == unsafe { types::TRUE }),
            ObjectType::None => serializer.serialize_unit(),
            ObjectType::Dict => {
                if self.recursion_depth == RECURSION_LIMIT {
                    return Err(ser::Error::custom("Recursion limit reached"));
                }
                let length = unsafe { dict_len(self.object) };
                if length == 0 {
                    tri!(serializer.serialize_map(Some(0))).end()
                } else if length == 1 {
                    // Fast path: single key — no allocation or sorting needed
                    let mut pos = 0_isize;
                    let mut str_size: ffi::Py_ssize_t = 0;
                    let mut key: *mut ffi::PyObject = std::ptr::null_mut();
                    let mut value: *mut ffi::PyObject = std::ptr::null_mut();
                    unsafe {
                        ffi::PyDict_Next(self.object, &raw mut pos, &raw mut key, &raw mut value);
                    }
                    let object_type = unsafe { Py_TYPE(key) };
                    let (key_unicode, owned) = if object_type == unsafe { types::STR_TYPE } {
                        (key, false)
                    } else {
                        let is_str = unsafe {
                            PyObject_IsInstance(key, types::STR_TYPE.cast::<ffi::PyObject>())
                        };
                        if is_str < 0 {
                            return Err(ser::Error::custom("Error while checking key type"));
                        }
                        if is_str > 0 && is_enum_subclass(object_type) {
                            let attr = unsafe { PyObject_GetAttr(key, types::VALUE_STR) };
                            if attr.is_null() {
                                let py = unsafe { Python::assume_attached() };
                                let py_error = pyo3::PyErr::fetch(py);
                                return Err(ser::Error::custom(format!(
                                    "Failed to access enum key value: {py_error}",
                                )));
                            }
                            (attr, true)
                        } else {
                            return Err(ser::Error::custom(format!(
                                "Dict key must be str or str enum. Got '{}'",
                                get_type_name(object_type)
                            )));
                        }
                    };
                    let ptr = unsafe { PyUnicode_AsUTF8AndSize(key_unicode, &raw mut str_size) };
                    if ptr.is_null() {
                        let py = unsafe { Python::assume_attached() };
                        let py_error = pyo3::PyErr::fetch(py);
                        if owned {
                            unsafe { ffi::Py_DECREF(key_unicode) };
                        }
                        return Err(ser::Error::custom(format!(
                            "Failed to get key as UTF-8: {py_error}",
                        )));
                    }
                    let key_str = unsafe {
                        std::str::from_utf8_unchecked(std::slice::from_raw_parts(
                            ptr.cast::<u8>(),
                            str_size as usize,
                        ))
                    };
                    let mut map = tri!(serializer.serialize_map(Some(1)));
                    let result = map.serialize_entry(
                        key_str,
                        &CanonicalPyObject::new(value, self.recursion_depth + 1, self.scratch_pool),
                    );
                    if owned {
                        unsafe { ffi::Py_DECREF(key_unicode) };
                    }
                    tri!(result);
                    map.end()
                } else {
                    // Collect all key-value pairs, sort by key, then serialize
                    let mut scratch = DictEntryScratch::with_capacity(self.scratch_pool, length);
                    let entries = scratch.entries_mut();
                    let mut owned_key_refs = OwnedKeyRefs::default();
                    let mut pos = 0_isize;
                    let mut str_size: ffi::Py_ssize_t = 0;
                    let mut key: *mut ffi::PyObject = std::ptr::null_mut();
                    let mut value: *mut ffi::PyObject = std::ptr::null_mut();
                    for _ in 0..length {
                        unsafe {
                            ffi::PyDict_Next(
                                self.object,
                                &raw mut pos,
                                &raw mut key,
                                &raw mut value,
                            );
                        }
                        let object_type = unsafe { Py_TYPE(key) };
                        let key_unicode = if object_type == unsafe { types::STR_TYPE } {
                            key
                        } else {
                            let is_str = unsafe {
                                PyObject_IsInstance(key, types::STR_TYPE.cast::<ffi::PyObject>())
                            };
                            if is_str < 0 {
                                return Err(ser::Error::custom("Error while checking key type"));
                            }
                            if is_str > 0 && is_enum_subclass(object_type) {
                                let attr = unsafe { PyObject_GetAttr(key, types::VALUE_STR) };
                                if attr.is_null() {
                                    let py = unsafe { Python::assume_attached() };
                                    let py_error = pyo3::PyErr::fetch(py);
                                    return Err(ser::Error::custom(format!(
                                        "Failed to access enum key value: {py_error}"
                                    )));
                                }
                                owned_key_refs.push(attr);
                                attr
                            } else {
                                return Err(ser::Error::custom(format!(
                                    "Dict key must be str or str enum. Got '{}'",
                                    get_type_name(object_type)
                                )));
                            }
                        };

                        let ptr =
                            unsafe { PyUnicode_AsUTF8AndSize(key_unicode, &raw mut str_size) };
                        if ptr.is_null() {
                            let py = unsafe { Python::assume_attached() };
                            let py_error = pyo3::PyErr::fetch(py);
                            return Err(ser::Error::custom(format!(
                                "Failed to get key as UTF-8: {py_error}",
                            )));
                        }
                        entries.push(DictEntry {
                            key_ptr: ptr.cast::<u8>(),
                            key_len: str_size as usize,
                            value,
                        });
                    }
                    // Sort keys alphabetically for canonical form
                    entries.sort_unstable_by(|a, b| unsafe {
                        std::slice::from_raw_parts(a.key_ptr, a.key_len)
                            .cmp(std::slice::from_raw_parts(b.key_ptr, b.key_len))
                    });

                    let mut map = tri!(serializer.serialize_map(Some(length)));
                    for entry in entries.iter() {
                        let key_str = unsafe {
                            std::str::from_utf8_unchecked(std::slice::from_raw_parts(
                                entry.key_ptr,
                                entry.key_len,
                            ))
                        };
                        tri!(map.serialize_entry(
                            key_str,
                            &CanonicalPyObject::new(
                                entry.value,
                                self.recursion_depth + 1,
                                self.scratch_pool,
                            ),
                        ));
                    }
                    map.end()
                }
            }
            ObjectType::List => {
                if self.recursion_depth == RECURSION_LIMIT {
                    return Err(ser::Error::custom("Recursion limit reached"));
                }
                let length = unsafe { pylist_len(self.object) };
                if length == 0 {
                    tri!(serializer.serialize_seq(Some(0))).end()
                } else {
                    let mut type_ptr = std::ptr::null_mut();
                    let mut ob_type = ObjectType::Str;
                    let mut sequence = tri!(serializer.serialize_seq(Some(length)));
                    for i in 0..length {
                        let elem = unsafe { pylist_get_item(self.object, i as ffi::Py_ssize_t) };
                        let current_ob_type = unsafe { Py_TYPE(elem) };
                        if current_ob_type != type_ptr {
                            type_ptr = current_ob_type;
                            ob_type = get_object_type(current_ob_type);
                        }
                        tri!(sequence.serialize_element(&CanonicalPyObject::with_obtype(
                            elem,
                            ob_type,
                            self.recursion_depth + 1,
                            self.scratch_pool,
                        )));
                    }
                    sequence.end()
                }
            }
            ObjectType::Tuple => {
                if self.recursion_depth == RECURSION_LIMIT {
                    return Err(ser::Error::custom("Recursion limit reached"));
                }
                let length = unsafe { pytuple_len(self.object) };
                if length == 0 {
                    tri!(serializer.serialize_seq(Some(0))).end()
                } else {
                    let mut type_ptr = std::ptr::null_mut();
                    let mut ob_type = ObjectType::Str;
                    let mut sequence = tri!(serializer.serialize_seq(Some(length)));
                    for i in 0..length {
                        let elem = unsafe { pytuple_get_item(self.object, i as ffi::Py_ssize_t) };
                        let current_ob_type = unsafe { Py_TYPE(elem) };
                        if current_ob_type != type_ptr {
                            type_ptr = current_ob_type;
                            ob_type = get_object_type(current_ob_type);
                        }
                        tri!(sequence.serialize_element(&CanonicalPyObject::with_obtype(
                            elem,
                            ob_type,
                            self.recursion_depth + 1,
                            self.scratch_pool,
                        )));
                    }
                    sequence.end()
                }
            }
            ObjectType::Decimal => serialize_decimal(self.object, serializer),
            ObjectType::Enum => {
                let value = unsafe { PyObject_GetAttr(self.object, types::VALUE_STR) };
                if value.is_null() {
                    let py = unsafe { Python::assume_attached() };
                    let py_error = pyo3::PyErr::fetch(py);
                    return Err(ser::Error::custom(format!(
                        "Failed to access enum value: {py_error}",
                    )));
                }
                #[allow(clippy::arithmetic_side_effects)]
                let result =
                    CanonicalPyObject::new(value, self.recursion_depth + 1, self.scratch_pool)
                        .serialize(serializer);
                unsafe { ffi::Py_DECREF(value) };
                result
            }
            ObjectType::Unknown => {
                let object_type = unsafe { Py_TYPE(self.object) };
                Err(ser::Error::custom(format!(
                    "Unsupported type: '{}'",
                    get_type_name(object_type)
                )))
            }
        }
    }
}

#[inline]
fn initial_output_capacity(object: *mut ffi::PyObject, object_type: ObjectType) -> usize {
    const MIN_CAPACITY: usize = 16;
    const MAX_PREALLOC: usize = 1 << 20; // 1 MiB

    let estimated = match object_type {
        ObjectType::Dict => {
            let len = unsafe { dict_len(object) };
            len.saturating_mul(24).saturating_add(2)
        }
        ObjectType::List => {
            let len = unsafe { pylist_len(object) };
            len.saturating_mul(12).saturating_add(2)
        }
        ObjectType::Tuple => {
            let len = unsafe { pytuple_len(object) };
            len.saturating_mul(12).saturating_add(2)
        }
        ObjectType::Str => 64,
        ObjectType::Int | ObjectType::Float | ObjectType::Decimal => 32,
        ObjectType::Bool | ObjectType::None => 8,
        ObjectType::Enum | ObjectType::Unknown => MIN_CAPACITY,
    };

    estimated.clamp(MIN_CAPACITY, MAX_PREALLOC)
}

fn to_canonical_string(object: *mut ffi::PyObject) -> serde_json::Result<String> {
    let object_type = get_object_type_from_object(object);
    let mut output = Vec::with_capacity(initial_output_capacity(object, object_type));
    let formatter = CanonicalFormatter {
        default: CompactFormatter,
    };
    let scratch_pool = RefCell::new(Vec::new());
    let mut serializer = serde_json::Serializer::with_formatter(&mut output, formatter);
    CanonicalPyObject::with_obtype(object, object_type, 0, &scratch_pool)
        .serialize(&mut serializer)?;
    Ok(unsafe { String::from_utf8_unchecked(output) })
}

#[pyfunction]
pub(crate) fn canonical_dumps(object: &Bound<'_, PyAny>) -> PyResult<String> {
    to_canonical_string(object.as_ptr())
        .map_err(|e| exceptions::PyValueError::new_err(e.to_string()))
}
