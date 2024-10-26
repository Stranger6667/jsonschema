use pyo3::{
    exceptions,
    ffi::{
        PyDictObject, PyFloat_AS_DOUBLE, PyList_GET_ITEM, PyList_GET_SIZE, PyLong_AsLongLong,
        PyObject_GetAttr, PyTuple_GET_ITEM, PyTuple_GET_SIZE, PyUnicode_AsUTF8AndSize, Py_DECREF,
        Py_TPFLAGS_DICT_SUBCLASS, Py_TYPE,
    },
    prelude::*,
    types::PyAny,
};
use serde::{
    ser::{self, Serialize, SerializeMap, SerializeSeq},
    Serializer,
};

use crate::{ffi, types};
use std::ffi::CStr;

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
fn is_enum_subclass(object_type: *mut pyo3::ffi::PyTypeObject) -> bool {
    unsafe { (*(object_type.cast::<ffi::PyTypeObject>())).ob_type == types::ENUM_TYPE }
}

#[inline]
fn is_dict_subclass(object_type: *mut pyo3::ffi::PyTypeObject) -> bool {
    unsafe { (*object_type).tp_flags & Py_TPFLAGS_DICT_SUBCLASS != 0 }
}

fn get_object_type_from_object(object: *mut pyo3::ffi::PyObject) -> ObjectType {
    unsafe {
        let object_type = Py_TYPE(object);
        get_object_type(object_type)
    }
}

fn get_type_name(object_type: *mut pyo3::ffi::PyTypeObject) -> std::borrow::Cow<'static, str> {
    unsafe { CStr::from_ptr((*object_type).tp_name).to_string_lossy() }
}

#[inline]
fn check_type_is_str<E: ser::Error>(object: *mut pyo3::ffi::PyObject) -> Result<(), E> {
    let object_type = unsafe { Py_TYPE(object) };
    if object_type != unsafe { types::STR_TYPE } {
        return Err(ser::Error::custom(format!(
            "Dict key must be str. Got '{}'",
            get_type_name(object_type)
        )));
    }
    Ok(())
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

macro_rules! bail_on_integer_conversion_error {
    ($value:expr) => {
        if !$value.is_null() {
            let repr = unsafe { pyo3::ffi::PyObject_Str($value) };
            let mut size = 0;
            let ptr = unsafe { PyUnicode_AsUTF8AndSize(repr, &mut size) };
            return if !ptr.is_null() {
                let slice = unsafe {
                    std::str::from_utf8_unchecked(std::slice::from_raw_parts(
                        ptr.cast::<u8>(),
                        size as usize,
                    ))
                };
                let message = String::from(slice);
                unsafe { Py_DECREF(repr) };
                Err(ser::Error::custom(message))
            } else {
                Err(ser::Error::custom(
                    "Internal Error: Failed to convert exception to string",
                ))
            };
        }
    };
}

macro_rules! tri {
    ($expr:expr) => {
        match $expr {
            Ok(val) => val,
            Err(err) => return Err(err),
        }
    };
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
                let ptr = unsafe { PyUnicode_AsUTF8AndSize(self.object, &mut str_size) };
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
                        bail_on_integer_conversion_error!(exception);
                    };
                    #[cfg(not(Py_3_12))]
                    {
                        let mut ptype: *mut pyo3::ffi::PyObject = std::ptr::null_mut();
                        let mut pvalue: *mut pyo3::ffi::PyObject = std::ptr::null_mut();
                        let mut ptraceback: *mut pyo3::ffi::PyObject = std::ptr::null_mut();
                        unsafe { pyo3::ffi::PyErr_Fetch(&mut ptype, &mut pvalue, &mut ptraceback) };
                        bail_on_integer_conversion_error!(pvalue);
                    };
                }
                serializer.serialize_i64(value)
            }
            ObjectType::Float => {
                serializer.serialize_f64(unsafe { PyFloat_AS_DOUBLE(self.object) })
            }
            ObjectType::Bool => serializer.serialize_bool(self.object == unsafe { types::TRUE }),
            ObjectType::None => serializer.serialize_unit(),
            ObjectType::Dict => {
                if self.recursion_depth == RECURSION_LIMIT {
                    return Err(ser::Error::custom("Recursion limit reached"));
                }
                let length = unsafe { (*self.object.cast::<PyDictObject>()).ma_used } as usize;
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
                            pyo3::ffi::PyDict_Next(self.object, &mut pos, &mut key, &mut value);
                        }
                        tri!(check_type_is_str(key));
                        let ptr = unsafe { PyUnicode_AsUTF8AndSize(key, &mut str_size) };
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
                let length = unsafe { PyList_GET_SIZE(self.object) as usize };
                if length == 0 {
                    tri!(serializer.serialize_seq(Some(0))).end()
                } else {
                    let mut type_ptr = std::ptr::null_mut();
                    let mut ob_type = ObjectType::Str;
                    let mut sequence = tri!(serializer.serialize_seq(Some(length)));
                    for i in 0..length {
                        let elem = unsafe { PyList_GET_ITEM(self.object, i as isize) };
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
                let length = unsafe { PyTuple_GET_SIZE(self.object) as usize };
                if length == 0 {
                    tri!(serializer.serialize_seq(Some(0))).end()
                } else {
                    let mut type_ptr = std::ptr::null_mut();
                    let mut ob_type = ObjectType::Str;
                    let mut sequence = tri!(serializer.serialize_seq(Some(length)));
                    for i in 0..length {
                        let elem = unsafe { PyTuple_GET_ITEM(self.object, i as isize) };
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
                    unsafe { CStr::from_ptr((*object_type).tp_name).to_string_lossy() }
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
