// Each integration-test file is its own crate, so a helper only one of them uses looks dead here.
#![allow(dead_code)]

use std::fmt::Write as _;

pub(crate) use jsonschema_value::jsonb_encode::*;

pub(crate) fn to_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            write!(out, "{byte:02x}").expect("write to String never fails");
            out
        })
}
