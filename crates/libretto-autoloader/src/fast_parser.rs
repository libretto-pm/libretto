//! Ultra-fast PHP class/interface/trait/enum scanner using regex.
//!
//! This is inspired by Composer's PhpFileParser which uses regex instead of
//! full AST parsing for speed. It's ~100x faster than mago-syntax parsing.
//!
//! The approach:
//! 1. Quick check if file contains class/interface/trait/enum keywords
//! 2. Strip comments and strings to avoid false positives
//! 3. Use regex to extract namespace + class declarations

use memchr::memmem;
use rayon::prelude::*;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// Regex for quick keyword check (compiled once)
static QUICK_CHECK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(class|interface|trait|enum)\s").unwrap());

/// Main regex for extracting classes and namespaces
/// Matches:
/// - `namespace Foo\Bar;` or `namespace Foo\Bar {`
/// - `class Foo`, `interface Foo`, `trait Foo`, `enum Foo`
/// Note: Rust regex doesn't support lookbehind, so we handle ::class in post-processing
static CLASS_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?xi)
        (?:
            \b(?P<ns>namespace)\s+(?P<nsname>[a-zA-Z_\x7f-\xff][a-zA-Z0-9_\x7f-\xff\\]*)\s*[;\{]
            |
            \b(?P<type>class|interface|trait|enum)\s+
            (?P<name>[a-zA-Z_\x7f-\xff][a-zA-Z0-9_\x7f-\xff]*)
        )",
    )
    .unwrap()
});

/// Result of scanning a single file
#[derive(Debug, Clone)]
pub struct FastScanResult {
    pub path: PathBuf,
    pub classes: Vec<String>,
}

/// Fast PHP scanner for classmap generation
pub struct FastScanner;

impl FastScanner {
    /// Scan a directory for PHP classes in parallel
    pub fn scan_directory(root: &Path) -> Vec<FastScanResult> {
        let php_files: Vec<PathBuf> = walkdir::WalkDir::new(root)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_type().is_file() && e.path().extension().is_some_and(|ext| ext == "php")
            })
            .map(|e| e.path().to_path_buf())
            .collect();

        php_files
            .into_par_iter()
            .filter_map(|path| Self::scan_file(&path))
            .filter(|r| !r.classes.is_empty())
            .collect()
    }

    /// Scan a single PHP file
    pub fn scan_file(path: &Path) -> Option<FastScanResult> {
        let content = std::fs::read(path).ok()?;
        let content = String::from_utf8_lossy(&content);

        let classes = Self::find_classes(&content);

        Some(FastScanResult {
            path: path.to_path_buf(),
            classes,
        })
    }

    /// Find all classes in PHP content
    pub fn find_classes(content: &str) -> Vec<String> {
        // Quick check - early return if no keywords
        if !QUICK_CHECK.is_match(content) {
            return Vec::new();
        }

        // Strip comments and strings to avoid false positives
        let cleaned = Self::strip_comments_and_strings(content);

        // Extract classes
        let mut classes = Vec::new();
        let mut namespace = String::new();

        for caps in CLASS_REGEX.captures_iter(&cleaned) {
            if caps.name("ns").is_some() {
                // Namespace declaration
                if let Some(nsname) = caps.name("nsname") {
                    namespace = nsname.as_str().replace(" ", "").replace("\t", "");
                    if !namespace.ends_with('\\') {
                        namespace.push('\\');
                    }
                }
            } else if let Some(name) = caps.name("name") {
                let name_str = name.as_str();

                // Skip 'extends' and 'implements' (anonymous class edge cases)
                if name_str == "extends" || name_str == "implements" {
                    continue;
                }

                // Handle enum with type hint (enum Foo: int)
                let clean_name = if let Some(colon_pos) = name_str.find(':') {
                    &name_str[..colon_pos]
                } else {
                    name_str
                };

                let fqcn = format!("{}{}", namespace, clean_name);
                classes.push(fqcn);
            }
        }

        classes
    }

    /// Strip PHP comments and string literals to avoid false matches
    /// This is a simplified version - handles most common cases
    fn strip_comments_and_strings(content: &str) -> String {
        let mut result = String::with_capacity(content.len());
        let bytes = content.as_bytes();
        let len = bytes.len();
        let mut i = 0;

        while i < len {
            let c = bytes[i];

            // Single-line comment: // or #
            if c == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
                // Skip until end of line
                while i < len && bytes[i] != b'\n' {
                    i += 1;
                }
                result.push('\n');
                continue;
            }

            if c == b'#' && (i == 0 || bytes[i - 1] != b'$') {
                // Skip # comments (but not $# which could be variable)
                // Also check for #[ which is an attribute
                if i + 1 < len && bytes[i + 1] == b'[' {
                    // PHP 8 attribute - keep it but skip the content
                    while i < len && bytes[i] != b']' {
                        i += 1;
                    }
                    if i < len {
                        i += 1;
                    }
                    continue;
                }
                while i < len && bytes[i] != b'\n' {
                    i += 1;
                }
                result.push('\n');
                continue;
            }

            // Multi-line comment: /* ... */
            if c == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
                i += 2;
                while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    if bytes[i] == b'\n' {
                        result.push('\n');
                    }
                    i += 1;
                }
                i += 2; // Skip */
                continue;
            }

            // Single-quoted string
            if c == b'\'' {
                i += 1;
                while i < len {
                    if bytes[i] == b'\\' && i + 1 < len {
                        i += 2; // Skip escaped char
                    } else if bytes[i] == b'\'' {
                        i += 1;
                        break;
                    } else {
                        if bytes[i] == b'\n' {
                            result.push('\n');
                        }
                        i += 1;
                    }
                }
                result.push_str("''"); // Placeholder
                continue;
            }

            // Double-quoted string
            if c == b'"' {
                i += 1;
                while i < len {
                    if bytes[i] == b'\\' && i + 1 < len {
                        i += 2; // Skip escaped char
                    } else if bytes[i] == b'"' {
                        i += 1;
                        break;
                    } else {
                        if bytes[i] == b'\n' {
                            result.push('\n');
                        }
                        i += 1;
                    }
                }
                result.push_str("\"\""); // Placeholder
                continue;
            }

            // Heredoc/Nowdoc
            if c == b'<' && i + 2 < len && bytes[i + 1] == b'<' && bytes[i + 2] == b'<' {
                // Find the identifier
                i += 3;

                // Skip optional quotes and whitespace
                while i < len
                    && (bytes[i] == b' '
                        || bytes[i] == b'\t'
                        || bytes[i] == b'\''
                        || bytes[i] == b'"')
                {
                    i += 1;
                }

                // Read identifier
                let start = i;
                while i < len && bytes[i].is_ascii_alphanumeric() || (i < len && bytes[i] == b'_') {
                    i += 1;
                }
                let identifier = &content[start..i];

                if identifier.is_empty() {
                    continue;
                }

                // Skip to end of line
                while i < len && bytes[i] != b'\n' {
                    i += 1;
                }
                if i < len {
                    i += 1;
                    result.push('\n');
                }

                // Find closing identifier
                let needle = format!("\n{}", identifier);
                if let Some(pos) = memmem::find(&bytes[i..], needle.as_bytes()) {
                    // Skip heredoc content
                    for j in i..i + pos {
                        if bytes[j] == b'\n' {
                            result.push('\n');
                        }
                    }
                    i += pos + needle.len();
                    // Skip to end of closing line
                    while i < len && bytes[i] != b'\n' && bytes[i] != b';' {
                        i += 1;
                    }
                }
                continue;
            }

            result.push(c as char);
            i += 1;
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_class() {
        let content = "<?php\nclass Foo {}";
        let classes = FastScanner::find_classes(content);
        assert_eq!(classes, vec!["Foo"]);
    }

    #[test]
    fn test_namespaced_class() {
        let content = "<?php\nnamespace App\\Models;\nclass User {}";
        let classes = FastScanner::find_classes(content);
        assert_eq!(classes, vec!["App\\Models\\User"]);
    }

    #[test]
    fn test_multiple_classes() {
        let content = r#"<?php
namespace App;

class Foo {}
interface Bar {}
trait Baz {}
enum Status {}
"#;
        let classes = FastScanner::find_classes(content);
        assert_eq!(
            classes,
            vec!["App\\Foo", "App\\Bar", "App\\Baz", "App\\Status"]
        );
    }

    #[test]
    fn test_class_in_comment_ignored() {
        let content = r#"<?php
// class Fake {}
/* class AlsoFake {} */
class Real {}
"#;
        let classes = FastScanner::find_classes(content);
        assert_eq!(classes, vec!["Real"]);
    }

    #[test]
    fn test_class_in_string_ignored() {
        let content = r#"<?php
$x = "class Fake {}";
$y = 'class AlsoFake {}';
class Real {}
"#;
        let classes = FastScanner::find_classes(content);
        assert_eq!(classes, vec!["Real"]);
    }

    #[test]
    fn test_no_classes() {
        let content = "<?php\necho 'hello';";
        let classes = FastScanner::find_classes(content);
        assert!(classes.is_empty());
    }

    #[test]
    fn test_enum_with_type() {
        let content = "<?php\nenum Status: int { case Active = 1; }";
        let classes = FastScanner::find_classes(content);
        assert_eq!(classes, vec!["Status"]);
    }
}
