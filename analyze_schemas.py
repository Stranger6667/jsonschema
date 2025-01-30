import json
from pathlib import Path
from jschon import create_catalog, JSON, JSONSchema, URI, LocalSource


def load_test_suite(path):
    with open(path) as f:
        return json.load(f)


def process_test_case(schema, test_cases):
    results = []

    # Create schema
    if isinstance(schema, dict) and schema.get("$defs") == {
        "true": True,
        "false": False,
    }:
        return []
    schema = JSONSchema(
        schema,
        uri=URI("urn:test"),
        metaschema_uri=URI("https://json-schema.org/draft/2020-12/schema"),
    )

    for test in test_cases:
        if not test.get("valid", True):  # Only process invalid cases
            instance = test["data"]
            validation_result = schema.evaluate(JSON(instance))

            # Get the validation output
            output = validation_result.output("basic")

            results.append({"instance": instance, "errors": output.get("errors", [])})

    return results


def main():
    catalog = create_catalog("2020-12")

    test_suite_path = Path("crates/jsonschema/tests/suite/tests/draft2020-12")

    catalog.add_uri_source(
        URI("http://localhost:1234/"),
        LocalSource(Path("crates/jsonschema/tests/suite/remotes/"), suffix=""),
    )
    catalog.add_uri_source(
        URI("https://json-schema.org/draft/2020-12/"),
        LocalSource(
            Path("crates/jsonschema-referencing/metaschemas/draft2020-12"),
            suffix=".json",
        ),
    )

    output = {"tests": []}

    # Process each test file
    for test_file in test_suite_path.glob("*.json"):
        test_cases = load_test_suite(test_file)

        for test_group in test_cases:
            schema = test_group["schema"]
            instances = process_test_case(schema, test_group["tests"])

            if instances:
                output["tests"].append(
                    {
                        "schema": schema,
                        "schema_id": test_file.stem,
                        "instances": instances,
                    }
                )

    # Save the dataset
    with open("output_basic_draft2020_12.json", "w") as f:
        json.dump(output, f, indent=2)


if __name__ == "__main__":
    main()
