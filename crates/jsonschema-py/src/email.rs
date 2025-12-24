use pyo3::prelude::*;

/// EmailOptions(require_tld=False, allow_domain_literal=True, allow_display_text=True, minimum_sub_domains=None)
///
/// Configuration for email format validation.
///
/// This allows customization of email format validation behavior beyond the default
/// JSON Schema spec requirements.
///
///     >>> from jsonschema_rs import validator_for, EmailOptions
///     >>> validator = validator_for(
///     ...     {"format": "email", "type": "string"},
///     ...     validate_formats=True,
///     ...     email_options=EmailOptions(require_tld=True)
///     ... )
///     >>> validator.is_valid("user@localhost")
///     False
///     >>> validator.is_valid("user@example.com")
///     True
///
/// Parameters:
///     require_tld: Require a top-level domain (e.g., reject "user@localhost")
///     allow_domain_literal: Allow IP address literals like "user@[127.0.0.1]"
///     allow_display_text: Allow display names like "Name <user@example.com>"
///     minimum_sub_domains: Minimum number of domain segments required
#[pyclass(module = "jsonschema_rs")]
pub(crate) struct EmailOptions {
    pub(crate) require_tld: bool,
    pub(crate) allow_domain_literal: bool,
    pub(crate) allow_display_text: bool,
    pub(crate) minimum_sub_domains: Option<usize>,
}

#[pymethods]
impl EmailOptions {
    #[new]
    #[pyo3(signature = (require_tld=false, allow_domain_literal=true, allow_display_text=true, minimum_sub_domains=None))]
    fn new(
        require_tld: bool,
        allow_domain_literal: bool,
        allow_display_text: bool,
        minimum_sub_domains: Option<usize>,
    ) -> Self {
        Self {
            require_tld,
            allow_domain_literal,
            allow_display_text,
            minimum_sub_domains,
        }
    }
}
