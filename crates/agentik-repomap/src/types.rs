//! Core types for repository mapping.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

/// Supported programming languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Go,
    Java,
    Unknown,
}

impl Language {
    /// Detect language from file extension.
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "rs" => Language::Rust,
            "ts" | "tsx" => Language::TypeScript,
            "js" | "jsx" | "mjs" | "cjs" => Language::JavaScript,
            "py" | "pyi" => Language::Python,
            "go" => Language::Go,
            "java" => Language::Java,
            _ => Language::Unknown,
        }
    }

    /// Detect language from file path.
    pub fn from_path(path: &std::path::Path) -> Self {
        path.extension()
            .and_then(|e| e.to_str())
            .map(Self::from_extension)
            .unwrap_or(Language::Unknown)
    }

    /// Check if this language is supported for parsing.
    pub fn is_supported(&self) -> bool {
        !matches!(self, Language::Unknown)
    }
}

/// Kind of symbol extracted from source code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    /// Function or method
    Function,
    /// Struct (Rust), class (Python/TS/Java)
    Struct,
    /// Enum type
    Enum,
    /// Trait (Rust), interface (TS/Java)
    Trait,
    /// Type alias
    TypeAlias,
    /// Constant or static
    Constant,
    /// Module declaration
    Module,
}

/// A symbol extracted from source code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    /// Symbol name
    pub name: String,
    /// Kind of symbol
    pub kind: SymbolKind,
    /// Signature (for functions) or declaration line
    pub signature: Option<String>,
    /// Line number where symbol is defined
    pub line: usize,
    /// Parent symbol (e.g., method's class)
    pub parent: Option<String>,
}

impl Symbol {
    /// Create a new symbol.
    pub fn new(name: impl Into<String>, kind: SymbolKind, line: usize) -> Self {
        Self {
            name: name.into(),
            kind,
            signature: None,
            line,
            parent: None,
        }
    }

    /// Set the signature.
    pub fn with_signature(mut self, sig: impl Into<String>) -> Self {
        self.signature = Some(sig.into());
        self
    }

    /// Set the parent symbol.
    pub fn with_parent(mut self, parent: impl Into<String>) -> Self {
        self.parent = Some(parent.into());
        self
    }
}

/// Import declaration from source code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Import {
    /// The raw import path as written in source
    pub raw_path: String,
    /// Resolved file path (if within repository)
    pub resolved_path: Option<PathBuf>,
    /// Specific items imported (for `from x import a, b`)
    pub items: Vec<String>,
    /// Line number of import
    pub line: usize,
}

impl Import {
    /// Create a new import.
    pub fn new(raw_path: impl Into<String>, line: usize) -> Self {
        Self {
            raw_path: raw_path.into(),
            resolved_path: None,
            items: Vec::new(),
            line,
        }
    }

    /// Add imported items.
    pub fn with_items(mut self, items: Vec<String>) -> Self {
        self.items = items;
        self
    }

    /// Set the resolved path.
    pub fn with_resolved(mut self, path: PathBuf) -> Self {
        self.resolved_path = Some(path);
        self
    }
}

/// Information about a single file in the repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    /// Relative path from repository root
    pub path: PathBuf,
    /// Detected language
    pub language: Language,
    /// Symbols defined in this file
    pub symbols: Vec<Symbol>,
    /// Imports/dependencies declared
    pub imports: Vec<Import>,
    /// File modification time (for caching)
    #[serde(with = "system_time_serde")]
    pub mtime: SystemTime,
    /// File size in bytes
    pub size: u64,
}

impl FileInfo {
    /// Create new file info.
    pub fn new(path: impl Into<PathBuf>, language: Language) -> Self {
        Self {
            path: path.into(),
            language,
            symbols: Vec::new(),
            imports: Vec::new(),
            mtime: SystemTime::UNIX_EPOCH,
            size: 0,
        }
    }

    /// Set symbols.
    pub fn with_symbols(mut self, symbols: Vec<Symbol>) -> Self {
        self.symbols = symbols;
        self
    }

    /// Set imports.
    pub fn with_imports(mut self, imports: Vec<Import>) -> Self {
        self.imports = imports;
        self
    }

    /// Set modification time.
    pub fn with_mtime(mut self, mtime: SystemTime) -> Self {
        self.mtime = mtime;
        self
    }

    /// Set file size.
    pub fn with_size(mut self, size: u64) -> Self {
        self.size = size;
        self
    }

    /// Get function symbols.
    pub fn functions(&self) -> impl Iterator<Item = &Symbol> {
        self.symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
    }

    /// Get type symbols (struct, enum, trait).
    pub fn types(&self) -> impl Iterator<Item = &Symbol> {
        self.symbols.iter().filter(|s| {
            matches!(
                s.kind,
                SymbolKind::Struct | SymbolKind::Enum | SymbolKind::Trait
            )
        })
    }
}

/// The complete repository map.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoMap {
    /// Repository root path
    pub root: PathBuf,
    /// All parsed files
    pub files: HashMap<PathBuf, FileInfo>,
    /// PageRank scores for each file
    pub ranks: HashMap<PathBuf, f64>,
    /// Cache version for compatibility
    pub version: u32,
}

impl RepoMap {
    /// Current cache version.
    pub const VERSION: u32 = 1;

    /// Create a new empty repo map.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            files: HashMap::new(),
            ranks: HashMap::new(),
            version: Self::VERSION,
        }
    }

    /// Add a file to the map.
    pub fn add_file(&mut self, file: FileInfo) {
        self.files.insert(file.path.clone(), file);
    }

    /// Get a file by path.
    pub fn get_file(&self, path: &std::path::Path) -> Option<&FileInfo> {
        self.files.get(path)
    }

    /// Get files sorted by rank (highest first).
    pub fn files_by_rank(&self) -> Vec<&FileInfo> {
        let mut files: Vec<_> = self.files.values().collect();
        files.sort_by(|a, b| {
            let rank_a = self.ranks.get(&a.path).unwrap_or(&0.0);
            let rank_b = self.ranks.get(&b.path).unwrap_or(&0.0);
            rank_b.partial_cmp(rank_a).unwrap_or(std::cmp::Ordering::Equal)
        });
        files
    }

    /// Get the rank of a file.
    pub fn get_rank(&self, path: &std::path::Path) -> f64 {
        self.ranks.get(path).copied().unwrap_or(0.0)
    }

    /// Total number of files.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Total number of symbols across all files.
    pub fn symbol_count(&self) -> usize {
        self.files.values().map(|f| f.symbols.len()).sum()
    }

    /// Check if the cache version is compatible.
    pub fn is_compatible(&self) -> bool {
        self.version == Self::VERSION
    }
}

/// Serde helper for SystemTime.
mod system_time_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    pub fn serialize<S>(time: &SystemTime, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let duration = time
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO);
        duration.as_secs().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<SystemTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = u64::deserialize(deserializer)?;
        Ok(UNIX_EPOCH + Duration::from_secs(secs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_detection() {
        assert_eq!(Language::from_extension("rs"), Language::Rust);
        assert_eq!(Language::from_extension("ts"), Language::TypeScript);
        assert_eq!(Language::from_extension("tsx"), Language::TypeScript);
        assert_eq!(Language::from_extension("py"), Language::Python);
        assert_eq!(Language::from_extension("go"), Language::Go);
        assert_eq!(Language::from_extension("java"), Language::Java);
        assert_eq!(Language::from_extension("txt"), Language::Unknown);
    }

    #[test]
    fn test_symbol_creation() {
        let sym = Symbol::new("my_function", SymbolKind::Function, 42)
            .with_signature("fn my_function(x: i32) -> bool");

        assert_eq!(sym.name, "my_function");
        assert_eq!(sym.kind, SymbolKind::Function);
        assert_eq!(sym.line, 42);
        assert!(sym.signature.is_some());
    }

    #[test]
    fn test_repo_map_files_by_rank() {
        let mut map = RepoMap::new("/test");

        map.add_file(FileInfo::new("a.rs", Language::Rust));
        map.add_file(FileInfo::new("b.rs", Language::Rust));
        map.add_file(FileInfo::new("c.rs", Language::Rust));

        map.ranks.insert(PathBuf::from("a.rs"), 0.5);
        map.ranks.insert(PathBuf::from("b.rs"), 0.8);
        map.ranks.insert(PathBuf::from("c.rs"), 0.3);

        let ranked = map.files_by_rank();
        assert_eq!(ranked[0].path, PathBuf::from("b.rs"));
        assert_eq!(ranked[1].path, PathBuf::from("a.rs"));
        assert_eq!(ranked[2].path, PathBuf::from("c.rs"));
    }
}
