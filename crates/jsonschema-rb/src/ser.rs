//! Serialization between Ruby values and `serde_json::Value`.
use magnus::{
    gc::register_mark_object,
    prelude::*,
    rb_sys::AsRawValue,
    value::{Lazy, ReprValue},
    Error, Integer, RArray, RClass, RHash, RString, Ruby, Symbol, TryConvert, Value,
};
use rb_sys::{ruby_value_type, RB_TYPE};
use serde_json::{Map, Number, Value as JsonValue};
use std::fmt;

static BIG_DECIMAL_CLASS: Lazy<RClass> = Lazy::new(|ruby| {
    // Ensure bigdecimal is loaded
    let _: Value = ruby
        .eval("require 'bigdecimal'")
        .expect("Failed to require bigdecimal");
    let cls: RClass = ruby
        .eval("BigDecimal")
        .expect("BigDecimal class must exist");
    register_mark_object(cls);
    cls
});

const RECURSION_LIMIT: u16 = 255;

#[inline]
pub fn to_value(ruby: &Ruby, value: Value) -> Result<JsonValue, Error> {
    to_value_recursive(ruby, value, 0)
}

/// Convert a Ruby value in schema position to a `serde_json::Value`.
///
/// If the value is a String, attempt to parse it as JSON first.
/// This allows passing JSON strings as schemas (e.g. `'{"type":"integer"}'`).
/// If parsing fails, falls back to treating it as a plain string value.
#[inline]
pub fn to_schema_value(ruby: &Ruby, value: Value) -> Result<JsonValue, Error> {
    // SAFETY: We're reading the type tag of a valid Ruby value
    #[allow(unsafe_code)]
    let value_type = unsafe { RB_TYPE(value.as_raw()) };
    if value_type == ruby_value_type::RUBY_T_STRING {
        if let Some(rstring) = RString::from_value(value) {
            // SAFETY: rstring is valid and we're in Ruby VM context
            #[allow(unsafe_code)]
            let bytes = unsafe { rstring.as_slice() };
            if let Ok(parsed) = serde_json::from_slice(bytes) {
                return Ok(parsed);
            }
        }
    }
    to_value_typed(ruby, value, value_type, 0)
}

fn to_value_recursive(ruby: &Ruby, value: Value, depth: u16) -> Result<JsonValue, Error> {
    if value.is_nil() {
        return Ok(JsonValue::Null);
    }

    // SAFETY: We're reading the type tag of a valid Ruby value
    #[allow(unsafe_code)]
    let value_type = unsafe { RB_TYPE(value.as_raw()) };

    to_value_typed(ruby, value, value_type, depth)
}

fn to_value_typed(
    ruby: &Ruby,
    value: Value,
    value_type: ruby_value_type,
    depth: u16,
) -> Result<JsonValue, Error> {
    match value_type {
        ruby_value_type::RUBY_T_TRUE => Ok(JsonValue::Bool(true)),
        ruby_value_type::RUBY_T_FALSE => Ok(JsonValue::Bool(false)),
        ruby_value_type::RUBY_T_FIXNUM | ruby_value_type::RUBY_T_BIGNUM => {
            convert_integer(ruby, value)
        }
        ruby_value_type::RUBY_T_FLOAT => {
            let f = f64::try_convert(value)?;
            Number::from_f64(f).map(JsonValue::Number).ok_or_else(|| {
                Error::new(
                    ruby.exception_arg_error(),
                    "Cannot convert NaN or Infinity to JSON",
                )
            })
        }
        ruby_value_type::RUBY_T_STRING => {
            let Some(rstring) = RString::from_value(value) else {
                unreachable!("We checked the type tag")
            };
            // SAFETY: rstring is valid and we're in Ruby VM context
            #[allow(unsafe_code)]
            let bytes = unsafe { rstring.as_slice() };
            match std::str::from_utf8(bytes) {
                Ok(s) => Ok(JsonValue::String(s.to_owned())),
                Err(_) => Err(Error::new(
                    ruby.exception_encoding_error(),
                    "String is not valid UTF-8",
                )),
            }
        }
        ruby_value_type::RUBY_T_SYMBOL => {
            let Some(sym) = Symbol::from_value(value) else {
                unreachable!("We checked the type tag")
            };
            let name = sym.name()?;
            Ok(JsonValue::String(name.to_string()))
        }
        ruby_value_type::RUBY_T_ARRAY => {
            if depth >= RECURSION_LIMIT {
                return Err(Error::new(
                    ruby.exception_arg_error(),
                    format!("Exceeded maximum nesting depth ({RECURSION_LIMIT})"),
                ));
            }
            let Some(arr) = RArray::from_value(value) else {
                unreachable!("We checked the type tag")
            };
            let len = arr.len();
            let mut json_arr = Vec::with_capacity(len);
            // Do not use `RArray::as_slice` here: recursive conversion may call
            // Ruby APIs for nested values, and `as_slice` borrows Ruby-managed
            // memory that must not be held across Ruby calls/GC.
            for idx in 0..len {
                let idx = isize::try_from(idx).map_err(|_| {
                    Error::new(
                        ruby.exception_arg_error(),
                        "Array index exceeds supported range",
                    )
                })?;
                let item: Value = arr.entry(idx)?;
                json_arr.push(to_value_recursive(ruby, item, depth + 1)?);
            }
            Ok(JsonValue::Array(json_arr))
        }
        ruby_value_type::RUBY_T_HASH => {
            if depth >= RECURSION_LIMIT {
                return Err(Error::new(
                    ruby.exception_arg_error(),
                    format!("Exceeded maximum nesting depth ({RECURSION_LIMIT})"),
                ));
            }
            let Some(hash) = RHash::from_value(value) else {
                unreachable!("We checked the type tag")
            };
            let mut map = Map::with_capacity(hash.len());
            hash.foreach(|key: Value, val: Value| {
                let key_str = hash_key_to_string(ruby, key)?;
                let json_val = to_value_recursive(ruby, val, depth + 1)?;
                map.insert(key_str, json_val);
                Ok(magnus::r_hash::ForEach::Continue)
            })?;
            Ok(JsonValue::Object(map))
        }
        ruby_value_type::RUBY_T_DATA if value.is_kind_of(ruby.get_inner(&BIG_DECIMAL_CLASS)) => {
            convert_big_decimal(ruby, value)
        }
        _ => {
            let class = value.class();
            #[allow(unsafe_code)]
            let class_name = unsafe { class.name() };
            Err(Error::new(
                ruby.exception_type_error(),
                format!("Unsupported type: '{class_name}'"),
            ))
        }
    }
}

/// Convert Ruby BigDecimal to JSON Number while preserving precision.
#[inline]
fn convert_big_decimal(ruby: &Ruby, value: Value) -> Result<JsonValue, Error> {
    let decimal_text: String = value.funcall("to_s", ("F",))?;
    if let Ok(JsonValue::Number(n)) = serde_json::from_str::<JsonValue>(&decimal_text) {
        return Ok(JsonValue::Number(n));
    }
    Err(Error::new(
        ruby.exception_arg_error(),
        "Cannot convert BigDecimal NaN or Infinity to JSON",
    ))
}

/// Convert Ruby Integer to JSON Number
/// Handles Fixnum, Bignum, and arbitrary precision
#[inline]
fn convert_integer(ruby: &Ruby, value: Value) -> Result<JsonValue, Error> {
    // Try i64 first (handles most integers including negative fixnums)
    if let Ok(i) = i64::try_convert(value) {
        return Ok(JsonValue::Number(Number::from(i)));
    }

    // For bignums, try Integer methods
    if let Some(int) = Integer::from_value(value) {
        // Try u64 for large positive integers
        if let Ok(u) = int.to_u64() {
            return Ok(JsonValue::Number(Number::from(u)));
        }
        // Arbitrary precision via string parsing
        let s: String = int.funcall("to_s", ())?;
        if let Ok(JsonValue::Number(n)) = serde_json::from_str::<JsonValue>(&s) {
            return Ok(JsonValue::Number(n));
        }
    }

    Err(Error::new(
        ruby.exception_type_error(),
        "Cannot convert Integer to JSON",
    ))
}

#[inline]
fn hash_key_to_string(ruby: &Ruby, key: Value) -> Result<String, Error> {
    #[allow(unsafe_code)]
    let key_type = unsafe { RB_TYPE(key.as_raw()) };

    match key_type {
        ruby_value_type::RUBY_T_STRING => {
            if let Some(rstring) = RString::from_value(key) {
                // SAFETY: rstring is valid
                #[allow(unsafe_code)]
                let bytes = unsafe { rstring.as_slice() };
                return std::str::from_utf8(bytes)
                    .map(std::borrow::ToOwned::to_owned)
                    .map_err(|_| {
                        Error::new(
                            ruby.exception_encoding_error(),
                            "Hash key is not valid UTF-8",
                        )
                    });
            }
        }
        ruby_value_type::RUBY_T_SYMBOL => {
            if let Some(sym) = Symbol::from_value(key) {
                return Ok(sym.name()?.to_string());
            }
        }
        _ => {}
    }

    Err(Error::new(
        ruby.exception_type_error(),
        "Hash keys must be strings or symbols",
    ))
}

#[inline]
pub fn map_to_ruby(ruby: &Ruby, map: &Map<String, JsonValue>) -> Result<Value, Error> {
    let rb_hash = ruby.hash_new_capa(map.len());
    for (k, v) in map {
        rb_hash.aset(k.as_str(), value_to_ruby(ruby, v)?)?;
    }
    Ok(rb_hash.as_value())
}

#[inline]
pub fn value_to_ruby(ruby: &Ruby, value: &JsonValue) -> Result<Value, Error> {
    match value {
        JsonValue::Null => Ok(ruby.qnil().as_value()),
        JsonValue::Bool(b) => Ok(ruby.into_value(*b)),
        JsonValue::Number(n) => number_to_ruby(ruby, n),
        JsonValue::String(s) => Ok(ruby.into_value(s.as_str())),
        JsonValue::Array(arr) => {
            let rb_arr = ruby.ary_new_capa(arr.len());
            for item in arr {
                rb_arr.push(value_to_ruby(ruby, item)?)?;
            }
            Ok(rb_arr.as_value())
        }
        JsonValue::Object(obj) => {
            let rb_hash = ruby.hash_new_capa(obj.len());
            for (k, v) in obj {
                rb_hash.aset(k.as_str(), value_to_ruby(ruby, v)?)?;
            }
            Ok(rb_hash.as_value())
        }
    }
}

#[inline]
fn number_to_ruby(ruby: &Ruby, number: &Number) -> Result<Value, Error> {
    if let Some(i) = number.as_i64() {
        return Ok(ruby.into_value(i));
    }
    if let Some(u) = number.as_u64() {
        return Ok(ruby.integer_from_u64(u).as_value());
    }
    number_string_to_ruby(ruby, &number.to_string())
}

#[inline]
fn number_string_to_ruby(ruby: &Ruby, number: &str) -> Result<Value, Error> {
    if !number.contains(['.', 'e', 'E']) {
        return ruby.module_kernel().funcall("Integer", (number,));
    }

    if let Ok(f) = number.parse::<f64>() {
        if f.is_finite()
            && Number::from_f64(f).is_some_and(|roundtrip| roundtrip.to_string() == number)
        {
            return Ok(ruby.into_value(f));
        }
    }

    let _ = ruby.get_inner(&BIG_DECIMAL_CLASS);
    ruby.module_kernel().funcall("BigDecimal", (number,))
}

/// Token used by serde_json with the `arbitrary_precision` feature.
const SERDE_JSON_NUMBER_TOKEN: &str = "$serde_json::private::Number";

#[derive(Debug)]
struct RubySerError(String);

impl fmt::Display for RubySerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RubySerError {}

impl serde::ser::Error for RubySerError {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        RubySerError(msg.to_string())
    }
}

/// A [`serde::Serializer`] that directly produces Ruby [`Value`] objects.
#[derive(Clone, Copy)]
struct RubySerializer<'a> {
    ruby: &'a Ruby,
}

impl<'a> RubySerializer<'a> {
    fn new(ruby: &'a Ruby) -> Self {
        RubySerializer { ruby }
    }

    /// Parse a raw number string into a Ruby Integer, Float, or BigDecimal.
    fn parse_number(&self, s: &str) -> Result<Value, RubySerError> {
        number_string_to_ruby(self.ruby, s)
            .map_err(|e| RubySerError(format!("number conversion failed: {e}")))
    }
}

impl<'a> serde::Serializer for RubySerializer<'a> {
    type Ok = Value;
    type Error = RubySerError;

    type SerializeSeq = RubySeqSerializer<'a>;
    type SerializeTuple = RubySeqSerializer<'a>;
    type SerializeTupleStruct = RubySeqSerializer<'a>;
    type SerializeTupleVariant = RubySeqSerializer<'a>;
    type SerializeMap = RubyMapSerializer<'a>;
    type SerializeStruct = RubyStructSerializer<'a>;
    type SerializeStructVariant = RubyStructSerializer<'a>;

    #[inline]
    fn serialize_bool(self, v: bool) -> Result<Value, RubySerError> {
        Ok(self.ruby.into_value(v))
    }

    #[inline]
    fn serialize_i8(self, v: i8) -> Result<Value, RubySerError> {
        self.serialize_i64(i64::from(v))
    }

    #[inline]
    fn serialize_i16(self, v: i16) -> Result<Value, RubySerError> {
        self.serialize_i64(i64::from(v))
    }

    #[inline]
    fn serialize_i32(self, v: i32) -> Result<Value, RubySerError> {
        self.serialize_i64(i64::from(v))
    }

    #[inline]
    fn serialize_i64(self, v: i64) -> Result<Value, RubySerError> {
        Ok(self.ruby.into_value(v))
    }

    #[inline]
    fn serialize_u8(self, v: u8) -> Result<Value, RubySerError> {
        self.serialize_u64(u64::from(v))
    }

    #[inline]
    fn serialize_u16(self, v: u16) -> Result<Value, RubySerError> {
        self.serialize_u64(u64::from(v))
    }

    #[inline]
    fn serialize_u32(self, v: u32) -> Result<Value, RubySerError> {
        self.serialize_u64(u64::from(v))
    }

    #[inline]
    fn serialize_u64(self, v: u64) -> Result<Value, RubySerError> {
        Ok(self.ruby.integer_from_u64(v).as_value())
    }

    #[inline]
    fn serialize_f32(self, v: f32) -> Result<Value, RubySerError> {
        self.serialize_f64(f64::from(v))
    }

    #[inline]
    fn serialize_f64(self, v: f64) -> Result<Value, RubySerError> {
        Ok(self.ruby.into_value(v))
    }

    #[inline]
    fn serialize_char(self, v: char) -> Result<Value, RubySerError> {
        let mut buf = [0u8; 4];
        Ok(self.ruby.into_value(v.encode_utf8(&mut buf) as &str))
    }

    #[inline]
    fn serialize_str(self, v: &str) -> Result<Value, RubySerError> {
        Ok(self.ruby.into_value(v))
    }

    fn serialize_bytes(self, v: &[u8]) -> Result<Value, RubySerError> {
        Ok(self.ruby.str_from_slice(v).as_value())
    }

    #[inline]
    fn serialize_none(self) -> Result<Value, RubySerError> {
        Ok(self.ruby.qnil().as_value())
    }

    #[inline]
    fn serialize_some<T: ?Sized + serde::Serialize>(
        self,
        value: &T,
    ) -> Result<Value, RubySerError> {
        value.serialize(self)
    }

    #[inline]
    fn serialize_unit(self) -> Result<Value, RubySerError> {
        Ok(self.ruby.qnil().as_value())
    }

    #[inline]
    fn serialize_unit_struct(self, _name: &'static str) -> Result<Value, RubySerError> {
        Ok(self.ruby.qnil().as_value())
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<Value, RubySerError> {
        Ok(self.ruby.into_value(variant))
    }

    fn serialize_newtype_struct<T: ?Sized + serde::Serialize>(
        self,
        name: &'static str,
        value: &T,
    ) -> Result<Value, RubySerError> {
        if name == SERDE_JSON_NUMBER_TOKEN {
            // inner serializes as a raw number string.
            // Serialize to Ruby String first, then parse as number.
            let rb_str = value.serialize(self)?;
            if let Some(rstring) = RString::from_value(rb_str) {
                let number = {
                    // Copy bytes into an owned String before calling Ruby:
                    // `parse_number` may invoke `Kernel.Integer` / `BigDecimal`,
                    // so keeping an `as_slice` borrow alive would be unsound.
                    // SAFETY: `rstring` is valid and we're in Ruby VM context.
                    #[allow(unsafe_code)]
                    let bytes = unsafe { rstring.as_slice() };
                    std::str::from_utf8(bytes)
                        .map(std::borrow::ToOwned::to_owned)
                        .map_err(|_| {
                            serde::ser::Error::custom("invalid arbitrary precision number")
                        })?
                };
                return self.parse_number(&number);
            }
            return Err(serde::ser::Error::custom(
                "invalid arbitrary precision number",
            ));
        }
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: ?Sized + serde::Serialize>(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        value: &T,
    ) -> Result<Value, RubySerError> {
        value.serialize(self)
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<RubySeqSerializer<'a>, RubySerError> {
        let arr = match len {
            Some(n) => self.ruby.ary_new_capa(n),
            None => self.ruby.ary_new(),
        };
        Ok(RubySeqSerializer {
            ruby: self.ruby,
            arr,
        })
    }

    fn serialize_tuple(self, len: usize) -> Result<RubySeqSerializer<'a>, RubySerError> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<RubySeqSerializer<'a>, RubySerError> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        len: usize,
    ) -> Result<RubySeqSerializer<'a>, RubySerError> {
        self.serialize_seq(Some(len))
    }

    fn serialize_map(self, len: Option<usize>) -> Result<RubyMapSerializer<'a>, RubySerError> {
        let hash = match len {
            Some(n) => self.ruby.hash_new_capa(n),
            None => self.ruby.hash_new(),
        };
        Ok(RubyMapSerializer {
            ruby: self.ruby,
            hash,
            next_key: None,
        })
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<RubyStructSerializer<'a>, RubySerError> {
        Ok(RubyStructSerializer {
            ruby: self.ruby,
            hash: self.ruby.hash_new_capa(len),
        })
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        len: usize,
    ) -> Result<RubyStructSerializer<'a>, RubySerError> {
        Ok(RubyStructSerializer {
            ruby: self.ruby,
            hash: self.ruby.hash_new_capa(len),
        })
    }
}

/// Sequence serializer producing Ruby Arrays.
struct RubySeqSerializer<'a> {
    ruby: &'a Ruby,
    arr: RArray,
}

impl serde::ser::SerializeSeq for RubySeqSerializer<'_> {
    type Ok = Value;
    type Error = RubySerError;

    fn serialize_element<T: ?Sized + serde::Serialize>(
        &mut self,
        value: &T,
    ) -> Result<(), RubySerError> {
        let v = value.serialize(RubySerializer::new(self.ruby))?;
        self.arr.push(v).map_err(serde::ser::Error::custom)
    }

    fn end(self) -> Result<Value, RubySerError> {
        Ok(self.arr.as_value())
    }
}

impl serde::ser::SerializeTuple for RubySeqSerializer<'_> {
    type Ok = Value;
    type Error = RubySerError;

    fn serialize_element<T: ?Sized + serde::Serialize>(
        &mut self,
        value: &T,
    ) -> Result<(), RubySerError> {
        serde::ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Value, RubySerError> {
        serde::ser::SerializeSeq::end(self)
    }
}

impl serde::ser::SerializeTupleStruct for RubySeqSerializer<'_> {
    type Ok = Value;
    type Error = RubySerError;

    fn serialize_field<T: ?Sized + serde::Serialize>(
        &mut self,
        value: &T,
    ) -> Result<(), RubySerError> {
        serde::ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Value, RubySerError> {
        serde::ser::SerializeSeq::end(self)
    }
}

impl serde::ser::SerializeTupleVariant for RubySeqSerializer<'_> {
    type Ok = Value;
    type Error = RubySerError;

    fn serialize_field<T: ?Sized + serde::Serialize>(
        &mut self,
        value: &T,
    ) -> Result<(), RubySerError> {
        serde::ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Value, RubySerError> {
        serde::ser::SerializeSeq::end(self)
    }
}

/// Map serializer producing Ruby Hashes.
struct RubyMapSerializer<'a> {
    ruby: &'a Ruby,
    hash: RHash,
    next_key: Option<Value>,
}

impl serde::ser::SerializeMap for RubyMapSerializer<'_> {
    type Ok = Value;
    type Error = RubySerError;

    fn serialize_key<T: ?Sized + serde::Serialize>(&mut self, key: &T) -> Result<(), RubySerError> {
        self.next_key = Some(key.serialize(RubySerializer::new(self.ruby))?);
        Ok(())
    }

    fn serialize_value<T: ?Sized + serde::Serialize>(
        &mut self,
        value: &T,
    ) -> Result<(), RubySerError> {
        let key = self
            .next_key
            .take()
            .expect("serialize_value called without serialize_key");
        let val = value.serialize(RubySerializer::new(self.ruby))?;
        self.hash.aset(key, val).map_err(serde::ser::Error::custom)
    }

    fn end(self) -> Result<Value, RubySerError> {
        Ok(self.hash.as_value())
    }
}

/// Struct serializer producing Ruby Hashes with symbol keys.
struct RubyStructSerializer<'a> {
    ruby: &'a Ruby,
    hash: RHash,
}

impl serde::ser::SerializeStruct for RubyStructSerializer<'_> {
    type Ok = Value;
    type Error = RubySerError;

    fn serialize_field<T: ?Sized + serde::Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), RubySerError> {
        let val = value.serialize(RubySerializer::new(self.ruby))?;
        let sym = self.ruby.sym_new(key);
        self.hash.aset(sym, val).map_err(serde::ser::Error::custom)
    }

    fn end(self) -> Result<Value, RubySerError> {
        Ok(self.hash.as_value())
    }
}

impl serde::ser::SerializeStructVariant for RubyStructSerializer<'_> {
    type Ok = Value;
    type Error = RubySerError;

    fn serialize_field<T: ?Sized + serde::Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), RubySerError> {
        serde::ser::SerializeStruct::serialize_field(self, key, value)
    }

    fn end(self) -> Result<Value, RubySerError> {
        serde::ser::SerializeStruct::end(self)
    }
}

/// Serialize any [`serde::Serialize`] type directly to a Ruby [`Value`].
pub fn serialize_to_ruby<T: serde::Serialize>(ruby: &Ruby, value: &T) -> Result<Value, Error> {
    value
        .serialize(RubySerializer::new(ruby))
        .map_err(|err| Error::new(ruby.exception_runtime_error(), err.to_string()))
}
