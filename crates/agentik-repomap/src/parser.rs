//! Tree-sitter based code parser.

use std::path::Path;

use tree_sitter::{Language as TSLanguage, Parser};

use crate::types::{FileInfo, Import, Language, Symbol, SymbolKind};

/// Error type for parsing operations.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("unsupported language: {0:?}")]
    UnsupportedLanguage(Language),

    #[error("failed to parse file: {0}")]
    ParseFailed(String),

    #[error("tree-sitter error: {0}")]
    TreeSitter(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("query error: {0}")]
    Query(#[from] tree_sitter::QueryError),
}

/// Multi-language tree-sitter parser.
pub struct TreeSitterParser {
    rust_parser: Parser,
    typescript_parser: Parser,
    javascript_parser: Parser,
    python_parser: Parser,
    go_parser: Parser,
    java_parser: Parser,
}

impl TreeSitterParser {
    /// Create a new parser with all supported languages initialized.
    pub fn new() -> Result<Self, ParseError> {
        Ok(Self {
            rust_parser: Self::create_parser(tree_sitter_rust::LANGUAGE.into())?,
            typescript_parser: Self::create_parser(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())?,
            javascript_parser: Self::create_parser(tree_sitter_javascript::LANGUAGE.into())?,
            python_parser: Self::create_parser(tree_sitter_python::LANGUAGE.into())?,
            go_parser: Self::create_parser(tree_sitter_go::LANGUAGE.into())?,
            java_parser: Self::create_parser(tree_sitter_java::LANGUAGE.into())?,
        })
    }

    fn create_parser(language: TSLanguage) -> Result<Parser, ParseError> {
        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .map_err(|e| ParseError::TreeSitter(e.to_string()))?;
        Ok(parser)
    }

    /// Parse a file and extract symbols and imports.
    pub fn parse_file(&mut self, path: &Path, content: &str) -> Result<FileInfo, ParseError> {
        let language = Language::from_path(path);
        if !language.is_supported() {
            return Err(ParseError::UnsupportedLanguage(language));
        }

        let mut file_info = FileInfo::new(path.to_path_buf(), language);

        let (symbols, imports) = match language {
            Language::Rust => self.parse_rust(content)?,
            Language::TypeScript => self.parse_typescript(content)?,
            Language::JavaScript => self.parse_javascript(content)?,
            Language::Python => self.parse_python(content)?,
            Language::Go => self.parse_go(content)?,
            Language::Java => self.parse_java(content)?,
            Language::Unknown => return Err(ParseError::UnsupportedLanguage(language)),
        };

        file_info.symbols = symbols;
        file_info.imports = imports;

        Ok(file_info)
    }

    /// Parse Rust source code.
    fn parse_rust(&mut self, content: &str) -> Result<(Vec<Symbol>, Vec<Import>), ParseError> {
        let tree = self
            .rust_parser
            .parse(content, None)
            .ok_or_else(|| ParseError::ParseFailed("Rust parsing failed".to_string()))?;

        let mut symbols = Vec::new();
        let mut imports = Vec::new();

        let root = tree.root_node();
        let mut cursor = root.walk();

        // Walk the tree to find declarations
        self.walk_rust_node(&mut cursor, content, &mut symbols, &mut imports, None);

        Ok((symbols, imports))
    }

    fn walk_rust_node(
        &self,
        cursor: &mut tree_sitter::TreeCursor,
        content: &str,
        symbols: &mut Vec<Symbol>,
        imports: &mut Vec<Import>,
        parent: Option<&str>,
    ) {
        loop {
            let node = cursor.node();
            let kind = node.kind();

            match kind {
                "function_item" | "function_signature_item" => {
                    if let Some(sym) = self.extract_rust_function(&node, content, parent) {
                        symbols.push(sym);
                    }
                }
                "struct_item" => {
                    if let Some(name) = self.get_rust_name(&node, content) {
                        let line = node.start_position().row + 1;
                        symbols.push(Symbol::new(&name, SymbolKind::Struct, line));

                        // Walk into impl blocks for methods
                        if cursor.goto_first_child() {
                            self.walk_rust_node(cursor, content, symbols, imports, Some(&name));
                            cursor.goto_parent();
                        }
                    }
                }
                "enum_item" => {
                    if let Some(name) = self.get_rust_name(&node, content) {
                        let line = node.start_position().row + 1;
                        symbols.push(Symbol::new(&name, SymbolKind::Enum, line));
                    }
                }
                "trait_item" => {
                    if let Some(name) = self.get_rust_name(&node, content) {
                        let line = node.start_position().row + 1;
                        symbols.push(Symbol::new(&name, SymbolKind::Trait, line));

                        // Walk into trait for method signatures
                        if cursor.goto_first_child() {
                            self.walk_rust_node(cursor, content, symbols, imports, Some(&name));
                            cursor.goto_parent();
                        }
                    }
                }
                "impl_item" => {
                    // Get the type being implemented
                    let impl_type = self.get_rust_impl_type(&node, content);
                    if cursor.goto_first_child() {
                        self.walk_rust_node(
                            cursor,
                            content,
                            symbols,
                            imports,
                            impl_type.as_deref(),
                        );
                        cursor.goto_parent();
                    }
                }
                "use_declaration" => {
                    if let Some(import) = self.extract_rust_use(&node, content) {
                        imports.push(import);
                    }
                }
                "mod_item" => {
                    if let Some(name) = self.get_rust_mod_name(&node, content) {
                        let line = node.start_position().row + 1;
                        symbols.push(Symbol::new(&name, SymbolKind::Module, line));
                    }
                }
                "type_item" => {
                    if let Some(name) = self.get_rust_name(&node, content) {
                        let line = node.start_position().row + 1;
                        symbols.push(Symbol::new(&name, SymbolKind::TypeAlias, line));
                    }
                }
                "const_item" | "static_item" => {
                    if let Some(name) = self.get_rust_name(&node, content) {
                        let line = node.start_position().row + 1;
                        symbols.push(Symbol::new(&name, SymbolKind::Constant, line));
                    }
                }
                _ => {
                    // Recurse into children
                    if cursor.goto_first_child() {
                        self.walk_rust_node(cursor, content, symbols, imports, parent);
                        cursor.goto_parent();
                    }
                }
            }

            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    fn extract_rust_function(
        &self,
        node: &tree_sitter::Node,
        content: &str,
        parent: Option<&str>,
    ) -> Option<Symbol> {
        let name_node = node.child_by_field_name("name")?;
        let name = name_node.utf8_text(content.as_bytes()).ok()?;
        let line = node.start_position().row + 1;

        // Extract signature (up to the opening brace or semicolon)
        let start = node.start_byte();
        let mut end = node.end_byte();

        // Find the body or semicolon to truncate
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if child.kind() == "block" || child.kind() == ";" {
                    end = child.start_byte();
                    break;
                }
            }
        }

        let signature = content
            .get(start..end)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let mut sym = Symbol::new(name, SymbolKind::Function, line);
        if let Some(sig) = signature {
            sym = sym.with_signature(sig);
        }
        if let Some(p) = parent {
            sym = sym.with_parent(p);
        }
        Some(sym)
    }

    fn get_rust_name(&self, node: &tree_sitter::Node, content: &str) -> Option<String> {
        node.child_by_field_name("name")
            .and_then(|n| n.utf8_text(content.as_bytes()).ok())
            .map(|s| s.to_string())
    }

    fn get_rust_mod_name(&self, node: &tree_sitter::Node, content: &str) -> Option<String> {
        // mod items have the name as child
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if child.kind() == "identifier" {
                    return child.utf8_text(content.as_bytes()).ok().map(|s| s.to_string());
                }
            }
        }
        None
    }

    fn get_rust_impl_type(&self, node: &tree_sitter::Node, content: &str) -> Option<String> {
        // Try to get the type being implemented
        node.child_by_field_name("type")
            .and_then(|n| n.utf8_text(content.as_bytes()).ok())
            .map(|s| s.to_string())
    }

    fn extract_rust_use(&self, node: &tree_sitter::Node, content: &str) -> Option<Import> {
        let line = node.start_position().row + 1;

        // Get the use tree
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if child.kind() == "use_tree" || child.kind() == "scoped_identifier" {
                    let path = child.utf8_text(content.as_bytes()).ok()?;
                    return Some(Import::new(path.to_string(), line));
                }
            }
        }
        None
    }

    /// Parse TypeScript source code.
    fn parse_typescript(&mut self, content: &str) -> Result<(Vec<Symbol>, Vec<Import>), ParseError> {
        let tree = self
            .typescript_parser
            .parse(content, None)
            .ok_or_else(|| ParseError::ParseFailed("TypeScript parsing failed".to_string()))?;

        let mut symbols = Vec::new();
        let mut imports = Vec::new();

        let root = tree.root_node();
        let mut cursor = root.walk();

        self.walk_ts_node(&mut cursor, content, &mut symbols, &mut imports, None);

        Ok((symbols, imports))
    }

    /// Parse JavaScript source code.
    fn parse_javascript(&mut self, content: &str) -> Result<(Vec<Symbol>, Vec<Import>), ParseError> {
        let tree = self
            .javascript_parser
            .parse(content, None)
            .ok_or_else(|| ParseError::ParseFailed("JavaScript parsing failed".to_string()))?;

        let mut symbols = Vec::new();
        let mut imports = Vec::new();

        let root = tree.root_node();
        let mut cursor = root.walk();

        self.walk_ts_node(&mut cursor, content, &mut symbols, &mut imports, None);

        Ok((symbols, imports))
    }

    fn walk_ts_node(
        &self,
        cursor: &mut tree_sitter::TreeCursor,
        content: &str,
        symbols: &mut Vec<Symbol>,
        imports: &mut Vec<Import>,
        parent: Option<&str>,
    ) {
        loop {
            let node = cursor.node();
            let kind = node.kind();

            match kind {
                "function_declaration" | "method_definition" | "arrow_function" => {
                    if let Some(sym) = self.extract_ts_function(&node, content, parent) {
                        symbols.push(sym);
                    }
                }
                "class_declaration" => {
                    if let Some(name) = self.get_ts_name(&node, content) {
                        let line = node.start_position().row + 1;
                        symbols.push(Symbol::new(&name, SymbolKind::Struct, line));

                        // Walk into class for methods
                        if cursor.goto_first_child() {
                            self.walk_ts_node(cursor, content, symbols, imports, Some(&name));
                            cursor.goto_parent();
                        }
                    }
                }
                "interface_declaration" | "type_alias_declaration" => {
                    if let Some(name) = self.get_ts_name(&node, content) {
                        let line = node.start_position().row + 1;
                        let kind = if kind == "interface_declaration" {
                            SymbolKind::Trait
                        } else {
                            SymbolKind::TypeAlias
                        };
                        symbols.push(Symbol::new(&name, kind, line));
                    }
                }
                "enum_declaration" => {
                    if let Some(name) = self.get_ts_name(&node, content) {
                        let line = node.start_position().row + 1;
                        symbols.push(Symbol::new(&name, SymbolKind::Enum, line));
                    }
                }
                "import_statement" => {
                    if let Some(import) = self.extract_ts_import(&node, content) {
                        imports.push(import);
                    }
                }
                "export_statement" => {
                    // Check for exported declarations
                    if cursor.goto_first_child() {
                        self.walk_ts_node(cursor, content, symbols, imports, parent);
                        cursor.goto_parent();
                    }
                }
                "lexical_declaration" | "variable_declaration" => {
                    // Check for const function assignments like `const foo = () => {}`
                    if cursor.goto_first_child() {
                        self.walk_ts_node(cursor, content, symbols, imports, parent);
                        cursor.goto_parent();
                    }
                }
                "variable_declarator" => {
                    // Check if this is a const arrow function
                    if let Some(sym) = self.extract_ts_const_function(&node, content, parent) {
                        symbols.push(sym);
                    }
                }
                _ => {
                    // Recurse into children
                    if cursor.goto_first_child() {
                        self.walk_ts_node(cursor, content, symbols, imports, parent);
                        cursor.goto_parent();
                    }
                }
            }

            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    fn extract_ts_function(
        &self,
        node: &tree_sitter::Node,
        content: &str,
        parent: Option<&str>,
    ) -> Option<Symbol> {
        let name = self.get_ts_name(node, content)?;
        let line = node.start_position().row + 1;

        // Extract signature
        let start = node.start_byte();
        let mut end = node.end_byte();

        // Find the body to truncate
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if child.kind() == "statement_block" {
                    end = child.start_byte();
                    break;
                }
            }
        }

        let signature = content
            .get(start..end)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let mut sym = Symbol::new(name, SymbolKind::Function, line);
        if let Some(sig) = signature {
            sym = sym.with_signature(sig);
        }
        if let Some(p) = parent {
            sym = sym.with_parent(p);
        }
        Some(sym)
    }

    fn extract_ts_const_function(
        &self,
        node: &tree_sitter::Node,
        content: &str,
        parent: Option<&str>,
    ) -> Option<Symbol> {
        // Check if the value is an arrow function
        let name_node = node.child_by_field_name("name")?;
        let value_node = node.child_by_field_name("value")?;

        if value_node.kind() != "arrow_function" {
            return None;
        }

        let name = name_node.utf8_text(content.as_bytes()).ok()?;
        let line = node.start_position().row + 1;

        let mut sym = Symbol::new(name, SymbolKind::Function, line);
        if let Some(p) = parent {
            sym = sym.with_parent(p);
        }
        Some(sym)
    }

    fn get_ts_name(&self, node: &tree_sitter::Node, content: &str) -> Option<String> {
        node.child_by_field_name("name")
            .and_then(|n| n.utf8_text(content.as_bytes()).ok())
            .map(|s| s.to_string())
    }

    fn extract_ts_import(&self, node: &tree_sitter::Node, content: &str) -> Option<Import> {
        let line = node.start_position().row + 1;

        // Find the source string
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if child.kind() == "string" {
                    let path = child
                        .utf8_text(content.as_bytes())
                        .ok()?
                        .trim_matches(|c| c == '"' || c == '\'');
                    return Some(Import::new(path, line));
                }
            }
        }
        None
    }

    /// Parse Python source code.
    fn parse_python(&mut self, content: &str) -> Result<(Vec<Symbol>, Vec<Import>), ParseError> {
        let tree = self
            .python_parser
            .parse(content, None)
            .ok_or_else(|| ParseError::ParseFailed("Python parsing failed".to_string()))?;

        let mut symbols = Vec::new();
        let mut imports = Vec::new();

        let root = tree.root_node();
        let mut cursor = root.walk();

        self.walk_python_node(&mut cursor, content, &mut symbols, &mut imports, None);

        Ok((symbols, imports))
    }

    fn walk_python_node(
        &self,
        cursor: &mut tree_sitter::TreeCursor,
        content: &str,
        symbols: &mut Vec<Symbol>,
        imports: &mut Vec<Import>,
        parent: Option<&str>,
    ) {
        loop {
            let node = cursor.node();
            let kind = node.kind();

            match kind {
                "function_definition" => {
                    if let Some(sym) = self.extract_python_function(&node, content, parent) {
                        symbols.push(sym);
                    }
                }
                "class_definition" => {
                    if let Some(name) = self.get_python_name(&node, content) {
                        let line = node.start_position().row + 1;
                        symbols.push(Symbol::new(&name, SymbolKind::Struct, line));

                        // Walk into class for methods
                        if cursor.goto_first_child() {
                            self.walk_python_node(cursor, content, symbols, imports, Some(&name));
                            cursor.goto_parent();
                        }
                    }
                }
                "import_statement" | "import_from_statement" => {
                    if let Some(import) = self.extract_python_import(&node, content) {
                        imports.push(import);
                    }
                }
                _ => {
                    // Recurse into children
                    if cursor.goto_first_child() {
                        self.walk_python_node(cursor, content, symbols, imports, parent);
                        cursor.goto_parent();
                    }
                }
            }

            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    fn extract_python_function(
        &self,
        node: &tree_sitter::Node,
        content: &str,
        parent: Option<&str>,
    ) -> Option<Symbol> {
        let name = self.get_python_name(node, content)?;
        let line = node.start_position().row + 1;

        // Extract signature (def line including parameters)
        let start = node.start_byte();
        let mut end = node.end_byte();

        // Find the body (block) to truncate
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if child.kind() == "block" {
                    end = child.start_byte();
                    break;
                }
            }
        }

        let signature = content
            .get(start..end)
            .map(|s| s.trim().trim_end_matches(':').trim().to_string())
            .filter(|s| !s.is_empty());

        let mut sym = Symbol::new(name, SymbolKind::Function, line);
        if let Some(sig) = signature {
            sym = sym.with_signature(sig);
        }
        if let Some(p) = parent {
            sym = sym.with_parent(p);
        }
        Some(sym)
    }

    fn get_python_name(&self, node: &tree_sitter::Node, content: &str) -> Option<String> {
        node.child_by_field_name("name")
            .and_then(|n| n.utf8_text(content.as_bytes()).ok())
            .map(|s| s.to_string())
    }

    fn extract_python_import(&self, node: &tree_sitter::Node, content: &str) -> Option<Import> {
        let line = node.start_position().row + 1;
        let kind = node.kind();

        if kind == "import_statement" {
            // import foo, bar
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.kind() == "dotted_name" {
                        let path = child.utf8_text(content.as_bytes()).ok()?;
                        return Some(Import::new(path, line));
                    }
                }
            }
        } else if kind == "import_from_statement" {
            // from foo import bar
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.kind() == "dotted_name" || child.kind() == "relative_import" {
                        let path = child.utf8_text(content.as_bytes()).ok()?;

                        // Collect imported items
                        let mut items = Vec::new();
                        for j in (i + 1)..node.child_count() {
                            if let Some(item_child) = node.child(j) {
                                if item_child.kind() == "identifier" {
                                    if let Ok(item) = item_child.utf8_text(content.as_bytes()) {
                                        items.push(item.to_string());
                                    }
                                }
                            }
                        }

                        return Some(Import::new(path, line).with_items(items));
                    }
                }
            }
        }
        None
    }

    /// Parse Go source code.
    fn parse_go(&mut self, content: &str) -> Result<(Vec<Symbol>, Vec<Import>), ParseError> {
        let tree = self
            .go_parser
            .parse(content, None)
            .ok_or_else(|| ParseError::ParseFailed("Go parsing failed".to_string()))?;

        let mut symbols = Vec::new();
        let mut imports = Vec::new();

        let root = tree.root_node();
        let mut cursor = root.walk();

        self.walk_go_node(&mut cursor, content, &mut symbols, &mut imports);

        Ok((symbols, imports))
    }

    fn walk_go_node(
        &self,
        cursor: &mut tree_sitter::TreeCursor,
        content: &str,
        symbols: &mut Vec<Symbol>,
        imports: &mut Vec<Import>,
    ) {
        loop {
            let node = cursor.node();
            let kind = node.kind();

            match kind {
                "function_declaration" | "method_declaration" => {
                    if let Some(sym) = self.extract_go_function(&node, content) {
                        symbols.push(sym);
                    }
                }
                "type_declaration" => {
                    if cursor.goto_first_child() {
                        self.walk_go_node(cursor, content, symbols, imports);
                        cursor.goto_parent();
                    }
                }
                "type_spec" => {
                    if let Some(sym) = self.extract_go_type(&node, content) {
                        symbols.push(sym);
                    }
                }
                "import_declaration" => {
                    self.extract_go_imports(&node, content, imports);
                }
                _ => {
                    if cursor.goto_first_child() {
                        self.walk_go_node(cursor, content, symbols, imports);
                        cursor.goto_parent();
                    }
                }
            }

            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    fn extract_go_function(&self, node: &tree_sitter::Node, content: &str) -> Option<Symbol> {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(content.as_bytes()).ok())?;
        let line = node.start_position().row + 1;

        // Get receiver for methods
        let parent = node
            .child_by_field_name("receiver")
            .and_then(|r| {
                // Extract type from receiver
                for i in 0..r.child_count() {
                    if let Some(child) = r.child(i) {
                        if child.kind() == "type_identifier" || child.kind() == "pointer_type" {
                            return child.utf8_text(content.as_bytes()).ok();
                        }
                    }
                }
                None
            })
            .map(|s| s.trim_start_matches('*').to_string());

        // Extract signature
        let start = node.start_byte();
        let mut end = node.end_byte();

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if child.kind() == "block" {
                    end = child.start_byte();
                    break;
                }
            }
        }

        let signature = content
            .get(start..end)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let mut sym = Symbol::new(name, SymbolKind::Function, line);
        if let Some(sig) = signature {
            sym = sym.with_signature(sig);
        }
        if let Some(p) = parent {
            sym = sym.with_parent(p);
        }
        Some(sym)
    }

    fn extract_go_type(&self, node: &tree_sitter::Node, content: &str) -> Option<Symbol> {
        let name = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(content.as_bytes()).ok())?;
        let line = node.start_position().row + 1;

        // Determine kind from type field
        let kind = node
            .child_by_field_name("type")
            .map(|t| match t.kind() {
                "struct_type" => SymbolKind::Struct,
                "interface_type" => SymbolKind::Trait,
                _ => SymbolKind::TypeAlias,
            })
            .unwrap_or(SymbolKind::TypeAlias);

        Some(Symbol::new(name, kind, line))
    }

    fn extract_go_imports(
        &self,
        node: &tree_sitter::Node,
        content: &str,
        imports: &mut Vec<Import>,
    ) {
        let line = node.start_position().row + 1;

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if child.kind() == "import_spec" || child.kind() == "interpreted_string_literal" {
                    if let Ok(path) = child.utf8_text(content.as_bytes()) {
                        let path = path.trim_matches('"');
                        imports.push(Import::new(path, line));
                    }
                } else if child.kind() == "import_spec_list" {
                    // Multiple imports
                    for j in 0..child.child_count() {
                        if let Some(spec) = child.child(j) {
                            if spec.kind() == "import_spec" {
                                for k in 0..spec.child_count() {
                                    if let Some(str_node) = spec.child(k) {
                                        if str_node.kind() == "interpreted_string_literal" {
                                            if let Ok(path) = str_node.utf8_text(content.as_bytes()) {
                                                let path = path.trim_matches('"');
                                                imports.push(Import::new(path, line));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Parse Java source code.
    fn parse_java(&mut self, content: &str) -> Result<(Vec<Symbol>, Vec<Import>), ParseError> {
        let tree = self
            .java_parser
            .parse(content, None)
            .ok_or_else(|| ParseError::ParseFailed("Java parsing failed".to_string()))?;

        let mut symbols = Vec::new();
        let mut imports = Vec::new();

        let root = tree.root_node();
        let mut cursor = root.walk();

        self.walk_java_node(&mut cursor, content, &mut symbols, &mut imports, None);

        Ok((symbols, imports))
    }

    fn walk_java_node(
        &self,
        cursor: &mut tree_sitter::TreeCursor,
        content: &str,
        symbols: &mut Vec<Symbol>,
        imports: &mut Vec<Import>,
        parent: Option<&str>,
    ) {
        loop {
            let node = cursor.node();
            let kind = node.kind();

            match kind {
                "method_declaration" | "constructor_declaration" => {
                    if let Some(sym) = self.extract_java_method(&node, content, parent) {
                        symbols.push(sym);
                    }
                }
                "class_declaration" => {
                    if let Some(name) = self.get_java_name(&node, content) {
                        let line = node.start_position().row + 1;
                        symbols.push(Symbol::new(&name, SymbolKind::Struct, line));

                        if cursor.goto_first_child() {
                            self.walk_java_node(cursor, content, symbols, imports, Some(&name));
                            cursor.goto_parent();
                        }
                    }
                }
                "interface_declaration" => {
                    if let Some(name) = self.get_java_name(&node, content) {
                        let line = node.start_position().row + 1;
                        symbols.push(Symbol::new(&name, SymbolKind::Trait, line));

                        if cursor.goto_first_child() {
                            self.walk_java_node(cursor, content, symbols, imports, Some(&name));
                            cursor.goto_parent();
                        }
                    }
                }
                "enum_declaration" => {
                    if let Some(name) = self.get_java_name(&node, content) {
                        let line = node.start_position().row + 1;
                        symbols.push(Symbol::new(&name, SymbolKind::Enum, line));
                    }
                }
                "import_declaration" => {
                    if let Some(import) = self.extract_java_import(&node, content) {
                        imports.push(import);
                    }
                }
                _ => {
                    if cursor.goto_first_child() {
                        self.walk_java_node(cursor, content, symbols, imports, parent);
                        cursor.goto_parent();
                    }
                }
            }

            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    fn extract_java_method(
        &self,
        node: &tree_sitter::Node,
        content: &str,
        parent: Option<&str>,
    ) -> Option<Symbol> {
        let name = self.get_java_name(node, content)?;
        let line = node.start_position().row + 1;

        // Extract signature
        let start = node.start_byte();
        let mut end = node.end_byte();

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if child.kind() == "block" {
                    end = child.start_byte();
                    break;
                }
            }
        }

        let signature = content
            .get(start..end)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let mut sym = Symbol::new(name, SymbolKind::Function, line);
        if let Some(sig) = signature {
            sym = sym.with_signature(sig);
        }
        if let Some(p) = parent {
            sym = sym.with_parent(p);
        }
        Some(sym)
    }

    fn get_java_name(&self, node: &tree_sitter::Node, content: &str) -> Option<String> {
        node.child_by_field_name("name")
            .and_then(|n| n.utf8_text(content.as_bytes()).ok())
            .map(|s| s.to_string())
    }

    fn extract_java_import(&self, node: &tree_sitter::Node, content: &str) -> Option<Import> {
        let line = node.start_position().row + 1;

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if child.kind() == "scoped_identifier" {
                    let path = child.utf8_text(content.as_bytes()).ok()?;
                    return Some(Import::new(path, line));
                }
            }
        }
        None
    }
}

impl Default for TreeSitterParser {
    fn default() -> Self {
        Self::new().expect("Failed to initialize tree-sitter parsers")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_parse_rust_function() {
        let mut parser = TreeSitterParser::new().unwrap();
        let content = r#"
fn hello_world(name: &str) -> String {
    format!("Hello, {}!", name)
}
"#;

        let file = parser
            .parse_file(Path::new("test.rs"), content)
            .unwrap();

        assert_eq!(file.symbols.len(), 1);
        assert_eq!(file.symbols[0].name, "hello_world");
        assert_eq!(file.symbols[0].kind, SymbolKind::Function);
        assert!(file.symbols[0].signature.is_some());
    }

    #[test]
    fn test_parse_rust_struct_with_impl() {
        let mut parser = TreeSitterParser::new().unwrap();
        let content = r#"
struct MyStruct {
    field: i32,
}

impl MyStruct {
    fn new() -> Self {
        Self { field: 0 }
    }

    fn get_field(&self) -> i32 {
        self.field
    }
}
"#;

        let file = parser
            .parse_file(Path::new("test.rs"), content)
            .unwrap();

        // Should have: struct + 2 methods
        assert!(file.symbols.len() >= 3);

        let struct_sym = file.symbols.iter().find(|s| s.name == "MyStruct").unwrap();
        assert_eq!(struct_sym.kind, SymbolKind::Struct);

        let new_sym = file.symbols.iter().find(|s| s.name == "new").unwrap();
        assert_eq!(new_sym.parent.as_deref(), Some("MyStruct"));
    }

    #[test]
    fn test_parse_rust_use() {
        let mut parser = TreeSitterParser::new().unwrap();
        let content = r#"
use std::collections::HashMap;
use crate::types;
"#;

        let file = parser
            .parse_file(Path::new("test.rs"), content)
            .unwrap();

        // Should capture use statements
        // Note: The parser captures the use tree structure, not the full path
        assert!(!file.imports.is_empty());
    }

    #[test]
    fn test_parse_typescript_class() {
        let mut parser = TreeSitterParser::new().unwrap();
        let content = r#"
import { Foo } from './foo';

class MyClass {
    constructor(private name: string) {}

    greet(): string {
        return `Hello, ${this.name}!`;
    }
}

export function helper(x: number): number {
    return x * 2;
}
"#;

        let file = parser
            .parse_file(Path::new("test.ts"), content)
            .unwrap();

        assert!(file.imports.len() >= 1);
        assert!(file.symbols.iter().any(|s| s.name == "MyClass"));
        assert!(file.symbols.iter().any(|s| s.name == "helper"));
    }

    #[test]
    fn test_parse_python_class() {
        let mut parser = TreeSitterParser::new().unwrap();
        let content = r#"
from typing import List
import os

class MyClass:
    def __init__(self, name: str):
        self.name = name

    def greet(self) -> str:
        return f"Hello, {self.name}!"

def helper(x: int) -> int:
    return x * 2
"#;

        let file = parser
            .parse_file(Path::new("test.py"), content)
            .unwrap();

        assert!(file.imports.len() >= 1);
        assert!(file.symbols.iter().any(|s| s.name == "MyClass"));
        assert!(file.symbols.iter().any(|s| s.name == "helper"));
        assert!(file.symbols.iter().any(|s| s.name == "__init__"));
    }

    #[test]
    fn test_unsupported_language() {
        let mut parser = TreeSitterParser::new().unwrap();
        let result = parser.parse_file(Path::new("test.txt"), "hello world");
        assert!(result.is_err());
    }
}
