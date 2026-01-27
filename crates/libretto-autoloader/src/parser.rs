//! Ultra-fast PHP parser using tree-sitter-php.
//!
//! Extracts class, interface, trait, and enum definitions with their namespaces.

use std::path::Path;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Parser, Query, QueryCursor};

/// PHP definition type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DefinitionKind {
    /// Class definition.
    Class,
    /// Interface definition.
    Interface,
    /// Trait definition.
    Trait,
    /// Enum definition (PHP 8.1+).
    Enum,
}

impl DefinitionKind {
    /// Get string representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Class => "class",
            Self::Interface => "interface",
            Self::Trait => "trait",
            Self::Enum => "enum",
        }
    }
}

/// A PHP definition (class, interface, trait, or enum).
#[derive(Debug, Clone)]
pub struct PhpDefinition {
    /// The fully qualified class name.
    pub fqcn: String,
    /// The short name (without namespace).
    pub name: String,
    /// The namespace (if any).
    pub namespace: Option<String>,
    /// Definition type.
    pub kind: DefinitionKind,
    /// Line number in the file.
    pub line: usize,
}

/// PHP file parser using tree-sitter.
pub struct PhpParser {
    parser: Parser,
    query: Query,
}

impl std::fmt::Debug for PhpParser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhpParser").finish_non_exhaustive()
    }
}

impl PhpParser {
    /// Create a new PHP parser.
    ///
    /// # Panics
    /// Panics if tree-sitter-php cannot be initialized.
    #[must_use]
    pub fn new() -> Self {
        let language = tree_sitter_php::LANGUAGE_PHP;

        let mut parser = Parser::new();
        parser
            .set_language(&language.into())
            .expect("Failed to set PHP language");

        // Query to find namespace, class, interface, trait, and enum declarations
        let query_source = r#"
            (namespace_definition
                name: (namespace_name) @namespace)

            (class_declaration
                name: (name) @class.name) @class

            (interface_declaration
                name: (name) @interface.name) @interface

            (trait_declaration
                name: (name) @trait.name) @trait

            (enum_declaration
                name: (name) @enum.name) @enum
        "#;

        let query =
            Query::new(&language.into(), query_source).expect("Failed to create tree-sitter query");

        Self { parser, query }
    }

    /// Parse a PHP file and extract definitions.
    ///
    /// # Errors
    /// Returns error if file cannot be read or parsed.
    pub fn parse_file(&mut self, path: &Path) -> std::io::Result<Vec<PhpDefinition>> {
        let content = std::fs::read(path)?;
        Ok(self.parse_bytes(&content))
    }

    /// Parse PHP content from bytes.
    #[must_use]
    pub fn parse_bytes(&mut self, content: &[u8]) -> Vec<PhpDefinition> {
        let tree = match self.parser.parse(content, None) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let root = tree.root_node();
        let mut cursor = QueryCursor::new();

        let mut definitions = Vec::new();
        let mut current_namespace: Option<String> = None;

        // Get capture indices
        let namespace_idx = self
            .query
            .capture_index_for_name("namespace")
            .unwrap_or(u32::MAX);
        let class_name_idx = self
            .query
            .capture_index_for_name("class.name")
            .unwrap_or(u32::MAX);
        let interface_name_idx = self
            .query
            .capture_index_for_name("interface.name")
            .unwrap_or(u32::MAX);
        let trait_name_idx = self
            .query
            .capture_index_for_name("trait.name")
            .unwrap_or(u32::MAX);
        let enum_name_idx = self
            .query
            .capture_index_for_name("enum.name")
            .unwrap_or(u32::MAX);

        // Use captures() with StreamingIterator
        let mut captures = cursor.captures(&self.query, root, content);
        while let Some((m, _)) = captures.next() {
            for capture in m.captures {
                let node = capture.node;
                let text: &str = match node.utf8_text(content) {
                    Ok(t) => t,
                    Err(_) => continue,
                };

                if capture.index == namespace_idx {
                    current_namespace = Some(text.to_string());
                } else {
                    let (kind, is_name) = if capture.index == class_name_idx {
                        (DefinitionKind::Class, true)
                    } else if capture.index == interface_name_idx {
                        (DefinitionKind::Interface, true)
                    } else if capture.index == trait_name_idx {
                        (DefinitionKind::Trait, true)
                    } else if capture.index == enum_name_idx {
                        (DefinitionKind::Enum, true)
                    } else {
                        continue;
                    };

                    if is_name {
                        let name = text.to_string();
                        let fqcn = match &current_namespace {
                            Some(ns) => format!("{}\\{}", ns, name),
                            None => name.clone(),
                        };

                        definitions.push(PhpDefinition {
                            fqcn,
                            name,
                            namespace: current_namespace.clone(),
                            kind,
                            line: node.start_position().row + 1,
                        });
                    }
                }
            }
        }

        definitions
    }

    /// Parse content as string (convenience method).
    #[must_use]
    pub fn parse_str(&mut self, content: &str) -> Vec<PhpDefinition> {
        self.parse_bytes(content.as_bytes())
    }
}

impl Default for PhpParser {
    fn default() -> Self {
        Self::new()
    }
}

/// Thread-local parser pool for parallel parsing.
pub struct ParserPool;

impl ParserPool {
    /// Get a parser for the current thread.
    #[must_use]
    pub fn get() -> PhpParser {
        PhpParser::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_class() {
        let mut parser = PhpParser::new();
        let content = r#"<?php
namespace App\Models;

class User {
    public $name;
}
"#;

        let defs = parser.parse_str(content);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].fqcn, "App\\Models\\User");
        assert_eq!(defs[0].name, "User");
        assert_eq!(defs[0].namespace, Some("App\\Models".to_string()));
        assert_eq!(defs[0].kind, DefinitionKind::Class);
    }

    #[test]
    fn parse_interface() {
        let mut parser = PhpParser::new();
        let content = r#"<?php
namespace App\Contracts;

interface UserRepositoryInterface {
    public function find(int $id): ?User;
}
"#;

        let defs = parser.parse_str(content);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].fqcn, "App\\Contracts\\UserRepositoryInterface");
        assert_eq!(defs[0].kind, DefinitionKind::Interface);
    }

    #[test]
    fn parse_trait() {
        let mut parser = PhpParser::new();
        let content = r#"<?php
namespace App\Traits;

trait HasUuid {
    public function generateUuid(): string {
        return bin2hex(random_bytes(16));
    }
}
"#;

        let defs = parser.parse_str(content);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].fqcn, "App\\Traits\\HasUuid");
        assert_eq!(defs[0].kind, DefinitionKind::Trait);
    }

    #[test]
    fn parse_enum() {
        let mut parser = PhpParser::new();
        let content = r#"<?php
namespace App\Enums;

enum Status: string {
    case Pending = 'pending';
    case Active = 'active';
    case Inactive = 'inactive';
}
"#;

        let defs = parser.parse_str(content);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].fqcn, "App\\Enums\\Status");
        assert_eq!(defs[0].kind, DefinitionKind::Enum);
    }

    #[test]
    fn parse_multiple_definitions() {
        let mut parser = PhpParser::new();
        let content = r#"<?php
namespace App\Models;

class User {}

interface Authenticatable {}

trait HasTimestamps {}

enum UserType: int {}
"#;

        let defs = parser.parse_str(content);
        assert_eq!(defs.len(), 4);
    }

    #[test]
    fn parse_no_namespace() {
        let mut parser = PhpParser::new();
        let content = r#"<?php
class GlobalClass {
}
"#;

        let defs = parser.parse_str(content);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].fqcn, "GlobalClass");
        assert!(defs[0].namespace.is_none());
    }
}
