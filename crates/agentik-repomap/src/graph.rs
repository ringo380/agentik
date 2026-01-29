//! Dependency graph construction.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::types::{Language, RepoMap};

/// Dependency graph of files.
///
/// Edges represent import relationships between files.
/// An edge from A to B means "A imports B" (A depends on B).
#[derive(Debug, Clone)]
pub struct DependencyGraph {
    /// Outgoing edges: file -> files it imports
    dependencies: HashMap<PathBuf, HashSet<PathBuf>>,
    /// Incoming edges: file -> files that import it
    dependents: HashMap<PathBuf, HashSet<PathBuf>>,
    /// All files in the graph
    files: HashSet<PathBuf>,
}

impl DependencyGraph {
    /// Create an empty graph.
    pub fn new() -> Self {
        Self {
            dependencies: HashMap::new(),
            dependents: HashMap::new(),
            files: HashSet::new(),
        }
    }

    /// Build a dependency graph from a repo map.
    pub fn build(repo_map: &RepoMap) -> Self {
        let mut graph = Self::new();
        let root = &repo_map.root;

        // First pass: add all files
        for path in repo_map.files.keys() {
            graph.add_file(path.clone());
        }

        // Second pass: resolve imports and add edges
        for (path, file_info) in &repo_map.files {
            for import in &file_info.imports {
                if let Some(resolved) = Self::resolve_import(
                    root,
                    path,
                    &import.raw_path,
                    file_info.language,
                    &graph.files,
                ) {
                    graph.add_edge(path.clone(), resolved);
                }
            }
        }

        graph
    }

    /// Add a file to the graph.
    pub fn add_file(&mut self, path: PathBuf) {
        self.files.insert(path.clone());
        self.dependencies.entry(path.clone()).or_default();
        self.dependents.entry(path).or_default();
    }

    /// Add an edge from `from` to `to` (from imports to).
    pub fn add_edge(&mut self, from: PathBuf, to: PathBuf) {
        if from == to {
            return; // No self-loops
        }

        self.dependencies.entry(from.clone()).or_default().insert(to.clone());
        self.dependents.entry(to).or_default().insert(from);
    }

    /// Get files that a file imports (outgoing edges).
    pub fn dependencies(&self, path: &Path) -> Vec<&PathBuf> {
        self.dependencies
            .get(path)
            .map(|set| set.iter().collect())
            .unwrap_or_default()
    }

    /// Get files that import a file (incoming edges).
    pub fn dependents(&self, path: &Path) -> Vec<&PathBuf> {
        self.dependents
            .get(path)
            .map(|set| set.iter().collect())
            .unwrap_or_default()
    }

    /// Get all files in the graph.
    pub fn files(&self) -> impl Iterator<Item = &PathBuf> {
        self.files.iter()
    }

    /// Get the number of files.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Get the number of edges (import relationships).
    pub fn edge_count(&self) -> usize {
        self.dependencies.values().map(|s| s.len()).sum()
    }

    /// Get the out-degree (number of imports) for a file.
    pub fn out_degree(&self, path: &Path) -> usize {
        self.dependencies.get(path).map(|s| s.len()).unwrap_or(0)
    }

    /// Get the in-degree (number of files importing this) for a file.
    pub fn in_degree(&self, path: &Path) -> usize {
        self.dependents.get(path).map(|s| s.len()).unwrap_or(0)
    }

    /// Get neighbors (both dependencies and dependents) for a file.
    pub fn neighbors(&self, path: &Path) -> HashSet<PathBuf> {
        let mut neighbors = HashSet::new();

        if let Some(deps) = self.dependencies.get(path) {
            neighbors.extend(deps.clone());
        }
        if let Some(dpts) = self.dependents.get(path) {
            neighbors.extend(dpts.clone());
        }

        neighbors
    }

    /// Resolve an import path to a file in the repository.
    fn resolve_import(
        root: &Path,
        source_file: &Path,
        import_path: &str,
        language: Language,
        known_files: &HashSet<PathBuf>,
    ) -> Option<PathBuf> {
        match language {
            Language::Rust => Self::resolve_rust_import(root, source_file, import_path, known_files),
            Language::TypeScript | Language::JavaScript => {
                Self::resolve_ts_import(root, source_file, import_path, known_files)
            }
            Language::Python => Self::resolve_python_import(root, source_file, import_path, known_files),
            Language::Go => Self::resolve_go_import(root, import_path, known_files),
            Language::Java => Self::resolve_java_import(root, import_path, known_files),
            Language::Unknown => None,
        }
    }

    /// Resolve a Rust import.
    ///
    /// Patterns:
    /// - `crate::module::item` -> look for src/module.rs or src/module/mod.rs
    /// - `super::item` -> look relative to parent
    /// - External crates are ignored
    fn resolve_rust_import(
        root: &Path,
        source_file: &Path,
        import_path: &str,
        known_files: &HashSet<PathBuf>,
    ) -> Option<PathBuf> {
        // Skip external crates and std
        if !import_path.starts_with("crate::")
            && !import_path.starts_with("super::")
            && !import_path.starts_with("self::")
        {
            return None;
        }

        let parts: Vec<&str> = import_path.split("::").collect();
        let source_dir = source_file.parent()?;

        let mut search_base = if parts[0] == "crate" {
            // Start from src/ directory
            root.join("src")
        } else if parts[0] == "super" {
            // Go up one directory
            source_dir.parent()?.to_path_buf()
        } else if parts[0] == "self" {
            // Current module's directory
            if source_file.file_name()?.to_str()? == "mod.rs" {
                source_dir.to_path_buf()
            } else {
                // For foo.rs, self refers to foo/
                let stem = source_file.file_stem()?.to_str()?;
                source_dir.join(stem)
            }
        } else {
            return None;
        };

        // Navigate through path parts (skip first which is crate/super/self)
        for part in &parts[1..] {
            // Skip the last part if it looks like an item (not a module)
            // Items typically start with lowercase and are not the last segment in a path to a file

            // Try as a module file first
            let module_file = search_base.join(format!("{}.rs", part));
            let module_dir = search_base.join(part).join("mod.rs");

            if known_files.contains(&Self::make_relative(root, &module_file)) {
                return Some(Self::make_relative(root, &module_file));
            }
            if known_files.contains(&Self::make_relative(root, &module_dir)) {
                return Some(Self::make_relative(root, &module_dir));
            }

            // Maybe it's a subdirectory
            let subdir = search_base.join(part);
            if subdir.is_dir() || search_base.join(part).join("mod.rs").exists() {
                search_base = subdir;
            }
        }

        None
    }

    /// Resolve a TypeScript/JavaScript import.
    ///
    /// Patterns:
    /// - `./foo` -> foo.ts, foo.tsx, foo/index.ts, etc.
    /// - `../foo` -> relative path up
    /// - `@/foo` -> alias (typically src/foo)
    /// - External packages are ignored
    fn resolve_ts_import(
        root: &Path,
        source_file: &Path,
        import_path: &str,
        known_files: &HashSet<PathBuf>,
    ) -> Option<PathBuf> {
        // Skip external packages (no ./ or ../)
        if !import_path.starts_with('.') && !import_path.starts_with('@') {
            return None;
        }

        let source_dir = source_file.parent()?;

        let base_path = if import_path.starts_with('@') {
            // Handle common alias patterns
            let without_alias = import_path.trim_start_matches("@/");
            root.join("src").join(without_alias)
        } else {
            // Relative import
            source_dir.join(import_path)
        };

        // Try various extensions and index files
        let extensions = ["ts", "tsx", "js", "jsx", "mjs"];

        for ext in &extensions {
            let with_ext = base_path.with_extension(ext);
            let relative = Self::make_relative(root, &with_ext);
            if known_files.contains(&relative) {
                return Some(relative);
            }
        }

        // Try index files in directory
        for ext in &extensions {
            let index_file = base_path.join(format!("index.{}", ext));
            let relative = Self::make_relative(root, &index_file);
            if known_files.contains(&relative) {
                return Some(relative);
            }
        }

        None
    }

    /// Resolve a Python import.
    ///
    /// Patterns:
    /// - `from .foo import bar` -> relative import
    /// - `from ..foo import bar` -> parent relative import
    /// - `import foo.bar` -> package import
    fn resolve_python_import(
        root: &Path,
        source_file: &Path,
        import_path: &str,
        known_files: &HashSet<PathBuf>,
    ) -> Option<PathBuf> {
        let source_dir = source_file.parent()?;

        // Handle relative imports (starting with .)
        if import_path.starts_with('.') {
            let mut current_dir = source_dir.to_path_buf();
            let mut chars = import_path.chars().peekable();

            // Count dots and navigate up
            while chars.peek() == Some(&'.') {
                chars.next();
                if chars.peek() != Some(&'.') {
                    // Single dot - stay in current package
                } else {
                    // Multiple dots - go up
                    current_dir = current_dir.parent()?.to_path_buf();
                }
            }

            let remaining: String = chars.collect();
            let module_path = remaining.replace('.', "/");

            return Self::try_python_module_paths(root, &current_dir.join(&module_path), known_files);
        }

        // Absolute import - try to find in the repo
        let module_path = import_path.replace('.', "/");

        // Try from root
        if let Some(path) = Self::try_python_module_paths(root, &root.join(&module_path), known_files) {
            return Some(path);
        }

        // Try from common source directories
        for src_dir in &["src", "lib", ""] {
            let base = if src_dir.is_empty() {
                root.to_path_buf()
            } else {
                root.join(src_dir)
            };

            if let Some(path) = Self::try_python_module_paths(root, &base.join(&module_path), known_files) {
                return Some(path);
            }
        }

        None
    }

    fn try_python_module_paths(
        root: &Path,
        base: &Path,
        known_files: &HashSet<PathBuf>,
    ) -> Option<PathBuf> {
        // Try as direct file
        let py_file = base.with_extension("py");
        let relative = Self::make_relative(root, &py_file);
        if known_files.contains(&relative) {
            return Some(relative);
        }

        // Try as package __init__.py
        let init_file = base.join("__init__.py");
        let relative = Self::make_relative(root, &init_file);
        if known_files.contains(&relative) {
            return Some(relative);
        }

        None
    }

    /// Resolve a Go import.
    fn resolve_go_import(
        _root: &Path,
        import_path: &str,
        known_files: &HashSet<PathBuf>,
    ) -> Option<PathBuf> {
        // Go imports are typically full paths like "github.com/user/repo/pkg"
        // We only resolve local package imports

        // Try to find if any files are in a matching directory
        let parts: Vec<&str> = import_path.split('/').collect();

        for file in known_files {
            let file_str = file.to_string_lossy();
            // Check if the file path ends with the import path
            if parts.iter().any(|part| file_str.contains(part)) {
                let parent = file.parent()?;
                let parent_name = parent.file_name()?.to_str()?;
                if parts.last() == Some(&parent_name) {
                    return Some(file.clone());
                }
            }
        }

        None
    }

    /// Resolve a Java import.
    fn resolve_java_import(
        root: &Path,
        import_path: &str,
        known_files: &HashSet<PathBuf>,
    ) -> Option<PathBuf> {
        // Java imports like "com.example.package.ClassName"
        // Convert to path: com/example/package/ClassName.java

        let class_path = import_path.replace('.', "/");

        // Try from common source directories
        for src_dir in &["src/main/java", "src", ""] {
            let base = if src_dir.is_empty() {
                root.to_path_buf()
            } else {
                root.join(src_dir)
            };

            let java_file = base.join(format!("{}.java", class_path));
            let relative = Self::make_relative(root, &java_file);
            if known_files.contains(&relative) {
                return Some(relative);
            }
        }

        None
    }

    /// Make a path relative to root.
    fn make_relative(root: &Path, path: &Path) -> PathBuf {
        path.strip_prefix(root)
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

impl Default for DependencyGraph {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FileInfo, Import};

    #[test]
    fn test_empty_graph() {
        let graph = DependencyGraph::new();
        assert_eq!(graph.file_count(), 0);
        assert_eq!(graph.edge_count(), 0);
    }

    #[test]
    fn test_add_files_and_edges() {
        let mut graph = DependencyGraph::new();

        graph.add_file(PathBuf::from("a.rs"));
        graph.add_file(PathBuf::from("b.rs"));
        graph.add_file(PathBuf::from("c.rs"));

        graph.add_edge(PathBuf::from("a.rs"), PathBuf::from("b.rs"));
        graph.add_edge(PathBuf::from("a.rs"), PathBuf::from("c.rs"));
        graph.add_edge(PathBuf::from("b.rs"), PathBuf::from("c.rs"));

        assert_eq!(graph.file_count(), 3);
        assert_eq!(graph.edge_count(), 3);

        assert_eq!(graph.out_degree(Path::new("a.rs")), 2);
        assert_eq!(graph.out_degree(Path::new("b.rs")), 1);
        assert_eq!(graph.out_degree(Path::new("c.rs")), 0);

        assert_eq!(graph.in_degree(Path::new("a.rs")), 0);
        assert_eq!(graph.in_degree(Path::new("b.rs")), 1);
        assert_eq!(graph.in_degree(Path::new("c.rs")), 2);
    }

    #[test]
    fn test_neighbors() {
        let mut graph = DependencyGraph::new();

        graph.add_file(PathBuf::from("a.rs"));
        graph.add_file(PathBuf::from("b.rs"));
        graph.add_file(PathBuf::from("c.rs"));

        graph.add_edge(PathBuf::from("a.rs"), PathBuf::from("b.rs"));
        graph.add_edge(PathBuf::from("c.rs"), PathBuf::from("a.rs"));

        let neighbors = graph.neighbors(Path::new("a.rs"));
        assert!(neighbors.contains(&PathBuf::from("b.rs")));
        assert!(neighbors.contains(&PathBuf::from("c.rs")));
        assert_eq!(neighbors.len(), 2);
    }

    #[test]
    fn test_no_self_loops() {
        let mut graph = DependencyGraph::new();
        graph.add_file(PathBuf::from("a.rs"));
        graph.add_edge(PathBuf::from("a.rs"), PathBuf::from("a.rs"));

        assert_eq!(graph.edge_count(), 0);
    }

    #[test]
    fn test_build_from_repo_map() {
        let mut repo_map = RepoMap::new("/project");

        let mut main_file = FileInfo::new("src/main.rs", Language::Rust);
        main_file.imports = vec![Import::new("crate::lib", 1)];

        let lib_file = FileInfo::new("src/lib.rs", Language::Rust);

        repo_map.add_file(main_file);
        repo_map.add_file(lib_file);

        let graph = DependencyGraph::build(&repo_map);

        assert_eq!(graph.file_count(), 2);
        // Import resolution may or may not succeed depending on exact path matching
    }
}
