#[test]
fn ui_compile_failures() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
    // This case pins the message the macro emits when the backend is not compiled in, which it
    // can only do while the backend is not compiled in.
    #[cfg(not(feature = "pyo3"))]
    t.compile_fail("tests/ui/no-pyo3/*.rs");
}
