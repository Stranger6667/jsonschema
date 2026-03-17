#![no_main]
use libfuzzer_sys::fuzz_target;
use referencing::{uri, Draft, Registry, RegistryBuilder};

fuzz_target!(|data: (&[u8], &[u8], &[u8])| {
    let (schema, base, reference) = data;
    if let Ok(schema) = serde_json::from_slice::<serde_json::Value>(schema) {
        if let Ok(base) = std::str::from_utf8(base) {
            if let Ok(reference) = std::str::from_utf8(reference) {
                for draft in [
                    Draft::Draft4,
                    Draft::Draft6,
                    Draft::Draft7,
                    Draft::Draft201909,
                    Draft::Draft202012,
                ] {
                    if let Ok(registry) = Registry::new()
                        .draft(draft)
                        .add(base, &schema)
                        .and_then(RegistryBuilder::prepare)
                    {
                        let resolver = registry
                            .resolver(uri::from_str("http://example.com/schema.json").unwrap());
                        let _resolved = resolver.lookup(reference);
                    }
                }
            }
        }
    }
});
