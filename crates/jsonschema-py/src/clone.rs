use pyo3::{
    exceptions,
    ffi::{self, PyType_IsSubtype, Py_TYPE},
    prelude::*,
};

use crate::{
    ser::{dict_len, pylist_get_item, pylist_len, RECURSION_LIMIT},
    types,
};

/// Deep-clone a JSON-compatible Python object.
///
/// Only `dict` and `list` are copied. All other values (str, int, float, bool, None)
/// are shared by reference — they are immutable and safe to reuse.
///
/// Dict subclasses (e.g. `CaseInsensitiveDict`) are cloned into plain `dict`.
/// List subclasses and all other types are treated as immutable (incref + return).
///
/// # Safety
/// Caller must hold the GIL.
pub(crate) unsafe fn clone_impl(
    object: *mut ffi::PyObject,
    depth: u8,
) -> PyResult<*mut ffi::PyObject> {
    let object_type = Py_TYPE(object);

    // Hot path: immutable leaf values — one pointer comparison each, no allocation.
    // str is the most common value in JSON Schema documents (type names, $ref paths, etc.).
    if object_type == unsafe { types::STR_TYPE } {
        ffi::Py_IncRef(object);
        return Ok(object);
    }
    // BOOL_TYPE must precede INT_TYPE: Python bool is a subtype of int, but the
    // type pointer for `True`/`False` is BOOL_TYPE, not INT_TYPE. Exact pointer
    // comparison is used throughout, so ordering only matters conceptually here,
    // but listing bool first makes the intent explicit.
    if object_type == unsafe { types::BOOL_TYPE } {
        ffi::Py_IncRef(object);
        return Ok(object);
    }
    if object_type == unsafe { types::INT_TYPE } {
        ffi::Py_IncRef(object);
        return Ok(object);
    }
    if object_type == unsafe { types::FLOAT_TYPE } {
        ffi::Py_IncRef(object);
        return Ok(object);
    }
    if object_type == unsafe { types::NONE_TYPE } {
        ffi::Py_IncRef(object);
        return Ok(object);
    }

    // Recursion guard — checked only when we are about to enter a container.
    if depth == RECURSION_LIMIT {
        return Err(exceptions::PyValueError::new_err("Recursion limit reached"));
    }

    // Exact dict and list matches — common containers in JSON Schema.
    if object_type == unsafe { types::DICT_TYPE } {
        return clone_dict(object, depth + 1);
    }
    if object_type == unsafe { types::LIST_TYPE } {
        return clone_list(object, depth + 1);
    }

    // Dict subclass fallback (e.g. CaseInsensitiveDict from requests).
    // Rare in practice; cloned into a plain dict.
    if unsafe { PyType_IsSubtype(object_type, types::DICT_TYPE) } != 0 {
        return clone_dict(object, depth + 1);
    }

    // Everything else (tuple, custom objects, etc.) — treated as immutable.
    ffi::Py_IncRef(object);
    Ok(object)
}

/// Clone a Python dict into a new plain dict.
///
/// Start with a shallow `PyDict_Copy` and only recurse for nested containers
/// (`dict`, `list`, dict subclasses). Leaf values stay as-is from the shallow copy.
///
/// `PyDict_SetItem` increments the ref count of both key and value, so we
/// DECREF the owned `cloned_value` after the insert.
unsafe fn clone_dict(object: *mut ffi::PyObject, depth: u8) -> PyResult<*mut ffi::PyObject> {
    let output = ffi::PyDict_Copy(object);
    if output.is_null() {
        return Err(exceptions::PyValueError::new_err("Failed to copy dict"));
    }

    let size = dict_len(object);
    if size == 0 {
        return Ok(output);
    }

    let mut pos = 0_isize;
    let mut key: *mut ffi::PyObject = std::ptr::null_mut();
    let mut value: *mut ffi::PyObject = std::ptr::null_mut();

    for _ in 0..size {
        if ffi::PyDict_Next(object, &raw mut pos, &raw mut key, &raw mut value) == 0 {
            break;
        }

        let value_type = Py_TYPE(value);
        let needs_deep_clone = if value_type == unsafe { types::DICT_TYPE }
            || value_type == unsafe { types::LIST_TYPE }
        {
            true
        } else if value_type == unsafe { types::STR_TYPE }
            || value_type == unsafe { types::BOOL_TYPE }
            || value_type == unsafe { types::INT_TYPE }
            || value_type == unsafe { types::FLOAT_TYPE }
            || value_type == unsafe { types::NONE_TYPE }
        {
            false
        } else {
            PyType_IsSubtype(value_type, types::DICT_TYPE) != 0
        };

        if !needs_deep_clone {
            continue;
        }

        match clone_impl(value, depth) {
            Ok(cloned_value) => {
                let rc = ffi::PyDict_SetItem(output, key, cloned_value);
                // SetItem increfs both key and value — drop our owned ref to cloned_value.
                ffi::Py_DECREF(cloned_value);
                if rc < 0 {
                    ffi::Py_DECREF(output);
                    return Err(exceptions::PyValueError::new_err("Failed to set dict item"));
                }
            }
            Err(e) => {
                ffi::Py_DECREF(output);
                return Err(e);
            }
        }
    }

    Ok(output)
}

/// Clone a Python list into a new list of the same length.
///
/// Start with a shallow `PyList_GetSlice` copy and only recurse for nested
/// containers (`dict`, `list`, dict subclasses). Leaf values stay as-is from
/// the shallow copy.
///
/// `PyList_SetItem` *steals* the reference — no DECREF needed after the call.
unsafe fn clone_list(object: *mut ffi::PyObject, depth: u8) -> PyResult<*mut ffi::PyObject> {
    let size = pylist_len(object);
    let output = ffi::PyList_GetSlice(object, 0, size as ffi::Py_ssize_t);
    if output.is_null() {
        return Err(exceptions::PyValueError::new_err("Failed to copy list"));
    }

    for i in 0..size {
        let item = pylist_get_item(object, i as ffi::Py_ssize_t);
        if item.is_null() {
            ffi::Py_DECREF(output);
            return Err(exceptions::PyValueError::new_err("Failed to get list item"));
        }

        let item_type = Py_TYPE(item);
        let needs_deep_clone = if item_type == unsafe { types::DICT_TYPE }
            || item_type == unsafe { types::LIST_TYPE }
        {
            true
        } else if item_type == unsafe { types::STR_TYPE }
            || item_type == unsafe { types::BOOL_TYPE }
            || item_type == unsafe { types::INT_TYPE }
            || item_type == unsafe { types::FLOAT_TYPE }
            || item_type == unsafe { types::NONE_TYPE }
        {
            false
        } else {
            PyType_IsSubtype(item_type, types::DICT_TYPE) != 0
        };

        if !needs_deep_clone {
            continue;
        }

        match clone_impl(item, depth) {
            Ok(cloned) => {
                // SetItem steals the reference even on failure — do NOT Py_DECREF(cloned).
                let rc = ffi::PyList_SetItem(output, i as ffi::Py_ssize_t, cloned);
                if rc < 0 {
                    ffi::Py_DECREF(output);
                    return Err(exceptions::PyValueError::new_err("Failed to set list item"));
                }
            }
            Err(e) => {
                ffi::Py_DECREF(output);
                return Err(e);
            }
        }
    }

    Ok(output)
}

/// `canonical.schema.clone(object) -> object`
///
/// Deep-clone a JSON-compatible Python object. Only `dict` and `list` are
/// copied; all other values are shared by reference.
#[pyfunction(name = "clone")]
#[allow(unsafe_code)]
pub(crate) fn canonical_schema_clone(object: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let ptr = unsafe { clone_impl(object.as_ptr(), 0) }?;
    // SAFETY: clone_impl returns a valid PyObject with an owned reference.
    Ok(unsafe { Bound::from_owned_ptr(object.py(), ptr) }.unbind())
}
