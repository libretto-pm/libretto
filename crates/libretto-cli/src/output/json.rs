//! JSON output support for machine-readable CLI output.
//!
//! This module provides structured JSON output for errors and results,
//! suitable for automation, CI/CD pipelines, and tooling integration.

use libretto_core::error::Error as CoreError;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};

/// Global JSON output mode
static JSON_OUTPUT: AtomicBool = AtomicBool::new(false);

/// Enable JSON output mode.
pub fn enable() {
    JSON_OUTPUT.store(true, Ordering::Relaxed);
}

/// Disable JSON output mode.
pub fn disable() {
    JSON_OUTPUT.store(false, Ordering::Relaxed);
}

/// Check if JSON output is enabled.
pub fn is_enabled() -> bool {
    JSON_OUTPUT.load(Ordering::Relaxed)
}

/// JSON-serializable error structure.
#[derive(Debug, Serialize)]
pub struct JsonError {
    /// Error code (e.g., "E0101")
    pub code: String,
    /// Error code title
    pub title: String,
    /// Detailed error message
    pub message: String,
    /// Suggestions for fixing the error
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub suggestions: Vec<String>,
    /// Additional context about the error
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<ErrorContext>,
    /// Documentation URL
    pub docs_url: String,
}

/// Additional context for errors.
#[derive(Debug, Serialize)]
pub struct ErrorContext {
    /// Related package name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    /// Related file path
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// Related line number
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    /// Related URL
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// JSON-serializable result structure.
#[derive(Debug, Serialize)]
pub struct JsonResult<T> {
    /// Whether the operation succeeded
    pub success: bool,
    /// The result data (if success)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    /// Error information (if failure)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonError>,
}

impl JsonError {
    /// Create a `JsonError` from a core error.
    #[must_use]
    pub fn from_core_error(err: &CoreError) -> Self {
        let code = err.code();
        let suggestions = err.suggestions().to_vec();

        let context = extract_context(err);

        Self {
            code: code.as_str().to_string(),
            title: code.title().to_string(),
            message: err.to_string(),
            suggestions,
            context,
            docs_url: format!("https://libretto.dev/errors/{}", code.as_str()),
        }
    }

    /// Create a `JsonError` from an anyhow error.
    #[must_use]
    pub fn from_anyhow(err: &anyhow::Error) -> Self {
        // Try to downcast to CoreError for richer information
        if let Some(core_err) = err.downcast_ref::<CoreError>() {
            return Self::from_core_error(core_err);
        }

        // Generic error
        Self {
            code: "E0000".to_string(),
            title: "Unknown error".to_string(),
            message: err.to_string(),
            suggestions: vec![],
            context: None,
            docs_url: "https://libretto.dev/errors".to_string(),
        }
    }

    /// Print this error as JSON to stderr.
    pub fn print(&self) {
        if let Ok(json) = sonic_rs::to_string_pretty(self) {
            eprintln!("{json}");
        }
    }
}

impl<T: Serialize> JsonResult<T> {
    /// Create a successful result.
    #[must_use]
    pub fn success(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    /// Create a failed result from an error.
    #[must_use]
    pub fn failure(err: &anyhow::Error) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(JsonError::from_anyhow(err)),
        }
    }

    /// Print this result as JSON to stdout (or stderr for errors).
    pub fn print(&self) {
        if let Ok(json) = sonic_rs::to_string_pretty(self) {
            if self.success {
                println!("{json}");
            } else {
                eprintln!("{json}");
            }
        }
    }
}

/// Extract context from a core error.
fn extract_context(err: &CoreError) -> Option<ErrorContext> {
    match err {
        CoreError::PackageNotFound { name, .. } => Some(ErrorContext {
            package: Some(name.clone()),
            file: None,
            line: None,
            url: None,
        }),
        CoreError::VersionNotFound { name, .. } => Some(ErrorContext {
            package: Some(name.clone()),
            file: None,
            line: None,
            url: None,
        }),
        CoreError::InvalidManifest { path, line, .. } => Some(ErrorContext {
            package: None,
            file: path.as_ref().map(|p| p.display().to_string()),
            line: *line,
            url: None,
        }),
        CoreError::Network { url, .. } => Some(ErrorContext {
            package: None,
            file: None,
            line: None,
            url: url.clone(),
        }),
        CoreError::Io { path, .. } => Some(ErrorContext {
            package: None,
            file: Some(path.display().to_string()),
            line: None,
            url: None,
        }),
        CoreError::ChecksumMismatch { name, .. } => Some(ErrorContext {
            package: Some(name.clone()),
            file: None,
            line: None,
            url: None,
        }),
        CoreError::Script { script_name, .. } => Some(ErrorContext {
            package: None,
            file: Some(script_name.clone()),
            line: None,
            url: None,
        }),
        _ => None,
    }
}

/// Print an error in JSON format if enabled, otherwise human-readable.
pub fn print_error(err: &anyhow::Error) {
    if is_enabled() {
        JsonError::from_anyhow(err).print();
    } else {
        // Try to get rich error display
        if let Some(core_err) = err.downcast_ref::<CoreError>() {
            eprintln!("{}", core_err.display_with_suggestions());
        } else {
            super::error(&err.to_string());
        }
    }
}

/// Wrapper to conditionally output success or handle errors.
pub fn handle_result<T: Serialize>(result: Result<T, anyhow::Error>) -> Result<T, anyhow::Error> {
    match result {
        Ok(data) => {
            if is_enabled() {
                JsonResult::success(&data).print();
            }
            Ok(data)
        }
        Err(e) => {
            print_error(&e);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_error_from_core() {
        let err = CoreError::package_not_found("vendor/package");
        let json_err = JsonError::from_core_error(&err);

        assert_eq!(json_err.code, "E0101");
        assert_eq!(json_err.title, "Package not found");
        assert!(json_err.message.contains("vendor/package"));
        assert!(!json_err.suggestions.is_empty());
        assert!(json_err.docs_url.contains("E0101"));
    }

    #[test]
    fn test_json_result_success() {
        #[derive(Serialize)]
        struct TestData {
            count: usize,
        }

        let result: JsonResult<TestData> = JsonResult::success(TestData { count: 42 });
        assert!(result.success);
        assert!(result.data.is_some());
        assert!(result.error.is_none());
    }

    #[test]
    fn test_json_mode_toggle() {
        disable();
        assert!(!is_enabled());

        enable();
        assert!(is_enabled());

        disable();
        assert!(!is_enabled());
    }
}
