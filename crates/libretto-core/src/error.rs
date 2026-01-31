//! Error types for Libretto operations.
//!
//! Each error has:
//! - A unique error code (e.g., E0001) for easy reference and searching
//! - A clear error message explaining what went wrong
//! - Suggestions for how to fix the issue
//! - Optional context about the operation that failed

use std::fmt;
use std::path::PathBuf;
use thiserror::Error;

/// Error codes for Libretto errors.
///
/// These codes make it easy to search for solutions and reference specific errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    // Package errors (E01xx)
    /// Package not found in any repository
    E0101,
    /// Version constraint cannot be satisfied
    E0102,
    /// Package checksum mismatch
    E0103,
    /// Invalid package name format
    E0104,

    // Resolution errors (E02xx)
    /// Dependency resolution failed
    E0201,
    /// Circular dependency detected
    E0202,
    /// Conflicting version requirements
    E0203,

    // Network errors (E03xx)
    /// Network request failed
    E0301,
    /// Repository unreachable
    E0302,
    /// Authentication failed
    E0303,
    /// Rate limited by server
    E0304,
    /// SSL/TLS error
    E0305,

    // Manifest errors (E04xx)
    /// Invalid composer.json
    E0401,
    /// Missing required field
    E0402,
    /// Invalid JSON syntax
    E0403,
    /// Invalid version constraint format
    E0404,

    // IO errors (E05xx)
    /// File not found
    E0501,
    /// Permission denied
    E0502,
    /// Disk full
    E0503,
    /// Path too long
    E0504,
    /// File already exists
    E0505,

    // Cache errors (E06xx)
    /// Cache corrupted
    E0601,
    /// Cache directory not writable
    E0602,

    // Plugin errors (E07xx)
    /// Plugin not found
    E0701,
    /// Plugin execution failed
    E0702,
    /// Plugin compatibility issue
    E0703,

    // Archive errors (E08xx)
    /// Invalid archive format
    E0801,
    /// Archive extraction failed
    E0802,
    /// Archive corrupted
    E0803,

    // VCS errors (E09xx)
    /// Git operation failed
    E0901,
    /// Repository clone failed
    E0902,
    /// Branch/tag not found
    E0903,

    // Security errors (E10xx)
    /// Vulnerabilities found
    E1001,
    /// Integrity check failed
    E1002,
    /// Signature verification failed
    E1003,
    /// Untrusted source
    E1004,

    // Configuration errors (E11xx)
    /// Invalid configuration
    E1101,
    /// Missing configuration
    E1102,

    // Platform errors (E12xx)
    /// Unsupported platform
    E1201,
    /// PHP version mismatch
    E1202,
    /// Extension not available
    E1203,

    // Script errors (E13xx)
    /// Script execution failed
    E1301,
    /// Script timed out
    E1302,
    /// Script not found
    E1303,
}

impl ErrorCode {
    /// Get the string representation of the error code.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::E0101 => "E0101",
            Self::E0102 => "E0102",
            Self::E0103 => "E0103",
            Self::E0104 => "E0104",
            Self::E0201 => "E0201",
            Self::E0202 => "E0202",
            Self::E0203 => "E0203",
            Self::E0301 => "E0301",
            Self::E0302 => "E0302",
            Self::E0303 => "E0303",
            Self::E0304 => "E0304",
            Self::E0305 => "E0305",
            Self::E0401 => "E0401",
            Self::E0402 => "E0402",
            Self::E0403 => "E0403",
            Self::E0404 => "E0404",
            Self::E0501 => "E0501",
            Self::E0502 => "E0502",
            Self::E0503 => "E0503",
            Self::E0504 => "E0504",
            Self::E0505 => "E0505",
            Self::E0601 => "E0601",
            Self::E0602 => "E0602",
            Self::E0701 => "E0701",
            Self::E0702 => "E0702",
            Self::E0703 => "E0703",
            Self::E0801 => "E0801",
            Self::E0802 => "E0802",
            Self::E0803 => "E0803",
            Self::E0901 => "E0901",
            Self::E0902 => "E0902",
            Self::E0903 => "E0903",
            Self::E1001 => "E1001",
            Self::E1002 => "E1002",
            Self::E1003 => "E1003",
            Self::E1004 => "E1004",
            Self::E1101 => "E1101",
            Self::E1102 => "E1102",
            Self::E1201 => "E1201",
            Self::E1202 => "E1202",
            Self::E1203 => "E1203",
            Self::E1301 => "E1301",
            Self::E1302 => "E1302",
            Self::E1303 => "E1303",
        }
    }

    /// Get a brief title for this error code.
    #[must_use]
    pub const fn title(&self) -> &'static str {
        match self {
            Self::E0101 => "Package not found",
            Self::E0102 => "Version not satisfiable",
            Self::E0103 => "Checksum mismatch",
            Self::E0104 => "Invalid package name",
            Self::E0201 => "Resolution failed",
            Self::E0202 => "Circular dependency",
            Self::E0203 => "Conflicting versions",
            Self::E0301 => "Network error",
            Self::E0302 => "Repository unreachable",
            Self::E0303 => "Authentication failed",
            Self::E0304 => "Rate limited",
            Self::E0305 => "TLS error",
            Self::E0401 => "Invalid manifest",
            Self::E0402 => "Missing required field",
            Self::E0403 => "JSON syntax error",
            Self::E0404 => "Invalid version constraint",
            Self::E0501 => "File not found",
            Self::E0502 => "Permission denied",
            Self::E0503 => "Disk full",
            Self::E0504 => "Path too long",
            Self::E0505 => "File exists",
            Self::E0601 => "Cache corrupted",
            Self::E0602 => "Cache not writable",
            Self::E0701 => "Plugin not found",
            Self::E0702 => "Plugin execution failed",
            Self::E0703 => "Plugin incompatible",
            Self::E0801 => "Invalid archive",
            Self::E0802 => "Extraction failed",
            Self::E0803 => "Archive corrupted",
            Self::E0901 => "Git error",
            Self::E0902 => "Clone failed",
            Self::E0903 => "Ref not found",
            Self::E1001 => "Vulnerabilities found",
            Self::E1002 => "Integrity check failed",
            Self::E1003 => "Signature invalid",
            Self::E1004 => "Untrusted source",
            Self::E1101 => "Invalid configuration",
            Self::E1102 => "Missing configuration",
            Self::E1201 => "Unsupported platform",
            Self::E1202 => "PHP version mismatch",
            Self::E1203 => "Extension unavailable",
            Self::E1301 => "Script failed",
            Self::E1302 => "Script timeout",
            Self::E1303 => "Script not found",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Main error type for Libretto.
#[derive(Error, Debug)]
pub enum Error {
    /// Package not found.
    #[error("[{code}] package '{name}' not found")]
    PackageNotFound {
        /// Error code.
        #[source]
        code: ErrorCodeSource,
        /// Package name.
        name: String,
        /// Suggestions for fixing.
        suggestions: Vec<String>,
    },

    /// Version not satisfiable.
    #[error("[{code}] no version of '{name}' satisfies '{constraint}'")]
    VersionNotFound {
        /// Error code.
        #[source]
        code: ErrorCodeSource,
        /// Package name.
        name: String,
        /// Version constraint.
        constraint: String,
        /// Available versions (for suggestions).
        available_versions: Vec<String>,
        /// Suggestions for fixing.
        suggestions: Vec<String>,
    },

    /// Dependency resolution failed.
    #[error("[{code}] resolution failed: {message}")]
    Resolution {
        /// Error code.
        #[source]
        code: ErrorCodeSource,
        /// Error message.
        message: String,
        /// Conflicting packages.
        conflicting_packages: Vec<String>,
        /// Suggestions for fixing.
        suggestions: Vec<String>,
    },

    /// Circular dependency.
    #[error("[{code}] circular dependency detected: {cycle}")]
    CircularDependency {
        /// Error code.
        #[source]
        code: ErrorCodeSource,
        /// The dependency cycle.
        cycle: String,
        /// Packages involved.
        packages: Vec<String>,
        /// Suggestions for fixing.
        suggestions: Vec<String>,
    },

    /// Network error.
    #[error("[{code}] network error: {message}")]
    Network {
        /// Error code.
        #[source]
        code: ErrorCodeSource,
        /// Error message.
        message: String,
        /// URL that failed (if applicable).
        url: Option<String>,
        /// Suggestions for fixing.
        suggestions: Vec<String>,
    },

    /// Invalid manifest.
    #[error("[{code}] invalid manifest: {message}")]
    InvalidManifest {
        /// Error code.
        #[source]
        code: ErrorCodeSource,
        /// Error message.
        message: String,
        /// File path.
        path: Option<PathBuf>,
        /// Line number (if applicable).
        line: Option<usize>,
        /// Suggestions for fixing.
        suggestions: Vec<String>,
    },

    /// JSON error.
    #[error("[E0403] json error: {0}")]
    Json(#[from] sonic_rs::Error),

    /// IO error.
    #[error("[{code}] io error at {path}: {message}")]
    Io {
        /// Error code.
        #[source]
        code: ErrorCodeSource,
        /// File path.
        path: PathBuf,
        /// Error message.
        message: String,
        /// Suggestions for fixing.
        suggestions: Vec<String>,
    },

    /// Cache error.
    #[error("[{code}] cache error: {message}")]
    Cache {
        /// Error code.
        #[source]
        code: ErrorCodeSource,
        /// Error message.
        message: String,
        /// Suggestions for fixing.
        suggestions: Vec<String>,
    },

    /// Plugin error.
    #[error("[{code}] plugin error: {message}")]
    Plugin {
        /// Error code.
        #[source]
        code: ErrorCodeSource,
        /// Plugin name.
        plugin_name: Option<String>,
        /// Error message.
        message: String,
        /// Suggestions for fixing.
        suggestions: Vec<String>,
    },

    /// Archive error.
    #[error("[{code}] archive error: {message}")]
    Archive {
        /// Error code.
        #[source]
        code: ErrorCodeSource,
        /// Error message.
        message: String,
        /// Archive path.
        path: Option<PathBuf>,
        /// Suggestions for fixing.
        suggestions: Vec<String>,
    },

    /// VCS error.
    #[error("[{code}] vcs error: {message}")]
    Vcs {
        /// Error code.
        #[source]
        code: ErrorCodeSource,
        /// Error message.
        message: String,
        /// Repository URL.
        repository: Option<String>,
        /// Suggestions for fixing.
        suggestions: Vec<String>,
    },

    /// Checksum mismatch.
    #[error("[E0103] checksum mismatch for '{name}': expected {expected}, got {actual}")]
    ChecksumMismatch {
        /// Package name.
        name: String,
        /// Expected hash.
        expected: String,
        /// Actual hash.
        actual: String,
        /// Suggestions for fixing.
        suggestions: Vec<String>,
    },

    /// Security vulnerability.
    #[error("[E1001] security: {count} vulnerabilities found")]
    Security {
        /// Number of issues.
        count: usize,
        /// Affected packages.
        affected_packages: Vec<String>,
        /// Suggestions for fixing.
        suggestions: Vec<String>,
    },

    /// Configuration error.
    #[error("[{code}] config error: {message}")]
    Config {
        /// Error code.
        #[source]
        code: ErrorCodeSource,
        /// Error message.
        message: String,
        /// Configuration key.
        key: Option<String>,
        /// Suggestions for fixing.
        suggestions: Vec<String>,
    },

    /// Platform not supported.
    #[error("[{code}] unsupported platform: {message}")]
    UnsupportedPlatform {
        /// Error code.
        #[source]
        code: ErrorCodeSource,
        /// Error message.
        message: String,
        /// Required platform.
        required: Option<String>,
        /// Current platform.
        current: Option<String>,
        /// Suggestions for fixing.
        suggestions: Vec<String>,
    },

    /// Audit error.
    #[error("[E1001] audit error: {0}")]
    Audit(String),

    /// Integrity verification error.
    #[error("[E1002] integrity error: {message}")]
    Integrity {
        /// Error message.
        message: String,
        /// File or package that failed.
        target: Option<String>,
        /// Suggestions for fixing.
        suggestions: Vec<String>,
    },

    /// Signature verification error.
    #[error("[E1003] signature error: {message}")]
    Signature {
        /// Error message.
        message: String,
        /// What was being verified.
        target: Option<String>,
        /// Suggestions for fixing.
        suggestions: Vec<String>,
    },

    /// Script execution error.
    #[error("[{code}] script error: {message}")]
    Script {
        /// Error code.
        #[source]
        code: ErrorCodeSource,
        /// Script name.
        script_name: String,
        /// Error message.
        message: String,
        /// Exit code (if available).
        exit_code: Option<i32>,
        /// Suggestions for fixing.
        suggestions: Vec<String>,
    },
}

/// Wrapper to make `ErrorCode` usable as a source.
#[derive(Debug)]
pub struct ErrorCodeSource(pub ErrorCode);

impl fmt::Display for ErrorCodeSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.as_str())
    }
}

impl std::error::Error for ErrorCodeSource {}

impl Error {
    /// Get the error code for this error.
    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        match self {
            Self::PackageNotFound { code, .. } => code.0,
            Self::VersionNotFound { code, .. } => code.0,
            Self::Resolution { code, .. } => code.0,
            Self::CircularDependency { code, .. } => code.0,
            Self::Network { code, .. } => code.0,
            Self::InvalidManifest { code, .. } => code.0,
            Self::Json(_) => ErrorCode::E0403,
            Self::Io { code, .. } => code.0,
            Self::Cache { code, .. } => code.0,
            Self::Plugin { code, .. } => code.0,
            Self::Archive { code, .. } => code.0,
            Self::Vcs { code, .. } => code.0,
            Self::ChecksumMismatch { .. } => ErrorCode::E0103,
            Self::Security { .. } => ErrorCode::E1001,
            Self::Config { code, .. } => code.0,
            Self::UnsupportedPlatform { code, .. } => code.0,
            Self::Audit(_) => ErrorCode::E1001,
            Self::Integrity { .. } => ErrorCode::E1002,
            Self::Signature { .. } => ErrorCode::E1003,
            Self::Script { code, .. } => code.0,
        }
    }

    /// Get suggestions for fixing this error.
    #[must_use]
    pub fn suggestions(&self) -> &[String] {
        match self {
            Self::PackageNotFound { suggestions, .. }
            | Self::VersionNotFound { suggestions, .. }
            | Self::Resolution { suggestions, .. }
            | Self::CircularDependency { suggestions, .. }
            | Self::Network { suggestions, .. }
            | Self::InvalidManifest { suggestions, .. }
            | Self::Io { suggestions, .. }
            | Self::Cache { suggestions, .. }
            | Self::Plugin { suggestions, .. }
            | Self::Archive { suggestions, .. }
            | Self::Vcs { suggestions, .. }
            | Self::ChecksumMismatch { suggestions, .. }
            | Self::Security { suggestions, .. }
            | Self::Config { suggestions, .. }
            | Self::UnsupportedPlatform { suggestions, .. }
            | Self::Integrity { suggestions, .. }
            | Self::Signature { suggestions, .. }
            | Self::Script { suggestions, .. } => suggestions,
            Self::Json(_) | Self::Audit(_) => &[],
        }
    }

    /// Create an IO error with context.
    #[must_use]
    #[allow(clippy::needless_pass_by_value)]
    pub fn io(path: impl Into<PathBuf>, err: std::io::Error) -> Self {
        let path = path.into();
        let (code, suggestions) = match err.kind() {
            std::io::ErrorKind::NotFound => (
                ErrorCode::E0501,
                vec![
                    format!("Check if the path exists: {}", path.display()),
                    "Verify you're in the correct directory".to_string(),
                ],
            ),
            std::io::ErrorKind::PermissionDenied => (
                ErrorCode::E0502,
                vec![
                    format!("Check permissions on: {}", path.display()),
                    "Try running with appropriate permissions".to_string(),
                    "On Unix, check file ownership with 'ls -la'".to_string(),
                ],
            ),
            std::io::ErrorKind::AlreadyExists => (
                ErrorCode::E0505,
                vec![
                    format!("File already exists: {}", path.display()),
                    "Use --force to overwrite if intended".to_string(),
                ],
            ),
            _ => (
                ErrorCode::E0501,
                vec![format!("Check the file: {}", path.display())],
            ),
        };
        Self::Io {
            code: ErrorCodeSource(code),
            path,
            message: err.to_string(),
            suggestions,
        }
    }

    /// Create a package not found error with suggestions.
    #[must_use]
    pub fn package_not_found(name: impl Into<String>) -> Self {
        let name = name.into();
        Self::PackageNotFound {
            code: ErrorCodeSource(ErrorCode::E0101),
            suggestions: vec![
                format!("Search for the package: libretto search {name}"),
                "Check the package name for typos".to_string(),
                "Verify the package exists on packagist.org".to_string(),
                "For private packages, check your repository configuration".to_string(),
            ],
            name,
        }
    }

    /// Create a version not found error with suggestions.
    #[must_use]
    pub fn version_not_found(
        name: impl Into<String>,
        constraint: impl Into<String>,
        available: Vec<String>,
    ) -> Self {
        let name = name.into();
        let constraint = constraint.into();
        let mut suggestions = vec![
            format!("Try a less restrictive constraint (current: {constraint})"),
            format!("Available versions: {}", available.join(", ")),
        ];
        if let Some(latest) = available.first() {
            suggestions.insert(0, format!("Latest available version: {latest}"));
        }
        Self::VersionNotFound {
            code: ErrorCodeSource(ErrorCode::E0102),
            name,
            constraint,
            available_versions: available,
            suggestions,
        }
    }

    /// Create a resolution error with context.
    #[must_use]
    pub fn resolution(message: impl Into<String>, conflicting: Vec<String>) -> Self {
        let message = message.into();
        let mut suggestions = vec![
            "Run 'libretto why <package>' to understand version constraints".to_string(),
            "Try updating individual packages to resolve conflicts".to_string(),
        ];
        if !conflicting.is_empty() {
            suggestions.insert(
                0,
                format!("Conflicting packages: {}", conflicting.join(", ")),
            );
        }
        Self::Resolution {
            code: ErrorCodeSource(ErrorCode::E0201),
            message,
            conflicting_packages: conflicting,
            suggestions,
        }
    }

    /// Create a network error with suggestions.
    #[must_use]
    pub fn network(message: impl Into<String>, url: Option<String>) -> Self {
        let message = message.into();
        let mut suggestions = vec![
            "Check your internet connection".to_string(),
            "Verify proxy settings if behind a corporate firewall".to_string(),
        ];
        if let Some(ref u) = url {
            suggestions.push(format!("Try accessing {u} in a browser"));
        }
        if message.contains("timeout") {
            suggestions.push("Try increasing timeout with --timeout flag".to_string());
        }
        if message.contains("certificate") || message.contains("SSL") || message.contains("TLS") {
            suggestions.push("Check system certificates are up to date".to_string());
            suggestions.push("Try --insecure flag for testing (not recommended)".to_string());
        }
        Self::Network {
            code: ErrorCodeSource(if message.contains("auth") {
                ErrorCode::E0303
            } else if message.contains("rate") {
                ErrorCode::E0304
            } else if message.contains("SSL") || message.contains("TLS") {
                ErrorCode::E0305
            } else {
                ErrorCode::E0301
            }),
            message,
            url,
            suggestions,
        }
    }

    /// Create an invalid manifest error.
    #[must_use]
    pub fn invalid_manifest(
        message: impl Into<String>,
        path: Option<PathBuf>,
        line: Option<usize>,
    ) -> Self {
        let message = message.into();
        let mut suggestions = vec![
            "Validate your composer.json: libretto validate".to_string(),
            "Check JSON syntax with a JSON validator".to_string(),
        ];
        if let Some(ref p) = path {
            suggestions.push(format!("Edit the file: {}", p.display()));
        }
        if let Some(l) = line {
            suggestions.push(format!("Error is near line {l}"));
        }
        Self::InvalidManifest {
            code: ErrorCodeSource(ErrorCode::E0401),
            message,
            path,
            line,
            suggestions,
        }
    }

    /// Create a cache error.
    #[must_use]
    pub fn cache(message: impl Into<String>) -> Self {
        let message = message.into();
        Self::Cache {
            code: ErrorCodeSource(if message.contains("permission") {
                ErrorCode::E0602
            } else {
                ErrorCode::E0601
            }),
            message,
            suggestions: vec![
                "Try clearing the cache: libretto cache:clear".to_string(),
                "Check cache directory permissions".to_string(),
                "Verify disk space is available".to_string(),
            ],
        }
    }

    /// Create a checksum mismatch error.
    #[must_use]
    pub fn checksum_mismatch(
        name: impl Into<String>,
        expected: impl Into<String>,
        actual: impl Into<String>,
    ) -> Self {
        Self::ChecksumMismatch {
            name: name.into(),
            expected: expected.into(),
            actual: actual.into(),
            suggestions: vec![
                "The downloaded file may be corrupted. Try again.".to_string(),
                "Clear cache and retry: libretto cache:clear && libretto install".to_string(),
                "This could indicate a MITM attack - verify your network is secure".to_string(),
                "Report this issue if it persists".to_string(),
            ],
        }
    }

    /// Create a script error.
    #[must_use]
    pub fn script(
        script_name: impl Into<String>,
        message: impl Into<String>,
        exit_code: Option<i32>,
    ) -> Self {
        let script_name = script_name.into();
        let message = message.into();
        let code = if message.contains("timeout") {
            ErrorCode::E1302
        } else if message.contains("not found") {
            ErrorCode::E1303
        } else {
            ErrorCode::E1301
        };
        let mut suggestions = vec![
            format!("Check the script definition for '{script_name}' in composer.json"),
            "Ensure all required binaries are in PATH".to_string(),
        ];
        if let Some(exit) = exit_code {
            suggestions.push(format!("Script exited with code {exit}"));
        }
        if code == ErrorCode::E1302 {
            suggestions.push("Increase timeout with --script-timeout flag".to_string());
        }
        Self::Script {
            code: ErrorCodeSource(code),
            script_name,
            message,
            exit_code,
            suggestions,
        }
    }

    /// Create a security error with affected packages.
    #[must_use]
    pub fn security(count: usize, affected: Vec<String>) -> Self {
        let mut suggestions = vec![
            "Run 'libretto audit' for full vulnerability report".to_string(),
            "Update affected packages to patched versions".to_string(),
        ];
        if !affected.is_empty() {
            suggestions.push(format!("Affected: {}", affected.join(", ")));
        }
        Self::Security {
            count,
            affected_packages: affected,
            suggestions,
        }
    }

    /// Format the error with suggestions for display.
    #[must_use]
    pub fn display_with_suggestions(&self) -> String {
        let mut output = format!("{self}");
        let suggestions = self.suggestions();
        if !suggestions.is_empty() {
            output.push_str("\n\nSuggestions:");
            for suggestion in suggestions {
                output.push_str(&format!("\n  • {suggestion}"));
            }
        }
        output.push_str(&format!(
            "\n\nFor more info, see: https://libretto.dev/errors/{}",
            self.code()
        ));
        output
    }
}

/// Result type for Libretto operations.
pub type Result<T> = std::result::Result<T, Error>;

// Backward compatibility: simple constructors that don't require full context
impl Error {
    /// Create a simple resolution error (backward compatible).
    #[must_use]
    pub fn resolution_simple(message: impl Into<String>) -> Self {
        Self::resolution(message, vec![])
    }

    /// Create a simple network error (backward compatible).
    #[must_use]
    pub fn network_simple(message: impl Into<String>) -> Self {
        Self::network(message, None)
    }

    /// Create a simple cache error (backward compatible).
    #[must_use]
    pub fn cache_simple(message: impl Into<String>) -> Self {
        Self::cache(message)
    }

    /// Create a simple manifest error (backward compatible).
    #[must_use]
    pub fn manifest_simple(message: impl Into<String>) -> Self {
        Self::invalid_manifest(message, None, None)
    }

    /// Create a simple plugin error.
    #[must_use]
    pub fn plugin(message: impl Into<String>) -> Self {
        let message = message.into();
        Self::Plugin {
            code: ErrorCodeSource(ErrorCode::E0702),
            plugin_name: None,
            message,
            suggestions: vec![
                "Check plugin configuration in composer.json".to_string(),
                "Note: Composer plugins written in PHP have limited support".to_string(),
            ],
        }
    }

    /// Create a simple archive error.
    #[must_use]
    pub fn archive(message: impl Into<String>) -> Self {
        let message = message.into();
        Self::Archive {
            code: ErrorCodeSource(ErrorCode::E0802),
            message,
            path: None,
            suggestions: vec![
                "The archive may be corrupted. Try downloading again.".to_string(),
                "Clear cache and retry: libretto cache:clear".to_string(),
            ],
        }
    }

    /// Create a simple VCS error.
    #[must_use]
    pub fn vcs(message: impl Into<String>) -> Self {
        let message = message.into();
        Self::Vcs {
            code: ErrorCodeSource(ErrorCode::E0901),
            message,
            repository: None,
            suggestions: vec![
                "Check that git is installed and in PATH".to_string(),
                "Verify repository URL and credentials".to_string(),
            ],
        }
    }

    /// Create a simple config error.
    #[must_use]
    pub fn config(message: impl Into<String>) -> Self {
        let message = message.into();
        Self::Config {
            code: ErrorCodeSource(ErrorCode::E1101),
            message,
            key: None,
            suggestions: vec!["Check your configuration file for errors".to_string()],
        }
    }

    /// Create a simple platform error.
    #[must_use]
    pub fn unsupported_platform(message: impl Into<String>) -> Self {
        let message = message.into();
        Self::UnsupportedPlatform {
            code: ErrorCodeSource(ErrorCode::E1201),
            message,
            required: None,
            current: None,
            suggestions: vec![
                "Check platform requirements in composer.json".to_string(),
                "Use --ignore-platform-reqs to bypass (use with caution)".to_string(),
            ],
        }
    }

    /// Create a simple circular dependency error.
    #[must_use]
    pub fn circular_dependency(cycle: impl Into<String>) -> Self {
        let cycle = cycle.into();
        Self::CircularDependency {
            code: ErrorCodeSource(ErrorCode::E0202),
            packages: vec![],
            cycle,
            suggestions: vec![
                "Review the dependency chain for the listed packages".to_string(),
                "Consider refactoring to break the circular dependency".to_string(),
            ],
        }
    }

    /// Create an integrity error.
    #[must_use]
    pub fn integrity(message: impl Into<String>) -> Self {
        let message = message.into();
        Self::Integrity {
            message,
            target: None,
            suggestions: vec![
                "The file may have been tampered with or corrupted".to_string(),
                "Try downloading again: libretto cache:clear && libretto install".to_string(),
            ],
        }
    }

    /// Create a signature error.
    #[must_use]
    pub fn signature(message: impl Into<String>) -> Self {
        let message = message.into();
        Self::Signature {
            message,
            target: None,
            suggestions: vec![
                "Signature verification failed - the package may be compromised".to_string(),
                "Verify the package source is trusted".to_string(),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_codes() {
        assert_eq!(ErrorCode::E0101.as_str(), "E0101");
        assert_eq!(ErrorCode::E0101.title(), "Package not found");
    }

    #[test]
    fn test_package_not_found_error() {
        let err = Error::package_not_found("vendor/package");
        assert_eq!(err.code(), ErrorCode::E0101);
        assert!(!err.suggestions().is_empty());
        assert!(err.to_string().contains("[E0101]"));
    }

    #[test]
    fn test_version_not_found_error() {
        let err = Error::version_not_found(
            "vendor/package",
            "^2.0",
            vec!["1.0.0".to_string(), "1.5.0".to_string()],
        );
        assert_eq!(err.code(), ErrorCode::E0102);
        assert!(err.suggestions().len() >= 2);
    }

    #[test]
    fn test_network_error_code_detection() {
        let auth_err = Error::network("authentication failed", None);
        assert_eq!(auth_err.code(), ErrorCode::E0303);

        let tls_err = Error::network("SSL certificate error", None);
        assert_eq!(tls_err.code(), ErrorCode::E0305);

        let generic_err = Error::network("connection refused", None);
        assert_eq!(generic_err.code(), ErrorCode::E0301);
    }

    #[test]
    fn test_display_with_suggestions() {
        let err = Error::package_not_found("test/package");
        let display = err.display_with_suggestions();
        assert!(display.contains("Suggestions:"));
        assert!(display.contains("https://libretto.dev/errors/E0101"));
    }
}
