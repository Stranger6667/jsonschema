use proc_macro2::TokenStream;
use referencing::Draft;

use crate::codegen::symbols::EmitSymbols;

use super::{
    BackendAccessors, BackendIdentity, BackendMatchArms, BackendPatterns, BackendSymbols,
    BackendTypeChecks,
};

#[derive(Clone)]
pub(crate) struct StubBackend {
    id: &'static str,
}

impl StubBackend {
    pub(crate) const fn new(id: &'static str) -> Self {
        Self { id }
    }

    fn unimplemented(&self, method: &str) -> ! {
        todo!("{method} is not implemented for backend `{}`", self.id)
    }
}

impl BackendIdentity for StubBackend {
    fn id(&self) -> &'static str {
        self.id
    }
}

impl BackendSymbols for StubBackend {
    fn emit_symbols(&self) -> EmitSymbols {
        self.unimplemented("emit_symbols")
    }
}

impl BackendTypeChecks for StubBackend {
    fn instance_is_string(&self) -> TokenStream {
        self.unimplemented("instance_is_string")
    }

    fn instance_is_number(&self) -> TokenStream {
        self.unimplemented("instance_is_number")
    }

    fn instance_is_boolean(&self) -> TokenStream {
        self.unimplemented("instance_is_boolean")
    }

    fn instance_is_null(&self) -> TokenStream {
        self.unimplemented("instance_is_null")
    }

    fn instance_is_array(&self) -> TokenStream {
        self.unimplemented("instance_is_array")
    }

    fn instance_is_object(&self) -> TokenStream {
        self.unimplemented("instance_is_object")
    }

    fn instance_as_bool(&self) -> TokenStream {
        self.unimplemented("instance_as_bool")
    }

    fn instance_as_str(&self) -> TokenStream {
        self.unimplemented("instance_as_str")
    }

    fn integer_number_guard(&self, _draft: Draft) -> TokenStream {
        self.unimplemented("integer_number_guard")
    }

    fn instance_is_integer(&self, _draft: Draft) -> TokenStream {
        self.unimplemented("instance_is_integer")
    }
}

impl BackendMatchArms for StubBackend {
    fn match_string_arm(&self, _body: TokenStream) -> TokenStream {
        self.unimplemented("match_string_arm")
    }

    fn match_number_arm(&self, _body: TokenStream) -> TokenStream {
        self.unimplemented("match_number_arm")
    }

    fn match_boolean_arm(&self, _body: TokenStream) -> TokenStream {
        self.unimplemented("match_boolean_arm")
    }

    fn match_integer_arm(&self, _guard: TokenStream, _body: TokenStream) -> TokenStream {
        self.unimplemented("match_integer_arm")
    }

    fn match_array_arm(&self, _body: TokenStream) -> TokenStream {
        self.unimplemented("match_array_arm")
    }

    fn match_object_arm(&self, _body: TokenStream) -> TokenStream {
        self.unimplemented("match_object_arm")
    }
}

impl BackendAccessors for StubBackend {
    fn string_as_str(&self, _string_expr: TokenStream) -> TokenStream {
        self.unimplemented("string_as_str")
    }

    fn array_len(&self, _array_expr: TokenStream) -> TokenStream {
        self.unimplemented("array_len")
    }

    fn object_len(&self, _object_expr: TokenStream) -> TokenStream {
        self.unimplemented("object_len")
    }

    fn object_contains_key(&self, _object_expr: TokenStream, _key: &str) -> TokenStream {
        self.unimplemented("object_contains_key")
    }

    fn object_iter_all(&self, _object_expr: TokenStream, _body: TokenStream) -> TokenStream {
        self.unimplemented("object_iter_all")
    }

    fn key_as_str(&self, _key_expr: TokenStream) -> TokenStream {
        self.unimplemented("key_as_str")
    }

    fn key_as_value_ref(&self, _key_expr: TokenStream) -> TokenStream {
        self.unimplemented("key_as_value_ref")
    }

    fn instance_object_property_as_str(&self, _key: &str) -> TokenStream {
        self.unimplemented("instance_object_property_as_str")
    }
}

impl BackendPatterns for StubBackend {
    fn pattern_string(&self) -> TokenStream {
        self.unimplemented("pattern_string")
    }

    fn pattern_number(&self) -> TokenStream {
        self.unimplemented("pattern_number")
    }

    fn pattern_number_binding(&self) -> TokenStream {
        self.unimplemented("pattern_number_binding")
    }

    fn pattern_integer(&self, _guard: TokenStream) -> TokenStream {
        self.unimplemented("pattern_integer")
    }

    fn pattern_array(&self) -> TokenStream {
        self.unimplemented("pattern_array")
    }

    fn pattern_object(&self) -> TokenStream {
        self.unimplemented("pattern_object")
    }

    fn pattern_boolean(&self) -> TokenStream {
        self.unimplemented("pattern_boolean")
    }

    fn pattern_null(&self) -> TokenStream {
        self.unimplemented("pattern_null")
    }
}
