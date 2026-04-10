//! Rust language parser using tree-sitter-rust.

use deagle_core::{DeagleError, EdgeKind, Language, Node, NodeKind, Result};
use std::path::Path;

/// Result of parsing a file — nodes and their relationships.
pub struct ParseResult {
    pub nodes: Vec<Node>,
    pub edges: Vec<(usize, usize, EdgeKind)>, // (from_idx, to_idx, kind) — indexes into nodes vec
}

/// Parse a Rust source file and extract definitions + relationships.
pub fn parse(path: &Path, content: &str) -> Result<Vec<Node>> {
    parse_with_edges(path, content).map(|r| r.nodes)
}

/// Parse with edge extraction — returns nodes and relationship tuples.
pub fn parse_with_edges(path: &Path, content: &str) -> Result<ParseResult> {
    let mut parser = tree_sitter::Parser::new();
    let language = tree_sitter_rust::LANGUAGE;
    parser.set_language(&language.into()).map_err(|e| {
        DeagleError::Parse {
            file: path.display().to_string(),
            message: format!("Failed to set language: {}", e),
        }
    })?;

    let tree = parser.parse(content, None).ok_or_else(|| DeagleError::Parse {
        file: path.display().to_string(),
        message: "Failed to parse file".into(),
    })?;

    let mut nodes = Vec::new();
    let file_path = path.to_string_lossy().to_string();

    // Insert file node as index 0
    nodes.push(Node {
        id: 0,
        name: path.file_name().and_then(|n| n.to_str()).unwrap_or("unknown").to_string(),
        kind: NodeKind::File,
        language: Language::Rust,
        file_path: file_path.clone(),
        line_start: 1,
        line_end: content.lines().count() as u32,
        content: None,
    });

    extract_definitions(tree.root_node(), content, &file_path, &mut nodes);

    // Build CONTAINS edges: file (idx 0) → each top-level entity
    let mut edges = Vec::new();
    for i in 1..nodes.len() {
        edges.push((0, i, EdgeKind::Contains));
    }

    Ok(ParseResult { nodes, edges })
}

fn extract_definitions(
    node: tree_sitter::Node,
    source: &str,
    file_path: &str,
    results: &mut Vec<Node>,
) {
    let kind = match node.kind() {
        "function_item" => Some(NodeKind::Function),
        "struct_item" => Some(NodeKind::Struct),
        "enum_item" => Some(NodeKind::Enum),
        "trait_item" => Some(NodeKind::Trait),
        "impl_item" => None, // We extract methods inside
        "const_item" | "static_item" => Some(NodeKind::Constant),
        "type_item" => Some(NodeKind::TypeAlias),
        "mod_item" => Some(NodeKind::Module),
        "use_declaration" => Some(NodeKind::Import),
        _ => None,
    };

    if let Some(kind) = kind {
        if let Some(name) = extract_name(node, source, kind) {
            let start = node.start_position();
            let end = node.end_position();
            let content = node.utf8_text(source.as_bytes()).ok().map(|s| {
                // Truncate long content
                crate::truncate_content(s, 500)
            });

            results.push(Node {
                id: 0,
                name,
                kind,
                language: Language::Rust,
                file_path: file_path.to_string(),
                line_start: (start.row + 1) as u32,
                line_end: (end.row + 1) as u32,
                content,
            });
        }
    }

    // Recurse into children — extract methods from impl blocks
    if node.kind() == "impl_item" {
        extract_impl_methods(node, source, file_path, results);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "impl_item" || node.kind() != "impl_item" {
            extract_definitions(child, source, file_path, results);
        }
    }
}

fn extract_impl_methods(
    impl_node: tree_sitter::Node,
    source: &str,
    file_path: &str,
    results: &mut Vec<Node>,
) {
    let mut cursor = impl_node.walk();
    for child in impl_node.children(&mut cursor) {
        if child.kind() == "declaration_list" {
            let mut inner = child.walk();
            for item in child.children(&mut inner) {
                if item.kind() == "function_item" {
                    if let Some(name) = extract_name(item, source, NodeKind::Method) {
                        let start = item.start_position();
                        let end = item.end_position();
                        let content = item.utf8_text(source.as_bytes()).ok().map(|s| {
                            crate::truncate_content(s, 500)
                        });
                        results.push(Node {
                            id: 0,
                            name,
                            kind: NodeKind::Method,
                            language: Language::Rust,
                            file_path: file_path.to_string(),
                            line_start: (start.row + 1) as u32,
                            line_end: (end.row + 1) as u32,
                            content,
                        });
                    }
                }
            }
        }
    }
}

fn extract_name(node: tree_sitter::Node, source: &str, kind: NodeKind) -> Option<String> {
    match kind {
        NodeKind::Import => {
            // For use declarations, take the full text
            node.utf8_text(source.as_bytes())
                .ok()
                .map(|s| s.trim().to_string())
        }
        _ => {
            // Find the name/identifier child
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" || child.kind() == "type_identifier" {
                    return child.utf8_text(source.as_bytes())
                        .ok()
                        .map(|s| s.to_string());
                }
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const SAMPLE_RUST: &str = r#"
use std::collections::HashMap;

const MAX_SIZE: usize = 1024;

pub struct Config {
    name: String,
    values: HashMap<String, String>,
}

pub enum Status {
    Active,
    Inactive,
}

pub trait Processor {
    fn process(&self, input: &str) -> String;
}

impl Config {
    pub fn new(name: &str) -> Self {
        Self { name: name.to_string(), values: HashMap::new() }
    }

    pub fn get(&self, key: &str) -> Option<&String> {
        self.values.get(key)
    }
}

pub fn main() {
    let config = Config::new("test");
    println!("{:?}", config.get("key"));
}
"#;

    #[test]
    fn test_parse_rust_finds_all_definitions() {
        let path = PathBuf::from("test.rs");
        let nodes = parse(&path, SAMPLE_RUST).unwrap();

        let kinds: Vec<_> = nodes.iter().map(|n| n.kind).collect();
        assert!(kinds.contains(&NodeKind::Import), "should find use declaration");
        assert!(kinds.contains(&NodeKind::Constant), "should find const");
        assert!(kinds.contains(&NodeKind::Struct), "should find struct");
        assert!(kinds.contains(&NodeKind::Enum), "should find enum");
        assert!(kinds.contains(&NodeKind::Trait), "should find trait");
        assert!(kinds.contains(&NodeKind::Function), "should find function");
    }

    #[test]
    fn test_parse_rust_finds_methods() {
        let path = PathBuf::from("test.rs");
        let nodes = parse(&path, SAMPLE_RUST).unwrap();

        let methods: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Method).collect();
        assert!(methods.len() >= 2, "should find impl methods (new, get), got {}", methods.len());
        assert!(methods.iter().any(|m| m.name == "new"));
        assert!(methods.iter().any(|m| m.name == "get"));
    }

    #[test]
    fn test_parse_rust_struct_name() {
        let path = PathBuf::from("test.rs");
        let nodes = parse(&path, SAMPLE_RUST).unwrap();

        let structs: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Struct).collect();
        assert_eq!(structs.len(), 1);
        assert_eq!(structs[0].name, "Config");
        assert_eq!(structs[0].language, Language::Rust);
    }

    #[test]
    fn test_parse_rust_line_numbers() {
        let path = PathBuf::from("test.rs");
        let nodes = parse(&path, SAMPLE_RUST).unwrap();

        let main_fn = nodes.iter().find(|n| n.name == "main" && n.kind == NodeKind::Function);
        assert!(main_fn.is_some(), "should find main function");
        let main_fn = main_fn.unwrap();
        assert!(main_fn.line_start > 0, "line numbers should be 1-indexed");
    }

    #[test]
    fn test_parse_empty_file() {
        let path = PathBuf::from("empty.rs");
        let nodes = parse(&path, "").unwrap();
        // File node is always created; empty file has just the file node
        assert!(nodes.len() <= 1);
    }

    #[test]
    fn test_parse_rust_trait_method() {
        let path = PathBuf::from("test.rs");
        let nodes = parse(&path, SAMPLE_RUST).unwrap();

        let traits: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Trait).collect();
        assert_eq!(traits.len(), 1);
        assert_eq!(traits[0].name, "Processor");
    }
}
