//! Security auditing and integrity verification for Libretto.
//!
//! This crate provides comprehensive security features:
//! - Package integrity verification (SHA-256, SHA-1, BLAKE3)
//! - Signature verification (GPG/PGP, Ed25519)
//! - Security advisory checking
//! - Vulnerability analysis
//! - Platform requirements validation
//! - Secure credential management
//! - Audit logging
//!
//! # Performance
//!
//! Designed for ultra-high performance:
//! - SIMD-accelerated hashing (BLAKE3, SHA-256)
//! - Concurrent advisory fetching
//! - Cached vulnerability database
//! - Constant-time comparisons for security

#![warn(clippy::all)]
#![allow(clippy::module_name_repetitions)]

mod advisory;
mod audit_log;
mod credentials;
mod integrity;
mod permissions;
mod platform;
mod secure;
mod signature;

use chrono::{DateTime, Utc};
use libretto_core::{Error as CoreError, PackageId, Result as CoreResult, Version};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;
use tracing::{info, warn};
use url::Url;

// Re-exports
pub use advisory::{AdvisoryDatabase, AdvisoryError};
pub use audit_log::{AuditEntry, AuditLogError, AuditLogger, Operation};
pub use credentials::{Credential, CredentialError, CredentialManager, CredentialType};
pub use integrity::{
    Hash, HashAlgorithm, IntegrityError, IntegrityVerifier, hash_file, hash_file_all,
    verify_blake3, verify_file, verify_sha1, verify_sha256,
};
#[cfg(unix)]
pub use permissions::{
    PermissionError, PermissionMode, apply_umask, check_secure_permissions, effective_permissions,
    ensure_secure_dir, get_umask, set_permissions, set_secure_dir_permissions,
    set_secure_permissions,
};

#[cfg(not(unix))]
pub use permissions::{
    PermissionError, PermissionMode, check_secure_permissions, ensure_secure_dir, set_permissions,
    set_secure_dir_permissions, set_secure_permissions,
};
pub use platform::{
    PhpPlatform, PlatformError, PlatformValidator, Requirement, RequirementType, ValidationMode,
};
pub use secure::{
    SecureClientBuilder, SecurityError, create_secure_temp, create_secure_temp_dir, mask_sensitive,
    sanitize_path, validate_package_name, validate_url,
};
pub use signature::{
    Ed25519Verifier, PgpVerifier, SignatureAlgorithm, SignatureError, SignatureVerifier,
    TrustChain, TrustLevel, TrustedKey, TrustedSignatureVerifier, VerifiedSignature,
};

/// Vulnerability severity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Unknown severity.
    Unknown,
    /// Low severity.
    Low,
    /// Medium severity.
    Medium,
    /// High severity.
    High,
    /// Critical severity.
    Critical,
}

impl Severity {
    /// Get severity from CVSS score (0-10).
    #[must_use]
    pub fn from_cvss(score: f32) -> Self {
        match score {
            s if s >= 9.0 => Self::Critical,
            s if s >= 7.0 => Self::High,
            s if s >= 4.0 => Self::Medium,
            s if s > 0.0 => Self::Low,
            _ => Self::Unknown,
        }
    }

    /// Get ANSI color code for display.
    #[must_use]
    pub const fn color(&self) -> &'static str {
        match self {
            Self::Critical => "\x1b[91m", // Bright red
            Self::High => "\x1b[31m",     // Red
            Self::Medium => "\x1b[33m",   // Yellow
            Self::Low => "\x1b[36m",      // Cyan
            Self::Unknown => "\x1b[37m",  // White
        }
    }

    /// Get display name.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Critical => "CRITICAL",
            Self::High => "HIGH",
            Self::Medium => "MEDIUM",
            Self::Low => "LOW",
            Self::Unknown => "UNKNOWN",
        }
    }
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Security vulnerability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vulnerability {
    /// Advisory ID (e.g., CVE-2024-1234).
    pub advisory_id: String,
    /// Affected package.
    pub package: PackageId,
    /// Affected version range.
    pub affected_versions: String,
    /// Fixed in version (if known).
    pub fixed_version: Option<Version>,
    /// Severity level.
    pub severity: Severity,
    /// CVSS score (0-10).
    pub cvss_score: Option<f32>,
    /// Title/summary.
    pub title: String,
    /// Description.
    pub description: String,
    /// Reference URLs.
    pub references: Vec<Url>,
    /// Published date.
    pub published_at: Option<DateTime<Utc>>,
}

impl Vulnerability {
    /// Check if a version is affected (simplified).
    #[must_use]
    pub fn affects_version(&self, version: &Version) -> bool {
        if let Some(ref fixed) = self.fixed_version {
            version < fixed
        } else {
            true
        }
    }
}

/// Audit result for a single package.
#[derive(Debug, Clone)]
pub struct PackageAudit {
    /// Package identifier.
    pub package: PackageId,
    /// Package version.
    pub version: Version,
    /// Found vulnerabilities.
    pub vulnerabilities: Vec<Vulnerability>,
}

impl PackageAudit {
    /// Check if package is vulnerable.
    #[must_use]
    pub const fn is_vulnerable(&self) -> bool {
        !self.vulnerabilities.is_empty()
    }

    /// Get highest severity.
    #[must_use]
    pub fn max_severity(&self) -> Severity {
        self.vulnerabilities
            .iter()
            .map(|v| v.severity)
            .max()
            .unwrap_or(Severity::Unknown)
    }
}

/// Full audit report.
#[derive(Debug, Clone)]
pub struct AuditReport {
    /// Audited packages.
    pub packages: Vec<PackageAudit>,
    /// When audit was performed.
    pub audited_at: DateTime<Utc>,
    /// Advisory database version.
    pub database_version: Option<String>,
}

impl AuditReport {
    /// Get total vulnerability count.
    #[must_use]
    pub fn vulnerability_count(&self) -> usize {
        self.packages.iter().map(|p| p.vulnerabilities.len()).sum()
    }

    /// Get vulnerable package count.
    #[must_use]
    pub fn vulnerable_package_count(&self) -> usize {
        self.packages.iter().filter(|p| p.is_vulnerable()).count()
    }

    /// Check if any critical vulnerabilities exist.
    #[must_use]
    pub fn has_critical(&self) -> bool {
        self.packages
            .iter()
            .flat_map(|p| &p.vulnerabilities)
            .any(|v| v.severity == Severity::Critical)
    }

    /// Get all vulnerabilities grouped by severity.
    #[must_use]
    pub fn by_severity(&self) -> Vec<(Severity, Vec<&Vulnerability>)> {
        let mut by_sev: std::collections::BTreeMap<Severity, Vec<&Vulnerability>> =
            std::collections::BTreeMap::new();

        for pkg in &self.packages {
            for vuln in &pkg.vulnerabilities {
                by_sev.entry(vuln.severity).or_default().push(vuln);
            }
        }

        by_sev.into_iter().rev().collect()
    }

    /// Check if report passes (no critical/high vulnerabilities).
    #[must_use]
    pub fn passes(&self) -> bool {
        !self
            .packages
            .iter()
            .flat_map(|p| &p.vulnerabilities)
            .any(|v| matches!(v.severity, Severity::Critical | Severity::High))
    }
}

/// Security auditor with all features.
#[derive(Debug)]
pub struct Auditor {
    client: Client,
    advisory_db: AdvisoryDatabase,
    integrity_verifier: IntegrityVerifier,
    signature_verifier: SignatureVerifier,
    credential_manager: CredentialManager,
    audit_logger: Option<AuditLogger>,
}

impl Auditor {
    /// Create new auditor with default settings.
    ///
    /// # Errors
    /// Returns error if initialization fails.
    pub fn new() -> CoreResult<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .https_only(true)
            .build()
            .map_err(|e| CoreError::network_simple(e.to_string()))?;

        let advisory_db = AdvisoryDatabase::new().map_err(|e| CoreError::Audit(e.to_string()))?;

        Ok(Self {
            client,
            advisory_db,
            integrity_verifier: IntegrityVerifier::new(),
            signature_verifier: SignatureVerifier::new(),
            credential_manager: CredentialManager::default(),
            audit_logger: None,
        })
    }

    /// Enable audit logging to file.
    #[must_use]
    pub fn with_audit_log(mut self, path: impl Into<PathBuf>) -> Self {
        self.audit_logger = Some(AuditLogger::with_file(path));
        self
    }

    /// Get credential manager.
    #[must_use]
    pub const fn credentials(&self) -> &CredentialManager {
        &self.credential_manager
    }

    /// Get signature verifier.
    #[must_use]
    pub const fn signature_verifier(&mut self) -> &mut SignatureVerifier {
        &mut self.signature_verifier
    }

    /// Get HTTP client for making authenticated requests.
    #[must_use]
    pub const fn client(&self) -> &Client {
        &self.client
    }

    /// Get integrity verifier for computing hashes.
    #[must_use]
    pub const fn integrity_verifier(&self) -> &IntegrityVerifier {
        &self.integrity_verifier
    }

    /// Audit multiple packages for vulnerabilities.
    ///
    /// # Errors
    /// Returns error if audit fails.
    pub async fn audit(&self, packages: &[(PackageId, Version)]) -> CoreResult<AuditReport> {
        info!(packages = packages.len(), "starting security audit");

        // Fetch all advisories in a single bulk API call
        let vuln_map = self
            .advisory_db
            .check_packages(packages)
            .await
            .map_err(|e| CoreError::Audit(e.to_string()))?;

        let audits: Vec<PackageAudit> = packages
            .iter()
            .map(|(package_id, version)| PackageAudit {
                package: package_id.clone(),
                version: version.clone(),
                vulnerabilities: vuln_map.get(package_id).cloned().unwrap_or_default(),
            })
            .collect();

        let report = AuditReport {
            packages: audits,
            audited_at: Utc::now(),
            database_version: None,
        };

        // Log audit
        if let Some(ref logger) = self.audit_logger {
            let _ = logger
                .log_security_scan(packages.len(), report.vulnerability_count())
                .await;
        }

        info!(
            total = report.vulnerability_count(),
            packages = report.vulnerable_package_count(),
            "audit complete"
        );

        Ok(report)
    }

    /// Verify package integrity.
    ///
    /// # Errors
    /// Returns error if verification fails.
    pub async fn verify_integrity(
        &self,
        path: impl AsRef<Path>,
        expected_hash: &str,
        algorithm: HashAlgorithm,
    ) -> CoreResult<()> {
        verify_file(path, algorithm, expected_hash)
            .await
            .map_err(|e| CoreError::integrity(e.to_string()))
    }

    /// Verify package signature.
    ///
    /// # Errors
    /// Returns error if verification fails.
    pub async fn verify_signature(
        &self,
        data_path: impl AsRef<Path>,
        signature_path: impl AsRef<Path>,
    ) -> CoreResult<VerifiedSignature> {
        let data = tokio::fs::read(&data_path)
            .await
            .map_err(|e| CoreError::io(data_path.as_ref(), e))?;
        let signature = tokio::fs::read(&signature_path)
            .await
            .map_err(|e| CoreError::io(signature_path.as_ref(), e))?;

        self.signature_verifier
            .verify(&data, &signature)
            .map_err(|e| CoreError::signature(e.to_string()))
    }

    /// Clear advisory cache.
    pub fn clear_cache(&self) {
        self.advisory_db.clear_cache();
    }
}

/// Audit error type.
#[derive(Debug, Error)]
pub enum AuditError {
    /// Advisory error.
    #[error("advisory error: {0}")]
    Advisory(#[from] AdvisoryError),

    /// Integrity error.
    #[error("integrity error: {0}")]
    Integrity(#[from] IntegrityError),

    /// Signature error.
    #[error("signature error: {0}")]
    Signature(#[from] SignatureError),

    /// Platform error.
    #[error("platform error: {0}")]
    Platform(#[from] PlatformError),

    /// Security error.
    #[error("security error: {0}")]
    Security(#[from] SecurityError),

    /// Credential error.
    #[error("credential error: {0}")]
    Credential(#[from] CredentialError),

    /// Permission error.
    #[error("permission error: {0}")]
    Permission(#[from] PermissionError),

    /// Audit log error.
    #[error("audit log error: {0}")]
    AuditLog(#[from] AuditLogError),

    /// I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result type for audit operations.
pub type Result<T> = std::result::Result<T, AuditError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_severity_from_cvss() {
        assert_eq!(Severity::from_cvss(9.5), Severity::Critical);
        assert_eq!(Severity::from_cvss(7.5), Severity::High);
        assert_eq!(Severity::from_cvss(5.0), Severity::Medium);
        assert_eq!(Severity::from_cvss(2.0), Severity::Low);
        assert_eq!(Severity::from_cvss(0.0), Severity::Unknown);
    }

    #[test]
    fn test_severity_ordering() {
        assert!(Severity::Critical > Severity::High);
        assert!(Severity::High > Severity::Medium);
        assert!(Severity::Medium > Severity::Low);
    }

    #[test]
    fn test_empty_audit_report() {
        let report = AuditReport {
            packages: Vec::new(),
            audited_at: Utc::now(),
            database_version: None,
        };
        assert_eq!(report.vulnerability_count(), 0);
        assert!(!report.has_critical());
        assert!(report.passes());
    }

    #[test]
    fn test_auditor_creation() {
        let auditor = Auditor::new();
        assert!(auditor.is_ok());
    }
}
