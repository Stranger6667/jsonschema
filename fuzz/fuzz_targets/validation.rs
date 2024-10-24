#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: (&[u8], &[u8])| {
    let (schema, instance) = data;
    if let Ok(schema) = serde_json::from_slice(schema) {
        if let Ok(validator) = jsonschema::validator_for(&schema) {
            if let Ok(instance) = serde_json::from_slice(instance) {
                let _ = validator.is_valid(&instance);
                let _ = validator.validate(&instance);
                for error in validator.iter_errors(&instance) {
                    let _ = error.to_string();
                }
                let output = validator.apply(&instance).basic();
                let _ = serde_json::to_value(output).expect("Failed to serialize");
            }
        }
    }
});
