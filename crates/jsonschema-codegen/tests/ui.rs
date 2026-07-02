#[test]
fn ui_compile_failures() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
