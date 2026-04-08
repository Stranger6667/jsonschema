#![allow(clippy::needless_pass_by_value)]
use referencing::{Draft, Registry};
use referencing_testsuite::{suite, Test};

#[suite(
    path = "crates/jsonschema-referencing/tests/suite",
    drafts = [
        "json-schema-draft-04",
        "json-schema-draft-06",
        "json-schema-draft-07",
        "json-schema-draft-2019-09",
        "json-schema-draft-2020-12",
    ],
    xfail = [
        // `fluent-uri` does not normalize 80 port
        "json-schema-draft-04::rfc3986_normalization_on_insertion::test_5",
        "json-schema-draft-04::rfc3986_normalization_on_insertion::test_11",
        "json-schema-draft-04::rfc3986_normalization_on_retrieval::test_5",
        "json-schema-draft-04::rfc3986_normalization_on_retrieval::test_11",
        "json-schema-draft-06::rfc3986_normalization_on_insertion::test_5",
        "json-schema-draft-06::rfc3986_normalization_on_insertion::test_11",
        "json-schema-draft-06::rfc3986_normalization_on_retrieval::test_5",
        "json-schema-draft-06::rfc3986_normalization_on_retrieval::test_11",
        "json-schema-draft-07::rfc3986_normalization_on_insertion::test_5",
        "json-schema-draft-07::rfc3986_normalization_on_insertion::test_11",
        "json-schema-draft-07::rfc3986_normalization_on_retrieval::test_5",
        "json-schema-draft-07::rfc3986_normalization_on_retrieval::test_11",
        "json-schema-draft-2019-09::rfc3986_normalization_on_insertion::test_5",
        "json-schema-draft-2019-09::rfc3986_normalization_on_insertion::test_11",
        "json-schema-draft-2019-09::rfc3986_normalization_on_retrieval::test_5",
        "json-schema-draft-2019-09::rfc3986_normalization_on_retrieval::test_11",
        "json-schema-draft-2020-12::rfc3986_normalization_on_insertion::test_5",
        "json-schema-draft-2020-12::rfc3986_normalization_on_insertion::test_11",
        "json-schema-draft-2020-12::rfc3986_normalization_on_retrieval::test_5",
        "json-schema-draft-2020-12::rfc3986_normalization_on_retrieval::test_11",
    ]
)]
fn test_suite(draft: &'static str, test: Test) {
    let draft = match draft {
        "json-schema-draft-04" => Draft::Draft4,
        "json-schema-draft-06" => Draft::Draft6,
        "json-schema-draft-07" => Draft::Draft7,
        "json-schema-draft-2019-09" => Draft::Draft201909,
        "json-schema-draft-2020-12" => Draft::Draft202012,
        _ => panic!("Unknown draft"),
    };

    // Borrowed path: &Value entries, draft inferred from registry default
    let mut registry = Registry::new().draft(draft);
    for (uri, content) in &test.registry {
        registry = registry
            .add(uri.as_str(), content)
            .expect("Invalid registry input");
    }
    run_assertions(&registry.prepare().expect("Invalid registry"), &test);

    // Owned path: explicit Resource with draft
    let mut registry = Registry::new().draft(draft);
    for (uri, content) in &test.registry {
        registry = registry
            .add(uri.as_str(), draft.create_resource(content.clone()))
            .expect("Invalid registry input");
    }
    run_assertions(&registry.prepare().expect("Invalid registry"), &test);
}

fn run_assertions(registry: &Registry, test: &Test) {
    let resolver = registry.resolver(
        referencing::uri::from_str(test.base_uri.unwrap_or_default()).expect("Invalid base URI"),
    );
    if test.error.is_some() {
        assert!(resolver.lookup(test.reference).is_err());
    } else {
        let mut resolved = resolver.lookup(test.reference).expect("Invalid reference");
        assert_eq!(
            resolved.contents(),
            test.target.as_ref().expect("Should be present")
        );
        let mut then = test.then.as_deref();
        while let Some(then_) = then {
            resolved = resolved
                .resolver()
                .lookup(then_.reference)
                .expect("Invalid reference");
            assert_eq!(
                resolved.contents(),
                then_.target.as_ref().expect("Should be present")
            );
            then = then_.then.as_deref();
        }
    }
}
