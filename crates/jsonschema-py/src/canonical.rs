use pyo3::{exceptions, ffi, prelude::*};
use serde::{
    ser::{self, Serialize, SerializeMap, SerializeSeq},
    Serializer,
};
use serde_json::ser::{CompactFormatter, Formatter};
use std::{io, str::FromStr};

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

impl Formatter for CanonicalFormatter {
    #[inline]
    fn write_f64<W: io::Write + ?Sized>(&mut self, writer: &mut W, value: f64) -> io::Result<()> {
        if value.fract() == 0.0 {
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

struct CanonicalPyObject {
    object: *mut ffi::PyObject,
    object_type: ObjectType,
    recursion_depth: u8,
}

impl CanonicalPyObject {
    #[inline]
    fn new(object: *mut ffi::PyObject, recursion_depth: u8) -> Self {
        CanonicalPyObject {
            object,
            object_type: get_object_type_from_object(object),
            recursion_depth,
        }
    }

    #[inline]
    const fn with_obtype(
        object: *mut ffi::PyObject,
        object_type: ObjectType,
        recursion_depth: u8,
    ) -> Self {
        CanonicalPyObject {
            object,
            object_type,
            recursion_depth,
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

impl Serialize for CanonicalPyObject {
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
                        &CanonicalPyObject::new(value, self.recursion_depth + 1),
                    );
                    if owned {
                        unsafe { ffi::Py_DECREF(key_unicode) };
                    }
                    tri!(result);
                    map.end()
                } else {
                    // Collect all key-value pairs, sort by key, then serialize
                    let mut entries: Vec<(String, *mut ffi::PyObject)> = Vec::with_capacity(length);
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
                                        "Failed to access enum key value: {py_error}"
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

                        let key_string = unsafe {
                            let ptr = PyUnicode_AsUTF8AndSize(key_unicode, &raw mut str_size);
                            if ptr.is_null() {
                                let py = Python::assume_attached();
                                let py_error = pyo3::PyErr::fetch(py);
                                if owned {
                                    ffi::Py_DECREF(key_unicode);
                                }
                                return Err(ser::Error::custom(format!(
                                    "Failed to get key as UTF-8: {py_error}",
                                )));
                            }
                            std::str::from_utf8_unchecked(std::slice::from_raw_parts(
                                ptr.cast::<u8>(),
                                str_size as usize,
                            ))
                            .to_string()
                        };
                        if owned {
                            unsafe { ffi::Py_DECREF(key_unicode) };
                        }
                        entries.push((key_string, value));
                    }
                    // Sort keys alphabetically for canonical form
                    entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));

                    let mut map = tri!(serializer.serialize_map(Some(length)));
                    for (key_str, val_ptr) in &entries {
                        tri!(map.serialize_entry(
                            key_str.as_str(),
                            &CanonicalPyObject::new(*val_ptr, self.recursion_depth + 1),
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
                        )));
                    }
                    sequence.end()
                }
            }
            ObjectType::Decimal => {
                // Get string representation of the Decimal
                let str_obj = unsafe { ffi::PyObject_Str(self.object) };
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

                // Check for special values (NaN / Infinity)
                let upper = slice.to_uppercase();
                if upper.contains("NAN") || upper.contains("INF") {
                    unsafe { ffi::Py_DECREF(str_obj) };
                    return serializer.serialize_unit();
                }

                // Try converting to integer to detect integer-valued Decimals.
                // PyNumber_Long returns NULL for NaN/Inf (already handled above) and
                // truncates fractional values, so we compare back to the original.
                let py_int = unsafe { ffi::PyNumber_Long(self.object) };
                if py_int.is_null() {
                    // Shouldn't reach here after the NaN/Inf check, but handle safely
                    unsafe {
                        ffi::PyErr_Clear();
                        ffi::Py_DECREF(str_obj);
                    }
                    return Err(ser::Error::custom("Failed to convert Decimal to integer"));
                }

                // Compare: int(decimal) == decimal ?
                let cmp = unsafe { ffi::PyObject_RichCompareBool(py_int, self.object, ffi::Py_EQ) };
                if cmp > 0 {
                    // Integer-valued: serialize the integer
                    let result = serialize_large_int(py_int, serializer);
                    unsafe {
                        ffi::Py_DECREF(py_int);
                        ffi::Py_DECREF(str_obj);
                    }
                    result
                } else {
                    // Fractional: parse as serde_json::Number using the string form
                    unsafe { ffi::Py_DECREF(py_int) };
                    let result = if let Ok(num) = serde_json::Number::from_str(slice) {
                        serializer.serialize_some(&num)
                    } else {
                        Err(ser::Error::custom("Failed to parse Decimal as number"))
                    };
                    unsafe { ffi::Py_DECREF(str_obj) };
                    result
                }
            }
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
                    CanonicalPyObject::new(value, self.recursion_depth + 1).serialize(serializer);
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

fn to_canonical_string(object: *mut ffi::PyObject) -> serde_json::Result<String> {
    let mut output = Vec::with_capacity(16);
    let formatter = CanonicalFormatter {
        default: CompactFormatter,
    };
    let mut serializer = serde_json::Serializer::with_formatter(&mut output, formatter);
    CanonicalPyObject::new(object, 0).serialize(&mut serializer)?;
    Ok(unsafe { String::from_utf8_unchecked(output) })
}

#[pyfunction]
pub(crate) fn canonical_dumps(object: &Bound<'_, PyAny>) -> PyResult<String> {
    to_canonical_string(object.as_ptr())
        .map_err(|e| exceptions::PyValueError::new_err(e.to_string()))
}
