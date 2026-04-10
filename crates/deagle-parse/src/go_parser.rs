//! Go language parser using tree-sitter-go.

use deagle_core::{DeagleError, EdgeKind, Language, Node, NodeKind, Result};
use std::path::Path;

use crate::ParseResult;

/// Parse a Go source file and extract definitions.
pub fn parse(path: &Path, content: &str) -> Result<Vec<Node>> {
    parse_with_edges(path, content).map(|r| r.nodes)
}

/// Parse with edge extraction.
pub fn parse_with_edges(path: &Path, content: &str) -> Result<ParseResult> {
    let mut parser = tree_sitter::Parser::new();
    let language = tree_sitter_go::LANGUAGE;
    parser.set_language(&language.into()).map_err(|e| DeagleError::Parse {
        file: path.display().to_string(),
        message: format!("Failed to set language: {}", e),
    })?;

    let tree = parser.parse(content, None).ok_or_else(|| DeagleError::Parse {
        file: path.display().to_string(),
        message: "Failed to parse file".into(),
    })?;

    let mut nodes = Vec::new();
    let file_path = path.to_string_lossy().to_string();

    nodes.push(Node {
        id: 0,
        name: path.file_name().and_then(|n| n.to_str()).unwrap_or("unknown").to_string(),
        kind: NodeKind::File,
        language: Language::Go,
        file_path: file_path.clone(),
        line_start: 1,
        line_end: content.lines().count() as u32,
        content: None,
    });

    extract_definitions(tree.root_node(), content, &file_path, &mut nodes);

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
        "function_declaration" => Some(NodeKind::Function),
        "method_declaration" => Some(NodeKind::Method),
        "type_declaration" => None, // handled below — contains type_spec children
        "type_spec" => {
            // Check if it's a struct, interface, or type alias
            if let Some(type_node) = node.child_by_field_name("type") {
                match type_node.kind() {
                    "struct_type" => Some(NodeKind::Struct),
                    "interface_type" => Some(NodeKind::Interface),
                    _ => Some(NodeKind::TypeAlias),
                }
            } else {
                None
            }
        }
        "import_declaration" => Some(NodeKind::Import),
        "const_declaration" | "var_declaration" => None, // extract individual specs
        "const_spec" => Some(NodeKind::Constant),
        "package_clause" => Some(NodeKind::Module),
        _ => None,
    };

    if let Some(kind) = kind {
        if let Some(name) = extract_name(node, source, kind) {
            let start = node.start_position();
            let end = node.end_position();
            let content = node.utf8_text(source.as_bytes()).ok().map(|s| {
                crate::truncate_content(s, 500)
            });

            results.push(Node {
                id: 0,
                name,
                kind,
                language: Language::Go,
                file_path: file_path.to_string(),
                line_start: (start.row + 1) as u32,
                line_end: (end.row + 1) as u32,
                content,
            });
        }
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_definitions(child, source, file_path, results);
    }
}

fn extract_name(node: tree_sitter::Node, source: &str, kind: NodeKind) -> Option<String> {
    match kind {
        NodeKind::Import => {
            node.utf8_text(source.as_bytes())
                .ok()
                .map(|s| s.trim().to_string())
        }
        NodeKind::Module => {
            // package clause: "package main"
            if let Some(n) = node.child_by_field_name("name") {
                return n.utf8_text(source.as_bytes()).ok().map(|s| s.to_string());
            }
            // Fallback: find package_identifier child
            let mut c = node.walk();
            let children: Vec<_> = node.children(&mut c).collect();
            children.iter()
                .find(|n| n.kind() == "package_identifier")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .map(|s| s.to_string())
        }
        _ => {
            node.child_by_field_name("name")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .map(|s| s.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const SAMPLE_GO: &str = r#"
package main

import (
    "fmt"
    "net/http"
)

const MaxSize = 1024

type Config struct {
    Name   string
    Values map[string]string
}

type Handler interface {
    ServeHTTP(w http.ResponseWriter, r *http.Request)
}

func NewConfig(name string) *Config {
    return &Config{Name: name, Values: make(map[string]string)}
}

func (c *Config) Get(key string) string {
    return c.Values[key]
}

func main() {
    config := NewConfig("test")
    fmt.Println(config.Get("key"))
}
"#;

    #[test]
    fn test_parse_go_finds_all_definitions() {
        let path = PathBuf::from("main.go");
        let nodes = parse(&path, SAMPLE_GO).unwrap();
        let kinds: Vec<_> = nodes.iter().map(|n| n.kind).collect();
        assert!(kinds.contains(&NodeKind::Module), "should find package");
        assert!(kinds.contains(&NodeKind::Import), "should find import");
        assert!(kinds.contains(&NodeKind::Constant), "should find const");
        assert!(kinds.contains(&NodeKind::Struct), "should find struct");
        assert!(kinds.contains(&NodeKind::Interface), "should find interface");
        assert!(kinds.contains(&NodeKind::Function), "should find function");
        assert!(kinds.contains(&NodeKind::Method), "should find method");
    }

    #[test]
    fn test_parse_go_struct_name() {
        let path = PathBuf::from("main.go");
        let nodes = parse(&path, SAMPLE_GO).unwrap();
        let structs: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Struct).collect();
        assert_eq!(structs.len(), 1);
        assert_eq!(structs[0].name, "Config");
        assert_eq!(structs[0].language, Language::Go);
    }

    #[test]
    fn test_parse_go_methods() {
        let path = PathBuf::from("main.go");
        let nodes = parse(&path, SAMPLE_GO).unwrap();
        let methods: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Method).collect();
        assert!(methods.iter().any(|m| m.name == "Get"), "should find Get method");
    }

    #[test]
    fn test_parse_go_functions() {
        let path = PathBuf::from("main.go");
        let nodes = parse(&path, SAMPLE_GO).unwrap();
        let fns: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Function).collect();
        assert!(fns.iter().any(|f| f.name == "NewConfig"));
        assert!(fns.iter().any(|f| f.name == "main"));
    }

    #[test]
    fn test_parse_go_interface() {
        let path = PathBuf::from("main.go");
        let nodes = parse(&path, SAMPLE_GO).unwrap();
        let ifaces: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Interface).collect();
        assert_eq!(ifaces.len(), 1);
        assert_eq!(ifaces[0].name, "Handler");
    }

    #[test]
    fn test_parse_go_edges() {
        let path = PathBuf::from("main.go");
        let result = parse_with_edges(&path, SAMPLE_GO).unwrap();
        assert!(!result.edges.is_empty());
        for &(from, _, ref kind) in &result.edges {
            assert_eq!(from, 0);
            assert_eq!(*kind, EdgeKind::Contains);
        }
    }

    #[test]
    fn test_parse_empty_go() {
        let path = PathBuf::from("empty.go");
        let nodes = parse(&path, "").unwrap();
        assert!(nodes.len() <= 1);
    }
}
