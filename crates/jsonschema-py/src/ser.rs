use pyo3::{
    exceptions,
    ffi::{
        PyLong_AsLongLong, PyObject_GetAttr, PyObject_GetAttrString, PyObject_IsInstance,
        PyType_IsSubtype, PyUnicode_AsUTF8AndSize, Py_DECREF, Py_TYPE,
    },
    prelude::*,
    types::PyAny,
};
use serde::{
    ser::{self, Serialize, SerializeMap, SerializeSeq},
    Serializer,
};

use crate::types;
use std::{borrow::Cow, str::FromStr};

#[cfg(not(Py_LIMITED_API))]
use pyo3::ffi::{
    PyFloat_AS_DOUBLE, PyList_GET_ITEM, PyList_GET_SIZE, PyTuple_GET_ITEM, PyTuple_GET_SIZE,
};

#[cfg(all(not(Py_LIMITED_API), not(PyPy)))]
use pyo3::ffi::PyDictObject;

pub const RECURSION_LIMIT: u8 = 255;

#[derive(Clone, Copy)]
pub enum ObjectType {
    Str,
    Int,
    Bool,
    None,
    Float,
    List,
    Dict,
    Tuple,
    Enum,
    Unknown,
}

pub(crate) struct SerializePyObject {
    object: *mut pyo3::ffi::PyObject,
    object_type: ObjectType,
    recursion_depth: u8,
}

impl SerializePyObject {
    #[inline]
    pub fn new(object: *mut pyo3::ffi::PyObject, recursion_depth: u8) -> Self {
        SerializePyObject {
            object,
            object_type: get_object_type_from_object(object),
            recursion_depth,
        }
    }

    #[inline]
    pub const fn with_obtype(
        object: *mut pyo3::ffi::PyObject,
        object_type: ObjectType,
        recursion_depth: u8,
    ) -> Self {
        SerializePyObject {
            object,
            object_type,
            recursion_depth,
        }
    }
}

#[inline]
unsafe fn pyfloat_as_double(object: *mut pyo3::ffi::PyObject) -> f64 {
    #[cfg(Py_LIMITED_API)]
    {
        pyo3::ffi::PyFloat_AsDouble(object)
    }
    #[cfg(not(Py_LIMITED_API))]
    {
        PyFloat_AS_DOUBLE(object)
    }
}

#[inline]
unsafe fn pylist_len(object: *mut pyo3::ffi::PyObject) -> usize {
    #[cfg(Py_LIMITED_API)]
    {
        pyo3::ffi::PyList_Size(object) as usize
    }
    #[cfg(not(Py_LIMITED_API))]
    {
        PyList_GET_SIZE(object) as usize
    }
}

#[inline]
unsafe fn pylist_get_item(
    object: *mut pyo3::ffi::PyObject,
    index: pyo3::ffi::Py_ssize_t,
) -> *mut pyo3::ffi::PyObject {
    #[cfg(Py_LIMITED_API)]
    {
        pyo3::ffi::PyList_GetItem(object, index)
    }
    #[cfg(not(Py_LIMITED_API))]
    {
        PyList_GET_ITEM(object, index)
    }
}

#[inline]
unsafe fn pytuple_len(object: *mut pyo3::ffi::PyObject) -> usize {
    #[cfg(Py_LIMITED_API)]
    {
        pyo3::ffi::PyTuple_Size(object) as usize
    }
    #[cfg(not(Py_LIMITED_API))]
    {
        PyTuple_GET_SIZE(object) as usize
    }
}

#[inline]
unsafe fn pytuple_get_item(
    object: *mut pyo3::ffi::PyObject,
    index: pyo3::ffi::Py_ssize_t,
) -> *mut pyo3::ffi::PyObject {
    #[cfg(Py_LIMITED_API)]
    {
        pyo3::ffi::PyTuple_GetItem(object, index)
    }
    #[cfg(not(Py_LIMITED_API))]
    {
        PyTuple_GET_ITEM(object, index)
    }
}

#[inline]
unsafe fn dict_len(object: *mut pyo3::ffi::PyObject) -> usize {
    #[cfg(any(Py_LIMITED_API, PyPy))]
    {
        pyo3::ffi::PyDict_Size(object) as usize
    }
    #[cfg(all(not(Py_LIMITED_API), not(PyPy)))]
    {
        (*object.cast::<PyDictObject>()).ma_used as usize
    }
}

#[inline]
fn is_enum_subclass(object_type: *mut pyo3::ffi::PyTypeObject) -> bool {
    unsafe { PyType_IsSubtype(object_type, types::ENUM_BASE) != 0 }
}

#[inline]
fn is_dict_subclass(object_type: *mut pyo3::ffi::PyTypeObject) -> bool {
    unsafe { PyType_IsSubtype(object_type, types::DICT_TYPE) != 0 }
}

fn get_object_type_from_object(object: *mut pyo3::ffi::PyObject) -> ObjectType {
    unsafe {
        let object_type = Py_TYPE(object);
        get_object_type(object_type)
    }
}

fn get_type_name(object_type: *mut pyo3::ffi::PyTypeObject) -> Cow<'static, str> {
    unsafe {
        let name_obj = PyObject_GetAttrString(
            object_type.cast::<pyo3::ffi::PyObject>(),
            c"__name__".as_ptr(),
        );
        if name_obj.is_null() {
            return Cow::Borrowed("<unknown>");
        }
        let mut size: pyo3::ffi::Py_ssize_t = 0;
        let ptr = PyUnicode_AsUTF8AndSize(name_obj, &raw mut size);
        let cow = if ptr.is_null() {
            Cow::Borrowed("<unknown>")
        } else {
            let slice = std::slice::from_raw_parts(ptr.cast::<u8>(), size as usize);
            Cow::Owned(std::str::from_utf8_unchecked(slice).to_string())
        };
        Py_DECREF(name_obj);
        cow
    }
}

#[inline]
pub fn get_object_type(object_type: *mut pyo3::ffi::PyTypeObject) -> ObjectType {
    // Dict & str are the most popular in real-life JSON structures
    if object_type == unsafe { types::DICT_TYPE } {
        ObjectType::Dict
    } else if object_type == unsafe { types::STR_TYPE } {
        ObjectType::Str
    } else if object_type == unsafe { types::LIST_TYPE } {
        ObjectType::List
    } else if object_type == unsafe { types::INT_TYPE } {
        ObjectType::Int
    } else if object_type == unsafe { types::BOOL_TYPE } {
        ObjectType::Bool
    } else if object_type == unsafe { types::FLOAT_TYPE } {
        ObjectType::Float
    } else if object_type == unsafe { types::NONE_TYPE } {
        ObjectType::None
    } else if is_dict_subclass(object_type) {
        ObjectType::Dict
    } else if object_type == unsafe { types::TUPLE_TYPE } {
        ObjectType::Tuple
    } else if is_enum_subclass(object_type) {
        ObjectType::Enum
    } else {
        ObjectType::Unknown
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

/// Helper function to serialize a large integer that doesn't fit in i64
/// by converting it to a string and parsing as serde_json::Number
fn serialize_large_int<S>(
    object: *mut pyo3::ffi::PyObject,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let str_obj = unsafe { pyo3::ffi::PyObject_Str(object) };
    if str_obj.is_null() {
        return Err(ser::Error::custom(
            "Failed to convert large integer to string",
        ));
    }
    let mut str_size: pyo3::ffi::Py_ssize_t = 0;
    let ptr = unsafe { pyo3::ffi::PyUnicode_AsUTF8AndSize(str_obj, &raw mut str_size) };
    if ptr.is_null() {
        unsafe { pyo3::ffi::Py_DecRef(str_obj) };
        return Err(ser::Error::custom("Failed to get UTF-8 representation"));
    }
    let slice = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(
            ptr.cast::<u8>(),
            str_size as usize,
        ))
    };
    // With arbitrary_precision, serde_json can handle this as a number
    let result = if let Ok(num) = serde_json::Number::from_str(slice) {
        serializer.serialize_some(&num)
    } else {
        Err(ser::Error::custom("Failed to parse large integer"))
    };
    unsafe { pyo3::ffi::Py_DecRef(str_obj) };
    result
}

/// Convert a Python value to `serde_json::Value`
impl Serialize for SerializePyObject {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self.object_type {
            ObjectType::Str => {
                let mut str_size: pyo3::ffi::Py_ssize_t = 0;
                let ptr = unsafe { PyUnicode_AsUTF8AndSize(self.object, &raw mut str_size) };
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
                        let exception = unsafe { pyo3::ffi::PyErr_GetRaisedException() };
                        // Check if this is actually an overflow error
                        if !exception.is_null() {
                            unsafe { pyo3::ffi::PyErr_Clear() };
                            return serialize_large_int(self.object, serializer);
                        }
                    };
                    #[cfg(not(Py_3_12))]
                    {
                        let mut ptype: *mut pyo3::ffi::PyObject = std::ptr::null_mut();
                        let mut pvalue: *mut pyo3::ffi::PyObject = std::ptr::null_mut();
                        let mut ptraceback: *mut pyo3::ffi::PyObject = std::ptr::null_mut();
                        unsafe {
                            pyo3::ffi::PyErr_Fetch(
                                &raw mut ptype,
                                &raw mut pvalue,
                                &raw mut ptraceback,
                            );
                        }
                        // Check if this is actually an overflow error
                        let is_overflow = !pvalue.is_null();
                        if is_overflow {
                            unsafe {
                                if !ptype.is_null() {
                                    pyo3::ffi::Py_DecRef(ptype);
                                }
                                if !pvalue.is_null() {
                                    pyo3::ffi::Py_DecRef(pvalue);
                                }
                                if !ptraceback.is_null() {
                                    pyo3::ffi::Py_DecRef(ptraceback);
                                }
                            };
                            return serialize_large_int(self.object, serializer);
                        }
                    };
                }
                serializer.serialize_i64(value)
            }
            ObjectType::Float => {
                serializer.serialize_f64(unsafe { pyfloat_as_double(self.object) })
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
                } else {
                    let mut map = tri!(serializer.serialize_map(Some(length)));
                    let mut pos = 0_isize;
                    let mut str_size: pyo3::ffi::Py_ssize_t = 0;
                    let mut key: *mut pyo3::ffi::PyObject = std::ptr::null_mut();
                    let mut value: *mut pyo3::ffi::PyObject = std::ptr::null_mut();
                    for _ in 0..length {
                        unsafe {
                            pyo3::ffi::PyDict_Next(
                                self.object,
                                &raw mut pos,
                                &raw mut key,
                                &raw mut value,
                            );
                        }
                        let object_type = unsafe { Py_TYPE(key) };
                        let key_unicode = if object_type == unsafe { types::STR_TYPE } {
                            // if the key type is string, use it as is
                            key
                        } else {
                            let is_str = unsafe {
                                PyObject_IsInstance(
                                    key,
                                    types::STR_TYPE.cast::<pyo3::ffi::PyObject>(),
                                )
                            };
                            if is_str < 0 {
                                return Err(ser::Error::custom("Error while checking key type"));
                            }

                            // cover for both old-style str enums subclassing str and Enum and for new-style
                            // ones subclassing StrEnum
                            if is_str > 0 && is_enum_subclass(object_type) {
                                unsafe { PyObject_GetAttr(key, types::VALUE_STR) }
                            } else {
                                return Err(ser::Error::custom(format!(
                                    "Dict key must be str or str enum. Got '{}'",
                                    get_type_name(object_type)
                                )));
                            }
                        };

                        let ptr =
                            unsafe { PyUnicode_AsUTF8AndSize(key_unicode, &raw mut str_size) };
                        let slice = unsafe {
                            std::str::from_utf8_unchecked(std::slice::from_raw_parts(
                                ptr.cast::<u8>(),
                                str_size as usize,
                            ))
                        };
                        tri!(map.serialize_entry(
                            slice,
                            &SerializePyObject::new(value, self.recursion_depth + 1),
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
                        let elem =
                            unsafe { pylist_get_item(self.object, i as pyo3::ffi::Py_ssize_t) };
                        let current_ob_type = unsafe { Py_TYPE(elem) };
                        if current_ob_type != type_ptr {
                            type_ptr = current_ob_type;
                            ob_type = get_object_type(current_ob_type);
                        }
                        tri!(sequence.serialize_element(&SerializePyObject::with_obtype(
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
                        let elem =
                            unsafe { pytuple_get_item(self.object, i as pyo3::ffi::Py_ssize_t) };
                        let current_ob_type = unsafe { Py_TYPE(elem) };
                        if current_ob_type != type_ptr {
                            type_ptr = current_ob_type;
                            ob_type = get_object_type(current_ob_type);
                        }
                        tri!(sequence.serialize_element(&SerializePyObject::with_obtype(
                            elem,
                            ob_type,
                            self.recursion_depth + 1,
                        )));
                    }
                    sequence.end()
                }
            }
            ObjectType::Enum => {
                let value = unsafe { PyObject_GetAttr(self.object, types::VALUE_STR) };
                #[allow(clippy::arithmetic_side_effects)]
                SerializePyObject::new(value, self.recursion_depth + 1).serialize(serializer)
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
pub(crate) fn to_value(object: &Bound<'_, PyAny>) -> PyResult<serde_json::Value> {
    serde_json::to_value(SerializePyObject::new(object.as_ptr(), 0))
        .map_err(|err| exceptions::PyValueError::new_err(err.to_string()))
}
