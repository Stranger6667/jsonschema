use magnus::{function, Error, Ruby, Value};

macro_rules! bench_validator {
    ($is_valid:ident, $validate:ident, $struct:ident, $path:literal) => {
        #[jsonschema::validator(path = $path, backend = Magnus)]
        struct $struct;

        fn $is_valid(instance: Value) -> Result<bool, Error> {
            $struct::is_valid(&instance)
        }

        // Raising on an invalid instance keeps the shape of `Validator#validate!`.
        fn $validate(ruby: &Ruby, instance: Value) -> Result<(), Error> {
            match $struct::validate(&instance)? {
                Ok(()) => Ok(()),
                Err(error) => Err(Error::new(
                    ruby.exception_runtime_error(),
                    error.to_string(),
                )),
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

#[magnus::init(name = "jsonschema_bench_magnus")]
fn init(ruby: &Ruby) -> Result<(), Error> {
    let module = ruby.define_module("JSONSchemaBench")?;
    module.define_module_function("openapi_valid?", function!(openapi_is_valid, 1))?;
    module.define_module_function("openapi_validate!", function!(openapi_validate, 1))?;
    module.define_module_function("swagger_valid?", function!(swagger_is_valid, 1))?;
    module.define_module_function("swagger_validate!", function!(swagger_validate, 1))?;
    module.define_module_function("geojson_valid?", function!(geojson_is_valid, 1))?;
    module.define_module_function("geojson_validate!", function!(geojson_validate, 1))?;
    module.define_module_function("citm_valid?", function!(citm_is_valid, 1))?;
    module.define_module_function("citm_validate!", function!(citm_validate, 1))?;
    module.define_module_function("fast_valid?", function!(fast_is_valid, 1))?;
    module.define_module_function("fast_validate!", function!(fast_validate, 1))?;
    module.define_module_function("fhir_valid?", function!(fhir_is_valid, 1))?;
    module.define_module_function("fhir_validate!", function!(fhir_validate, 1))?;
    module.define_module_function("recursive_valid?", function!(recursive_is_valid, 1))?;
    module.define_module_function("recursive_validate!", function!(recursive_validate, 1))?;
    Ok(())
}
