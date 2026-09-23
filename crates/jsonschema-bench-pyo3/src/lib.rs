use pyo3::{exceptions::PyValueError, prelude::*, types::PyAny};

macro_rules! bench_validator {
    ($is_valid:ident, $validate:ident, $struct:ident, $path:literal) => {
        #[jsonschema::validator(path = $path, backend = Pyo3)]
        struct $struct;

        #[pyfunction]
        fn $is_valid(instance: &Bound<'_, PyAny>) -> PyResult<bool> {
            $struct::is_valid(instance)
        }

        // Raising on an invalid instance keeps the shape of `Validator.validate`.
        #[pyfunction]
        fn $validate(instance: &Bound<'_, PyAny>) -> PyResult<()> {
            match $struct::validate(instance)? {
                Ok(()) => Ok(()),
                Err(error) => Err(PyValueError::new_err(error.to_string())),
            }
        }
    };
}

bench_validator!(
    openapi_is_valid,
    openapi_validate,
    OpenApi,
    "../benchmark/data/openapi.json"
);
bench_validator!(
    swagger_is_valid,
    swagger_validate,
    Swagger,
    "../benchmark/data/swagger.json"
);
bench_validator!(
    geojson_is_valid,
    geojson_validate,
    GeoJson,
    "../benchmark/data/geojson.json"
);
bench_validator!(
    citm_is_valid,
    citm_validate,
    Citm,
    "../benchmark/data/citm_catalog_schema.json"
);
bench_validator!(
    fast_is_valid,
    fast_validate,
    Fast,
    "../benchmark/data/fast_schema.json"
);
bench_validator!(
    fhir_is_valid,
    fhir_validate,
    Fhir,
    "../benchmark/data/fhir.schema.json"
);
bench_validator!(
    recursive_is_valid,
    recursive_validate,
    Recursive,
    "../benchmark/data/recursive_schema.json"
);

#[pymodule]
fn jsonschema_bench_pyo3(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(openapi_is_valid, module)?)?;
    module.add_function(wrap_pyfunction!(openapi_validate, module)?)?;
    module.add_function(wrap_pyfunction!(swagger_is_valid, module)?)?;
    module.add_function(wrap_pyfunction!(swagger_validate, module)?)?;
    module.add_function(wrap_pyfunction!(geojson_is_valid, module)?)?;
    module.add_function(wrap_pyfunction!(geojson_validate, module)?)?;
    module.add_function(wrap_pyfunction!(citm_is_valid, module)?)?;
    module.add_function(wrap_pyfunction!(citm_validate, module)?)?;
    module.add_function(wrap_pyfunction!(fast_is_valid, module)?)?;
    module.add_function(wrap_pyfunction!(fast_validate, module)?)?;
    module.add_function(wrap_pyfunction!(fhir_is_valid, module)?)?;
    module.add_function(wrap_pyfunction!(fhir_validate, module)?)?;
    module.add_function(wrap_pyfunction!(recursive_is_valid, module)?)?;
    module.add_function(wrap_pyfunction!(recursive_validate, module)?)?;
    Ok(())
}
