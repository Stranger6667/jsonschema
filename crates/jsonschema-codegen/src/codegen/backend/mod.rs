use proc_macro2::TokenStream;
use referencing::Draft;

use crate::codegen::symbols::EmitSymbols;

pub(crate) mod serde_json;
pub(crate) mod stub;

pub(crate) trait BackendIdentity {
    fn id(&self) -> &'static str;
}

pub(crate) trait BackendSymbols {
    fn emit_symbols(&self) -> EmitSymbols;
}

pub(crate) trait BackendTypeChecks {
    fn instance_is_string(&self) -> TokenStream;
    fn instance_is_number(&self) -> TokenStream;
    fn instance_is_boolean(&self) -> TokenStream;
    fn instance_is_null(&self) -> TokenStream;
    fn instance_is_array(&self) -> TokenStream;
    fn instance_is_object(&self) -> TokenStream;
    fn instance_as_bool(&self) -> TokenStream;
    fn instance_as_str(&self) -> TokenStream;
    fn integer_number_guard(&self, draft: Draft) -> TokenStream;
    fn instance_is_integer(&self, draft: Draft) -> TokenStream;
}

pub(crate) trait BackendMatchArms {
    fn match_string_arm(&self, body: TokenStream) -> TokenStream;
    fn match_number_arm(&self, body: TokenStream) -> TokenStream;
    fn match_boolean_arm(&self, body: TokenStream) -> TokenStream;
    fn match_integer_arm(&self, guard: TokenStream, body: TokenStream) -> TokenStream;
    fn match_array_arm(&self, body: TokenStream) -> TokenStream;
    fn match_object_arm(&self, body: TokenStream) -> TokenStream;
}

pub(crate) trait BackendAccessors {
    fn string_as_str(&self, string_expr: TokenStream) -> TokenStream;
    fn array_len(&self, array_expr: TokenStream) -> TokenStream;
    fn object_len(&self, object_expr: TokenStream) -> TokenStream;
    fn object_contains_key(&self, object_expr: TokenStream, key: &str) -> TokenStream;
    fn object_iter_all(&self, object_expr: TokenStream, body: TokenStream) -> TokenStream;
    fn key_as_str(&self, key_expr: TokenStream) -> TokenStream;
    fn key_as_value_ref(&self, key_expr: TokenStream) -> TokenStream;
    fn instance_object_property_as_str(&self, key: &str) -> TokenStream;
}

pub(crate) trait BackendPatterns {
    fn pattern_string(&self) -> TokenStream;
    fn pattern_number(&self) -> TokenStream;
    fn pattern_number_binding(&self) -> TokenStream;
    fn pattern_integer(&self, guard: TokenStream) -> TokenStream;
    fn pattern_array(&self) -> TokenStream;
    fn pattern_object(&self) -> TokenStream;
    fn pattern_boolean(&self) -> TokenStream;
    fn pattern_null(&self) -> TokenStream;
}

pub(crate) trait Backend:
    BackendIdentity
    + BackendSymbols
    + BackendTypeChecks
    + BackendMatchArms
    + BackendAccessors
    + BackendPatterns
{
}

impl<T> Backend for T where
    T: BackendIdentity
        + BackendSymbols
        + BackendTypeChecks
        + BackendMatchArms
        + BackendAccessors
        + BackendPatterns
{
}

macro_rules! forward_backend_method {
    ($(fn $name:ident(&self $(, $arg:ident: $ty:ty)*) -> $ret:ty;)+) => {
        $(
            pub(crate) fn $name(&self $(, $arg: $ty)*) -> $ret {
                self.backend().$name($($arg),*)
            }
        )+
    };
}

#[derive(Clone)]
pub(crate) enum BackendKind {
    SerdeJson(serde_json::SerdeJsonBackend),
    PyO3Stub(stub::StubBackend),
    MagnusStub(stub::StubBackend),
}

impl BackendKind {
    pub(crate) fn serde_json() -> Self {
        Self::SerdeJson(serde_json::SerdeJsonBackend)
    }

    pub(crate) fn compile_only_stub_variants() -> [Self; 2] {
        [
            Self::PyO3Stub(stub::StubBackend::new("pyo3_stub")),
            Self::MagnusStub(stub::StubBackend::new("magnus_stub")),
        ]
    }

    fn backend(&self) -> &(dyn Backend + '_) {
        match self {
            Self::SerdeJson(backend) => backend,
            Self::PyO3Stub(backend) | Self::MagnusStub(backend) => backend,
        }
    }

    forward_backend_method! {
        fn id(&self) -> &'static str;
        fn emit_symbols(&self) -> EmitSymbols;
        fn instance_is_string(&self) -> TokenStream;
        fn instance_is_number(&self) -> TokenStream;
        fn instance_is_boolean(&self) -> TokenStream;
        fn instance_is_null(&self) -> TokenStream;
        fn instance_is_array(&self) -> TokenStream;
        fn instance_is_object(&self) -> TokenStream;
        fn instance_as_bool(&self) -> TokenStream;
        fn instance_as_str(&self) -> TokenStream;
        fn integer_number_guard(&self, draft: Draft) -> TokenStream;
        fn instance_is_integer(&self, draft: Draft) -> TokenStream;
        fn match_string_arm(&self, body: TokenStream) -> TokenStream;
        fn match_number_arm(&self, body: TokenStream) -> TokenStream;
        fn match_boolean_arm(&self, body: TokenStream) -> TokenStream;
        fn match_integer_arm(&self, guard: TokenStream, body: TokenStream) -> TokenStream;
        fn match_array_arm(&self, body: TokenStream) -> TokenStream;
        fn match_object_arm(&self, body: TokenStream) -> TokenStream;
        fn string_as_str(&self, string_expr: TokenStream) -> TokenStream;
        fn array_len(&self, array_expr: TokenStream) -> TokenStream;
        fn object_len(&self, object_expr: TokenStream) -> TokenStream;
        fn object_contains_key(&self, object_expr: TokenStream, key: &str) -> TokenStream;
        fn object_iter_all(&self, object_expr: TokenStream, body: TokenStream) -> TokenStream;
        fn key_as_str(&self, key_expr: TokenStream) -> TokenStream;
        fn key_as_value_ref(&self, key_expr: TokenStream) -> TokenStream;
        fn instance_object_property_as_str(&self, key: &str) -> TokenStream;
        fn pattern_string(&self) -> TokenStream;
        fn pattern_number(&self) -> TokenStream;
        fn pattern_number_binding(&self) -> TokenStream;
        fn pattern_integer(&self, guard: TokenStream) -> TokenStream;
        fn pattern_array(&self) -> TokenStream;
        fn pattern_object(&self) -> TokenStream;
        fn pattern_boolean(&self) -> TokenStream;
        fn pattern_null(&self) -> TokenStream;
    }
}
