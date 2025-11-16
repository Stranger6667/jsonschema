use pyo3::{
    ffi::{self, PyObject, PyTypeObject, Py_True},
    sync::PyOnceLock,
    types::{
        PyAnyMethods, PyBool, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple, PyType,
        PyTypeMethods,
    },
    Python,
};

pub static mut TRUE: *mut PyObject = std::ptr::null_mut::<PyObject>();

pub static mut STR_TYPE: *mut PyTypeObject = std::ptr::null_mut::<PyTypeObject>();
pub static mut INT_TYPE: *mut PyTypeObject = std::ptr::null_mut::<PyTypeObject>();
pub static mut BOOL_TYPE: *mut PyTypeObject = std::ptr::null_mut::<PyTypeObject>();
pub static mut NONE_TYPE: *mut PyTypeObject = std::ptr::null_mut::<PyTypeObject>();
pub static mut FLOAT_TYPE: *mut PyTypeObject = std::ptr::null_mut::<PyTypeObject>();
pub static mut LIST_TYPE: *mut PyTypeObject = std::ptr::null_mut::<PyTypeObject>();
pub static mut DICT_TYPE: *mut PyTypeObject = std::ptr::null_mut::<PyTypeObject>();
pub static mut TUPLE_TYPE: *mut PyTypeObject = std::ptr::null_mut::<PyTypeObject>();
pub static mut ENUM_TYPE: *mut PyTypeObject = std::ptr::null_mut::<PyTypeObject>();
pub static mut ENUM_BASE: *mut PyTypeObject = std::ptr::null_mut::<PyTypeObject>();
pub static mut VALUE_STR: *mut PyObject = std::ptr::null_mut::<PyObject>();

static INIT: PyOnceLock<()> = PyOnceLock::new();

fn look_up_enum_types(py: Python<'_>) -> (*mut PyTypeObject, *mut PyTypeObject) {
    let module = py
        .import("enum")
        .expect("failed to import the stdlib enum module");
    let enum_meta = module
        .getattr("EnumMeta")
        .expect("enum.EnumMeta is missing")
        .cast_into::<PyType>()
        .expect("enum.EnumMeta is not a type");
    let enum_base = module
        .getattr("Enum")
        .expect("enum.Enum is missing")
        .cast_into::<PyType>()
        .expect("enum.Enum is not a type");
    (enum_meta.as_type_ptr(), enum_base.as_type_ptr())
}

/// Set empty type object pointers with their actual values.
/// We need these Python-side type objects for direct comparison during conversion to serde types
/// NOTE. This function should be called before any serialization logic
pub fn init(py: Python<'_>) {
    INIT.get_or_init(py, || unsafe {
        TRUE = Py_True();
        STR_TYPE = py.get_type::<PyString>().as_type_ptr();
        DICT_TYPE = py.get_type::<PyDict>().as_type_ptr();
        TUPLE_TYPE = py.get_type::<PyTuple>().as_type_ptr();
        LIST_TYPE = py.get_type::<PyList>().as_type_ptr();
        NONE_TYPE = py.None().bind(py).get_type().as_type_ptr();
        BOOL_TYPE = py.get_type::<PyBool>().as_type_ptr();
        INT_TYPE = py.get_type::<PyInt>().as_type_ptr();
        FLOAT_TYPE = py.get_type::<PyFloat>().as_type_ptr();
        let (enum_meta, enum_base) = look_up_enum_types(py);
        ENUM_TYPE = enum_meta;
        ENUM_BASE = enum_base;
        VALUE_STR = ffi::PyUnicode_InternFromString(c"value".as_ptr());
    });
}
