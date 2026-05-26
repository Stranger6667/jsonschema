//! Public format validation functions.
//!
//! These are the same validators used internally by the `format` keyword.
//! They can be passed to [`ValidationOptions::with_format`](crate::ValidationOptions::with_format)
//! to register formats that are normally gated behind a newer draft.

pub use crate::keywords::format::{
    is_valid_date, is_valid_datetime, is_valid_duration, is_valid_hostname,
    is_valid_hostname_rfc1034, is_valid_idn_hostname, is_valid_ipv4, is_valid_ipv6, is_valid_iri,
    is_valid_iri_reference, is_valid_json_pointer, is_valid_relative_json_pointer, is_valid_time,
    is_valid_uri, is_valid_uri_reference, is_valid_uri_template, is_valid_uuid,
};
