use pyo3::ffi::{
    PyDict_New, PyFloat_FromDouble, PyImport_ImportModule, PyList_New, PyLong_FromLongLong,
    PyMapping_GetItemString, PyObject, PyObject_GenericGetDict, PyTuple_New, PyTypeObject,
    PyUnicode_New, Py_DECREF, Py_None, Py_TYPE, Py_True,
};
use std::sync::Once;

pub static mut TRUE: *mut pyo3::ffi::PyObject = std::ptr::null_mut::<PyObject>();

pub static mut STR_TYPE: *mut PyTypeObject = std::ptr::null_mut::<PyTypeObject>();
pub static mut INT_TYPE: *mut PyTypeObject = std::ptr::null_mut::<PyTypeObject>();
pub static mut BOOL_TYPE: *mut PyTypeObject = std::ptr::null_mut::<PyTypeObject>();
pub static mut NONE_TYPE: *mut PyTypeObject = std::ptr::null_mut::<PyTypeObject>();
pub static mut FLOAT_TYPE: *mut PyTypeObject = std::ptr::null_mut::<PyTypeObject>();
pub static mut LIST_TYPE: *mut PyTypeObject = std::ptr::null_mut::<PyTypeObject>();
pub static mut DICT_TYPE: *mut PyTypeObject = std::ptr::null_mut::<PyTypeObject>();
pub static mut TUPLE_TYPE: *mut PyTypeObject = std::ptr::null_mut::<PyTypeObject>();
pub static mut ENUM_TYPE: *mut PyTypeObject = std::ptr::null_mut::<PyTypeObject>();
pub static mut VALUE_STR: *mut PyObject = std::ptr::null_mut::<PyObject>();

static INIT: Once = Once::new();

// Taken from orjson
#[cold]
unsafe fn look_up_enum_type() -> *mut PyTypeObject {
    let module = PyImport_ImportModule(c"enum".as_ptr());
    let module_dict = PyObject_GenericGetDict(module, std::ptr::null_mut());
    let ptr = PyMapping_GetItemString(module_dict, c"EnumMeta".as_ptr()).cast::<PyTypeObject>();
    Py_DECREF(module_dict);
    Py_DECREF(module);
    ptr
}

/// Set empty type object pointers with their actual values.
/// We need these Python-side type objects for direct comparison during conversion to serde types
/// NOTE. This function should be called before any serialization logic
pub fn init() {
    INIT.call_once(|| unsafe {
        TRUE = Py_True();
        STR_TYPE = Py_TYPE(PyUnicode_New(0, 255));
        DICT_TYPE = Py_TYPE(PyDict_New());
        TUPLE_TYPE = Py_TYPE(PyTuple_New(0_isize));
        LIST_TYPE = Py_TYPE(PyList_New(0_isize));
        NONE_TYPE = Py_TYPE(Py_None());
        BOOL_TYPE = Py_TYPE(TRUE);
        INT_TYPE = Py_TYPE(PyLong_FromLongLong(0));
        FLOAT_TYPE = Py_TYPE(PyFloat_FromDouble(0.0));
        ENUM_TYPE = look_up_enum_type();
        VALUE_STR = pyo3::ffi::PyUnicode_InternFromString(c"value".as_ptr());
    });
}
