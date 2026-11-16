//! Diagnostics shared by the schema checks, the compiler and the validator.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

/// A problem found in a pack (or, for the validator, in a design), with where it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    /// A stable code (`schema`, `duplicate_pin`, `unknown_unit`, ...); see docs/HCP.md.
    pub code: String,
    pub severity: Severity,
    /// JSON path into the document, `$.rules[3].params`.
    pub path: String,
    /// The id of the rule, block, region, device or source the path is in, when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    pub message: String,
}

impl Diagnostic {
    pub fn error(code: &str, path: impl Into<String>, object: Option<String>, message: impl Into<String>) -> Self {
        Diagnostic { code: code.into(), severity: Severity::Error, path: path.into(), object, message: message.into() }
    }

    pub fn warning(code: &str, path: impl Into<String>, object: Option<String>, message: impl Into<String>) -> Self {
        Diagnostic { code: code.into(), severity: Severity::Warning, path: path.into(), object, message: message.into() }
    }
}
