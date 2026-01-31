//! Ultra-fast PHP class/interface/trait/enum scanner.
//!
//! Ported from PHP's zend_strip() + Composer's PhpFileParser.
//! Single-pass state machine that's as fast as PHP's C implementation.

use memchr::memmem;
use rayon::prelude::*;
use std::path::{Path, PathBuf};

/// Result of scanning a single file
#[derive(Debug, Clone)]
pub struct FastScanResult {
    pub path: PathBuf,
    pub classes: Vec<String>,
}

/// Fast PHP scanner for classmap generation
#[derive(Debug)]
pub struct FastScanner;

impl FastScanner {
    /// Scan a directory for PHP classes in parallel
    pub fn scan_directory(root: &Path) -> Vec<FastScanResult> {
        let php_files: Vec<PathBuf> = walkdir::WalkDir::new(root)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_type().is_file()
                    && e.path()
                        .extension()
                        .map_or(false, |ext| ext.eq_ignore_ascii_case("php"))
            })
            .map(|e| e.into_path())
            .collect();

        php_files
            .into_par_iter()
            .filter_map(|path| Self::scan_file(&path))
            .collect()
    }

    #[inline]
    pub fn scan_file(path: &Path) -> Option<FastScanResult> {
        let content = std::fs::read(path).ok()?;
        if content.is_empty() {
            return None;
        }
        let classes = Self::find_classes(&content);
        if classes.is_empty() {
            return None;
        }
        Some(FastScanResult {
            path: path.to_path_buf(),
            classes,
        })
    }

    /// Single-pass scanner - finds classes while skipping comments/strings inline.
    /// This is faster than strip-then-scan because we avoid allocating a second buffer.
    pub fn find_classes(content: &[u8]) -> Vec<String> {
        // Quick rejection - use SIMD to check if any keywords exist
        if !has_class_keyword(content) {
            return Vec::new();
        }

        let mut classes = Vec::with_capacity(4);
        let mut namespace = String::new();
        let len = content.len();
        let mut i = 0;

        // State machine - using constants for speed
        let mut in_line_comment = false;
        let mut in_block_comment = false;
        let mut in_single_string = false;
        let mut in_double_string = false;
        let mut in_heredoc = false;
        let mut heredoc_id: &[u8] = &[];

        while i < len {
            // Skip states - handle comments and strings
            if in_line_comment {
                if content[i] == b'\n' {
                    in_line_comment = false;
                }
                i += 1;
                continue;
            }

            if in_block_comment {
                if content[i] == b'*' && i + 1 < len && content[i + 1] == b'/' {
                    in_block_comment = false;
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }

            if in_single_string {
                if content[i] == b'\\' && i + 1 < len {
                    i += 2;
                } else if content[i] == b'\'' {
                    in_single_string = false;
                    i += 1;
                } else {
                    i += 1;
                }
                continue;
            }

            if in_double_string {
                if content[i] == b'\\' && i + 1 < len {
                    i += 2;
                } else if content[i] == b'"' {
                    in_double_string = false;
                    i += 1;
                } else {
                    i += 1;
                }
                continue;
            }

            if in_heredoc {
                // Check for closing identifier at start of line
                let line_start = i;
                // Skip whitespace (PHP 7.3+)
                while i < len && (content[i] == b' ' || content[i] == b'\t') {
                    i += 1;
                }
                if i + heredoc_id.len() <= len && &content[i..i + heredoc_id.len()] == heredoc_id {
                    let after = i + heredoc_id.len();
                    if after >= len
                        || content[after] == b';'
                        || content[after] == b'\n'
                        || content[after] == b','
                        || content[after] == b')'
                    {
                        in_heredoc = false;
                        i = after;
                        continue;
                    }
                }
                // Skip to next line
                i = line_start;
                while i < len && content[i] != b'\n' {
                    i += 1;
                }
                if i < len {
                    i += 1;
                }
                continue;
            }

            // Main code parsing
            let b = content[i];

            // Check for comments
            if b == b'/' && i + 1 < len {
                if content[i + 1] == b'/' {
                    in_line_comment = true;
                    i += 2;
                    continue;
                }
                if content[i + 1] == b'*' {
                    in_block_comment = true;
                    i += 2;
                    continue;
                }
            }

            if b == b'#' && i + 1 < len && content[i + 1] != b'[' {
                in_line_comment = true;
                i += 1;
                continue;
            }

            // Check for strings
            if b == b'\'' {
                in_single_string = true;
                i += 1;
                continue;
            }

            if b == b'"' {
                in_double_string = true;
                i += 1;
                continue;
            }

            // Check for heredoc/nowdoc
            if b == b'<' && i + 2 < len && content[i + 1] == b'<' && content[i + 2] == b'<' {
                i += 3;
                while i < len && content[i] == b' ' {
                    i += 1;
                }
                if i < len && (content[i] == b'\'' || content[i] == b'"') {
                    i += 1;
                }
                let id_start = i;
                while i < len && (content[i].is_ascii_alphanumeric() || content[i] == b'_') {
                    i += 1;
                }
                if i > id_start {
                    heredoc_id = &content[id_start..i];
                    in_heredoc = true;
                    // Skip closing quote
                    if i < len && (content[i] == b'\'' || content[i] == b'"') {
                        i += 1;
                    }
                    // Skip to newline
                    while i < len && content[i] != b'\n' {
                        i += 1;
                    }
                    if i < len {
                        i += 1;
                    }
                }
                continue;
            }

            // Check for keywords - only if preceded by valid boundary
            if i == 0 || is_boundary_char(content[i - 1]) {
                // namespace
                if b == b'n' && i + 9 <= len && &content[i..i + 9] == b"namespace" {
                    if i + 9 >= len || content[i + 9].is_ascii_whitespace() {
                        i += 9;
                        while i < len && content[i].is_ascii_whitespace() {
                            i += 1;
                        }
                        let ns_start = i;
                        while i < len {
                            let c = content[i];
                            if c.is_ascii_alphanumeric() || c == b'_' || c == b'\\' {
                                i += 1;
                            } else if c.is_ascii_whitespace() {
                                i += 1;
                            } else {
                                break;
                            }
                        }
                        namespace = content[ns_start..i]
                            .iter()
                            .filter(|&&c| !c.is_ascii_whitespace())
                            .map(|&c| c as char)
                            .collect();
                        if !namespace.is_empty() && !namespace.ends_with('\\') {
                            namespace.push('\\');
                        }
                        continue;
                    }
                }

                // class
                if b == b'c' && i + 5 <= len && &content[i..i + 5] == b"class" {
                    if i + 5 >= len || content[i + 5].is_ascii_whitespace() {
                        i += 5;
                        if let Some(name) = read_name(content, &mut i) {
                            classes.push(format!("{}{}", namespace, name));
                        }
                        continue;
                    }
                }

                // interface
                if b == b'i' && i + 9 <= len && &content[i..i + 9] == b"interface" {
                    if i + 9 >= len || content[i + 9].is_ascii_whitespace() {
                        i += 9;
                        if let Some(name) = read_name(content, &mut i) {
                            classes.push(format!("{}{}", namespace, name));
                        }
                        continue;
                    }
                }

                // trait
                if b == b't' && i + 5 <= len && &content[i..i + 5] == b"trait" {
                    if i + 5 >= len || content[i + 5].is_ascii_whitespace() {
                        i += 5;
                        if let Some(name) = read_name(content, &mut i) {
                            classes.push(format!("{}{}", namespace, name));
                        }
                        continue;
                    }
                }

                // enum
                if b == b'e' && i + 4 <= len && &content[i..i + 4] == b"enum" {
                    if i + 4 >= len || content[i + 4].is_ascii_whitespace() {
                        i += 4;
                        if let Some(name) = read_name(content, &mut i) {
                            classes.push(format!("{}{}", namespace, name));
                        }
                        continue;
                    }
                }
            }

            i += 1;
        }

        classes
    }
}

/// SIMD-accelerated check for class-like keywords
#[inline]
fn has_class_keyword(content: &[u8]) -> bool {
    memmem::find(content, b"class").is_some()
        || memmem::find(content, b"interface").is_some()
        || memmem::find(content, b"trait").is_some()
        || memmem::find(content, b"enum").is_some()
}

/// Check if character is a valid boundary (not part of identifier)
#[inline]
fn is_boundary_char(c: u8) -> bool {
    !c.is_ascii_alphanumeric() && c != b'_' && c != b':' && c != b'$'
}

/// Read class/interface/trait/enum name
#[inline]
fn read_name<'a>(content: &'a [u8], i: &mut usize) -> Option<&'a str> {
    let len = content.len();

    // Skip whitespace
    while *i < len && content[*i].is_ascii_whitespace() {
        *i += 1;
    }

    let start = *i;

    // Read identifier
    while *i < len {
        let c = content[*i];
        if c.is_ascii_alphanumeric() || c == b'_' {
            *i += 1;
        } else {
            break;
        }
    }

    if *i == start {
        return None;
    }

    let name = &content[start..*i];

    // Skip keywords
    if name == b"extends" || name == b"implements" {
        return None;
    }

    std::str::from_utf8(name).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_class() {
        let content = b"<?php\nclass Foo {}";
        let classes = FastScanner::find_classes(content);
        assert_eq!(classes, vec!["Foo"]);
    }

    #[test]
    fn test_namespaced_class() {
        let content = b"<?php\nnamespace App\\Models;\nclass User {}";
        let classes = FastScanner::find_classes(content);
        assert_eq!(classes, vec!["App\\Models\\User"]);
    }

    #[test]
    fn test_multiple_classes() {
        let content = br#"<?php
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
        let content = br#"<?php
// class Fake {}
/* class AlsoFake {} */
class Real {}
"#;
        let classes = FastScanner::find_classes(content);
        assert_eq!(classes, vec!["Real"]);
    }

    #[test]
    fn test_class_in_string_ignored() {
        let content = br#"<?php
$x = "class Fake {}";
$y = 'class AlsoFake {}';
class Real {}
"#;
        let classes = FastScanner::find_classes(content);
        assert_eq!(classes, vec!["Real"]);
    }

    #[test]
    fn test_no_classes() {
        let content = b"<?php\necho 'hello';";
        let classes = FastScanner::find_classes(content);
        assert!(classes.is_empty());
    }

    #[test]
    fn test_enum_with_type() {
        let content = b"<?php\nenum Status: int { case Active = 1; }";
        let classes = FastScanner::find_classes(content);
        assert_eq!(classes, vec!["Status"]);
    }

    #[test]
    fn test_class_constant_ignored() {
        let content = b"<?php\n$x = SomeClass::class;\nclass Real {}";
        let classes = FastScanner::find_classes(content);
        assert_eq!(classes, vec!["Real"]);
    }

    #[test]
    fn test_attribute() {
        let content = br#"<?php
#[Attribute]
class MyAttribute {}
"#;
        let classes = FastScanner::find_classes(content);
        assert_eq!(classes, vec!["MyAttribute"]);
    }

    #[test]
    fn test_heredoc() {
        let content = br#"<?php
$x = <<<EOT
class Fake {}
EOT;
class Real {}
"#;
        let classes = FastScanner::find_classes(content);
        assert_eq!(classes, vec!["Real"]);
    }
}
