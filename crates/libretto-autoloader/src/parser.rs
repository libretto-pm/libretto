//! Ultra-fast PHP parser using mago-syntax.
//!
//! Extracts class, interface, trait, and enum definitions with their namespaces.
//! Uses mago-syntax for faster, more accurate parsing than tree-sitter-php.

use bumpalo::Bump;
use mago_database::file::FileId;
use mago_span::HasSpan;
use mago_syntax::ast::{Namespace, NamespaceBody, Statement};
use mago_syntax::parser::parse_file_content;
use std::path::Path;

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

/// PHP file parser using mago-syntax.
///
/// mago-syntax is faster and more accurate than tree-sitter-php.
#[derive(Debug, Default)]
pub struct PhpParser {
    // mago-syntax uses arena allocation, so we don't need to store state here
    _private: (),
}

impl PhpParser {
    /// Create a new PHP parser.
    #[must_use]
    pub fn new() -> Self {
        Self { _private: () }
    }

    /// Parse a PHP file and extract definitions.
    ///
    /// # Errors
    /// Returns error if file cannot be read.
    pub fn parse_file(&mut self, path: &Path) -> std::io::Result<Vec<PhpDefinition>> {
        let content = std::fs::read_to_string(path)?;
        Ok(self.parse_str(&content))
    }

    /// Parse PHP content from bytes.
    #[must_use]
    pub fn parse_bytes(&mut self, content: &[u8]) -> Vec<PhpDefinition> {
        match std::str::from_utf8(content) {
            Ok(s) => self.parse_str(s),
            Err(_) => Vec::new(),
        }
    }

    /// Parse content as string.
    #[must_use]
    pub fn parse_str(&mut self, content: &str) -> Vec<PhpDefinition> {
        let arena = Bump::new();
        let file_id = FileId::zero(); // Use dummy/zero file ID for standalone parsing

        // Parse the PHP content
        // API: (program, Option<error>) - will change in 1.4.0 to just program with program.errors
        let (program, _error) = parse_file_content(&arena, file_id, content);

        let mut definitions = Vec::new();
        let mut current_namespace: Option<String> = None;

        // Process top-level statements
        for statement in &program.statements {
            self.process_statement(statement, &mut current_namespace, &mut definitions, content);
        }

        definitions
    }

    /// Process a single statement, extracting definitions.
    fn process_statement(
        &self,
        statement: &Statement<'_>,
        current_namespace: &mut Option<String>,
        definitions: &mut Vec<PhpDefinition>,
        source: &str,
    ) {
        match statement {
            Statement::Namespace(ns) => {
                self.process_namespace(ns, current_namespace, definitions, source);
            }
            Statement::Class(class) => {
                let name = class.name.value.to_string();
                let line = self.get_line_number(class.span().start.offset as usize, source);
                definitions.push(PhpDefinition {
                    fqcn: self.make_fqcn(current_namespace.as_deref(), &name),
                    name,
                    namespace: current_namespace.clone(),
                    kind: DefinitionKind::Class,
                    line,
                });
            }
            Statement::Interface(interface) => {
                let name = interface.name.value.to_string();
                let line = self.get_line_number(interface.span().start.offset as usize, source);
                definitions.push(PhpDefinition {
                    fqcn: self.make_fqcn(current_namespace.as_deref(), &name),
                    name,
                    namespace: current_namespace.clone(),
                    kind: DefinitionKind::Interface,
                    line,
                });
            }
            Statement::Trait(tr) => {
                let name = tr.name.value.to_string();
                let line = self.get_line_number(tr.span().start.offset as usize, source);
                definitions.push(PhpDefinition {
                    fqcn: self.make_fqcn(current_namespace.as_deref(), &name),
                    name,
                    namespace: current_namespace.clone(),
                    kind: DefinitionKind::Trait,
                    line,
                });
            }
            Statement::Enum(en) => {
                let name = en.name.value.to_string();
                let line = self.get_line_number(en.span().start.offset as usize, source);
                definitions.push(PhpDefinition {
                    fqcn: self.make_fqcn(current_namespace.as_deref(), &name),
                    name,
                    namespace: current_namespace.clone(),
                    kind: DefinitionKind::Enum,
                    line,
                });
            }
            // Other statements don't contain class-like definitions at top level
            _ => {}
        }
    }

    /// Process a namespace statement.
    fn process_namespace(
        &self,
        ns: &Namespace<'_>,
        current_namespace: &mut Option<String>,
        definitions: &mut Vec<PhpDefinition>,
        source: &str,
    ) {
        // Update current namespace
        *current_namespace = ns.name.as_ref().map(|id| id.value().to_string());

        // Process statements inside the namespace
        match &ns.body {
            NamespaceBody::Implicit(body) => {
                for stmt in &body.statements {
                    self.process_statement(stmt, current_namespace, definitions, source);
                }
            }
            NamespaceBody::BraceDelimited(block) => {
                for stmt in &block.statements {
                    self.process_statement(stmt, current_namespace, definitions, source);
                }
            }
        }
    }

    /// Build a fully qualified class name.
    fn make_fqcn(&self, namespace: Option<&str>, name: &str) -> String {
        match namespace {
            Some(ns) => format!("{ns}\\{name}"),
            None => name.to_string(),
        }
    }

    /// Get line number from byte offset.
    fn get_line_number(&self, offset: usize, source: &str) -> usize {
        source[..offset.min(source.len())]
            .bytes()
            .filter(|&b| b == b'\n')
            .count()
            + 1
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
        let content = r"<?php
namespace App\Models;

class User {
    public $name;
}
";

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
        let content = r"<?php
namespace App\Contracts;

interface UserRepositoryInterface {
    public function find(int $id): ?User;
}
";

        let defs = parser.parse_str(content);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].fqcn, "App\\Contracts\\UserRepositoryInterface");
        assert_eq!(defs[0].kind, DefinitionKind::Interface);
    }

    #[test]
    fn parse_trait() {
        let mut parser = PhpParser::new();
        let content = r"<?php
namespace App\Traits;

trait HasUuid {
    public function generateUuid(): string {
        return bin2hex(random_bytes(16));
    }
}
";

        let defs = parser.parse_str(content);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].fqcn, "App\\Traits\\HasUuid");
        assert_eq!(defs[0].kind, DefinitionKind::Trait);
    }

    #[test]
    fn parse_enum() {
        let mut parser = PhpParser::new();
        let content = r"<?php
namespace App\Enums;

enum Status: string {
    case Pending = 'pending';
    case Active = 'active';
    case Inactive = 'inactive';
}
";

        let defs = parser.parse_str(content);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].fqcn, "App\\Enums\\Status");
        assert_eq!(defs[0].kind, DefinitionKind::Enum);
    }

    #[test]
    fn parse_multiple_definitions() {
        let mut parser = PhpParser::new();
        let content = r"<?php
namespace App\Models;

class User {}

interface Authenticatable {}

trait HasTimestamps {}

enum UserType: int {}
";

        let defs = parser.parse_str(content);
        assert_eq!(defs.len(), 4);
    }

    #[test]
    fn parse_no_namespace() {
        let mut parser = PhpParser::new();
        let content = r"<?php
class GlobalClass {
}
";

        let defs = parser.parse_str(content);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].fqcn, "GlobalClass");
        assert!(defs[0].namespace.is_none());
    }

    #[test]
    fn parse_braced_namespace() {
        let mut parser = PhpParser::new();
        let content = r"<?php
namespace App\Services {
    class UserService {}
}

namespace App\Repositories {
    class UserRepository {}
}
";

        let defs = parser.parse_str(content);
        assert_eq!(defs.len(), 2);
        assert_eq!(defs[0].fqcn, "App\\Services\\UserService");
        assert_eq!(defs[1].fqcn, "App\\Repositories\\UserRepository");
    }
}
