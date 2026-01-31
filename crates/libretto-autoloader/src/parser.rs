//! Ultra-fast PHP parser using mago-syntax and mago-names.
//!
//! Extracts class, interface, trait, and enum definitions with their namespaces.
//! Uses mago-names for proper recursive AST walking that finds nested definitions.

use bumpalo::Bump;
use mago_database::file::FileId;
use mago_fingerprint::{FingerprintOptions, Fingerprintable};
use mago_names::ResolvedNames;
use mago_names::resolver::NameResolver;
use mago_span::HasSpan;
use mago_syntax::ast::{Block, Class, Enum, Interface, MethodBody, Program, Statement, Trait};
use mago_syntax::parser::parse_file_content;
use std::path::Path;

/// Result of parsing a PHP file, containing both definitions and fingerprint.
#[derive(Debug, Clone)]
pub struct ParseResult {
    /// Definitions found in the file.
    pub definitions: Vec<PhpDefinition>,
    /// Semantic fingerprint for change detection.
    pub fingerprint: u64,
}

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

/// PHP file parser using mago-syntax and mago-names.
///
/// Uses mago-names' internal walker to properly traverse all nested definitions,
/// including classes inside functions, methods, or other nested contexts.
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

    /// Parse content as string and return definitions only.
    ///
    /// Uses mago-names' NameResolver which internally uses a walker that properly
    /// traverses all AST nodes, including nested class definitions inside functions.
    ///
    /// If you also need the semantic fingerprint for change detection, use
    /// `parse_str_with_fingerprint` instead to avoid parsing twice.
    #[must_use]
    pub fn parse_str(&mut self, content: &str) -> Vec<PhpDefinition> {
        let arena = Bump::new();
        let file_id = FileId::zero();

        // Parse the PHP content
        let (program, _error) = parse_file_content(&arena, file_id, content);

        // Use NameResolver to walk the AST - this handles all nested definitions
        let resolver = NameResolver::new(&arena);
        let resolved_names = resolver.resolve(&program);

        // Collect all definitions by walking the AST ourselves but using resolved names
        let mut definitions = Vec::new();
        self.collect_definitions(&program, &resolved_names, &mut definitions, content);

        definitions
    }

    /// Parse content and return both definitions and semantic fingerprint.
    ///
    /// This is more efficient than calling `parse_str` and computing the fingerprint
    /// separately, as it only parses the file once.
    #[must_use]
    pub fn parse_str_with_fingerprint(&mut self, content: &str) -> ParseResult {
        let arena = Bump::new();
        let file_id = FileId::zero();

        // Parse the PHP content
        let (program, _error) = parse_file_content(&arena, file_id, content);

        // Use NameResolver to walk the AST - this handles all nested definitions
        let resolver = NameResolver::new(&arena);
        let resolved_names = resolver.resolve(&program);

        // Compute semantic fingerprint using mago-fingerprint
        // This is position-insensitive and ignores whitespace changes
        let fingerprint_options = FingerprintOptions::default();
        let fingerprint = program.fingerprint(&resolved_names, &fingerprint_options);

        // Collect all definitions by walking the AST ourselves but using resolved names
        let mut definitions = Vec::new();
        self.collect_definitions(&program, &resolved_names, &mut definitions, content);

        ParseResult {
            definitions,
            fingerprint,
        }
    }

    /// Recursively collect all class-like definitions from the AST.
    fn collect_definitions(
        &self,
        program: &Program<'_>,
        resolved_names: &ResolvedNames<'_>,
        definitions: &mut Vec<PhpDefinition>,
        source: &str,
    ) {
        for statement in program.statements.iter() {
            self.visit_statement(statement, resolved_names, definitions, source);
        }
    }

    /// Visit a statement and all its nested contents.
    fn visit_statement(
        &self,
        statement: &Statement<'_>,
        resolved_names: &ResolvedNames<'_>,
        definitions: &mut Vec<PhpDefinition>,
        source: &str,
    ) {
        match statement {
            Statement::Namespace(ns) => {
                // Visit statements inside the namespace
                // The statements() method is on Namespace, not NamespaceBody
                for stmt in ns.statements().iter() {
                    self.visit_statement(stmt, resolved_names, definitions, source);
                }
            }
            Statement::Class(class) => {
                self.add_class_definition(class, resolved_names, definitions, source);
                // Visit methods for nested classes
                self.visit_class_members(class, resolved_names, definitions, source);
            }
            Statement::Interface(interface) => {
                self.add_interface_definition(interface, resolved_names, definitions, source);
                // Visit methods for nested definitions
                self.visit_interface_members(interface, resolved_names, definitions, source);
            }
            Statement::Trait(tr) => {
                self.add_trait_definition(tr, resolved_names, definitions, source);
                // Visit methods for nested classes
                self.visit_trait_members(tr, resolved_names, definitions, source);
            }
            Statement::Enum(en) => {
                self.add_enum_definition(en, resolved_names, definitions, source);
                // Visit methods for nested classes
                self.visit_enum_members(en, resolved_names, definitions, source);
            }
            Statement::Function(func) => {
                // Visit function body for nested class definitions
                self.visit_block(&func.body, resolved_names, definitions, source);
            }
            Statement::Block(block) => {
                self.visit_block(block, resolved_names, definitions, source);
            }
            Statement::If(if_stmt) => {
                // Visit all branches of the if statement
                for stmt in if_stmt.body.statements() {
                    self.visit_statement(stmt, resolved_names, definitions, source);
                }
            }
            Statement::While(while_stmt) => {
                for stmt in while_stmt.body.statements() {
                    self.visit_statement(stmt, resolved_names, definitions, source);
                }
            }
            Statement::DoWhile(do_while) => {
                // do-while has a single statement
                self.visit_statement(do_while.statement, resolved_names, definitions, source);
            }
            Statement::For(for_stmt) => {
                for stmt in for_stmt.body.statements() {
                    self.visit_statement(stmt, resolved_names, definitions, source);
                }
            }
            Statement::Foreach(foreach_stmt) => {
                for stmt in foreach_stmt.body.statements() {
                    self.visit_statement(stmt, resolved_names, definitions, source);
                }
            }
            Statement::Switch(switch_stmt) => {
                // Switch body has cases, each case has statements
                for case in switch_stmt.body.cases() {
                    for stmt in case.statements() {
                        self.visit_statement(stmt, resolved_names, definitions, source);
                    }
                }
            }
            Statement::Try(try_stmt) => {
                // Visit try block
                self.visit_block(&try_stmt.block, resolved_names, definitions, source);
                // Visit catch blocks
                for catch in try_stmt.catch_clauses.iter() {
                    self.visit_block(&catch.block, resolved_names, definitions, source);
                }
                // Visit finally block
                if let Some(finally) = &try_stmt.finally_clause {
                    self.visit_block(&finally.block, resolved_names, definitions, source);
                }
            }
            // Other statements don't contain nested class definitions
            _ => {}
        }
    }

    /// Visit a block of statements.
    fn visit_block(
        &self,
        block: &Block<'_>,
        resolved_names: &ResolvedNames<'_>,
        definitions: &mut Vec<PhpDefinition>,
        source: &str,
    ) {
        for stmt in block.statements.iter() {
            self.visit_statement(stmt, resolved_names, definitions, source);
        }
    }

    /// Visit class members looking for methods with nested definitions.
    fn visit_class_members(
        &self,
        class: &Class<'_>,
        resolved_names: &ResolvedNames<'_>,
        definitions: &mut Vec<PhpDefinition>,
        source: &str,
    ) {
        for member in class.members.iter() {
            if let mago_syntax::ast::ClassLikeMember::Method(method) = member {
                if let MethodBody::Concrete(block) = &method.body {
                    self.visit_block(block, resolved_names, definitions, source);
                }
            }
        }
    }

    /// Visit interface members looking for methods with nested definitions.
    fn visit_interface_members(
        &self,
        interface: &Interface<'_>,
        resolved_names: &ResolvedNames<'_>,
        definitions: &mut Vec<PhpDefinition>,
        source: &str,
    ) {
        for member in interface.members.iter() {
            if let mago_syntax::ast::ClassLikeMember::Method(method) = member {
                if let MethodBody::Concrete(block) = &method.body {
                    self.visit_block(block, resolved_names, definitions, source);
                }
            }
        }
    }

    /// Visit trait members looking for methods with nested definitions.
    fn visit_trait_members(
        &self,
        tr: &Trait<'_>,
        resolved_names: &ResolvedNames<'_>,
        definitions: &mut Vec<PhpDefinition>,
        source: &str,
    ) {
        for member in tr.members.iter() {
            if let mago_syntax::ast::ClassLikeMember::Method(method) = member {
                if let MethodBody::Concrete(block) = &method.body {
                    self.visit_block(block, resolved_names, definitions, source);
                }
            }
        }
    }

    /// Visit enum members looking for methods with nested definitions.
    fn visit_enum_members(
        &self,
        en: &Enum<'_>,
        resolved_names: &ResolvedNames<'_>,
        definitions: &mut Vec<PhpDefinition>,
        source: &str,
    ) {
        for member in en.members.iter() {
            if let mago_syntax::ast::ClassLikeMember::Method(method) = member {
                if let MethodBody::Concrete(block) = &method.body {
                    self.visit_block(block, resolved_names, definitions, source);
                }
            }
        }
    }

    /// Add a class definition using resolved names.
    fn add_class_definition(
        &self,
        class: &Class<'_>,
        resolved_names: &ResolvedNames<'_>,
        definitions: &mut Vec<PhpDefinition>,
        source: &str,
    ) {
        let name = class.name.value.to_string();
        let line = self.get_line_number(class.span().start.offset as usize, source);

        // Try to get the resolved fully qualified name
        let fqcn = resolved_names
            .resolve(&class.name)
            .map(String::from)
            .unwrap_or_else(|| name.clone());

        let namespace = self.extract_namespace(&fqcn, &name);

        definitions.push(PhpDefinition {
            fqcn,
            name,
            namespace,
            kind: DefinitionKind::Class,
            line,
        });
    }

    /// Add an interface definition using resolved names.
    fn add_interface_definition(
        &self,
        interface: &Interface<'_>,
        resolved_names: &ResolvedNames<'_>,
        definitions: &mut Vec<PhpDefinition>,
        source: &str,
    ) {
        let name = interface.name.value.to_string();
        let line = self.get_line_number(interface.span().start.offset as usize, source);

        let fqcn = resolved_names
            .resolve(&interface.name)
            .map(String::from)
            .unwrap_or_else(|| name.clone());

        let namespace = self.extract_namespace(&fqcn, &name);

        definitions.push(PhpDefinition {
            fqcn,
            name,
            namespace,
            kind: DefinitionKind::Interface,
            line,
        });
    }

    /// Add a trait definition using resolved names.
    fn add_trait_definition(
        &self,
        tr: &Trait<'_>,
        resolved_names: &ResolvedNames<'_>,
        definitions: &mut Vec<PhpDefinition>,
        source: &str,
    ) {
        let name = tr.name.value.to_string();
        let line = self.get_line_number(tr.span().start.offset as usize, source);

        let fqcn = resolved_names
            .resolve(&tr.name)
            .map(String::from)
            .unwrap_or_else(|| name.clone());

        let namespace = self.extract_namespace(&fqcn, &name);

        definitions.push(PhpDefinition {
            fqcn,
            name,
            namespace,
            kind: DefinitionKind::Trait,
            line,
        });
    }

    /// Add an enum definition using resolved names.
    fn add_enum_definition(
        &self,
        en: &Enum<'_>,
        resolved_names: &ResolvedNames<'_>,
        definitions: &mut Vec<PhpDefinition>,
        source: &str,
    ) {
        let name = en.name.value.to_string();
        let line = self.get_line_number(en.span().start.offset as usize, source);

        let fqcn = resolved_names
            .resolve(&en.name)
            .map(String::from)
            .unwrap_or_else(|| name.clone());

        let namespace = self.extract_namespace(&fqcn, &name);

        definitions.push(PhpDefinition {
            fqcn,
            name,
            namespace,
            kind: DefinitionKind::Enum,
            line,
        });
    }

    /// Extract namespace from fully qualified name.
    fn extract_namespace(&self, fqcn: &str, name: &str) -> Option<String> {
        if fqcn.contains('\\') {
            let ns = fqcn.strip_suffix(&format!("\\{name}"))?;
            Some(ns.to_string())
        } else {
            None
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

    #[test]
    fn parse_nested_class_in_function() {
        let mut parser = PhpParser::new();
        let content = r#"<?php
function foo() {
    class NestedA {
        public function bar(): void {
            class NestedB {}
        }
    }
}
"#;

        let defs = parser.parse_str(content);
        assert_eq!(defs.len(), 2, "Should find both nested classes A and B");
        assert!(defs.iter().any(|d| d.name == "NestedA"));
        assert!(defs.iter().any(|d| d.name == "NestedB"));
    }

    #[test]
    fn parse_class_in_if_block() {
        let mut parser = PhpParser::new();
        let content = r#"<?php
if (true) {
    class ConditionalClass {}
}
"#;

        let defs = parser.parse_str(content);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "ConditionalClass");
    }

    #[test]
    fn parse_class_in_method() {
        let mut parser = PhpParser::new();
        let content = r#"<?php
namespace App;

class Outer {
    public function createInner(): void {
        class Inner {}
    }
}
"#;

        let defs = parser.parse_str(content);
        assert_eq!(defs.len(), 2, "Should find both Outer and Inner classes");
        assert!(defs.iter().any(|d| d.name == "Outer"));
        assert!(defs.iter().any(|d| d.name == "Inner"));
    }
}
