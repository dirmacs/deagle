//! Java language parser using tree-sitter-java.

use deagle_core::{DeagleError, EdgeKind, Language, Node, NodeKind, Result};
use std::path::Path;

use crate::ParseResult;

pub fn parse(path: &Path, content: &str) -> Result<Vec<Node>> {
    parse_with_edges(path, content).map(|r| r.nodes)
}

pub fn parse_with_edges(path: &Path, content: &str) -> Result<ParseResult> {
    let mut parser = tree_sitter::Parser::new();
    let language = tree_sitter_java::LANGUAGE;
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
        language: Language::Java,
        file_path: file_path.clone(),
        line_start: 1,
        line_end: content.lines().count() as u32,
        content: None,
    });

    extract_definitions(tree.root_node(), content, &file_path, &mut nodes, false);

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
    inside_class: bool,
) {
    let kind = match node.kind() {
        "method_declaration" => Some(if inside_class { NodeKind::Method } else { NodeKind::Function }),
        "constructor_declaration" => Some(NodeKind::Method),
        "class_declaration" => Some(NodeKind::Class),
        "interface_declaration" => Some(NodeKind::Interface),
        "enum_declaration" => Some(NodeKind::Enum),
        "import_declaration" => Some(NodeKind::Import),
        "constant_declaration" => Some(NodeKind::Constant),
        "field_declaration" => {
            // Check for static final (constants)
            let text = node.utf8_text(source.as_bytes()).unwrap_or_default();
            if text.contains("static") && text.contains("final") {
                Some(NodeKind::Constant)
            } else {
                None
            }
        }
        "package_declaration" => Some(NodeKind::Module),
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
                language: Language::Java,
                file_path: file_path.to_string(),
                line_start: (start.row + 1) as u32,
                line_end: (end.row + 1) as u32,
                content,
            });

            if kind == NodeKind::Class || kind == NodeKind::Interface || kind == NodeKind::Enum {
                if let Some(body) = node.child_by_field_name("body") {
                    let mut cursor = body.walk();
                    for child in body.children(&mut cursor) {
                        extract_definitions(child, source, file_path, results, true);
                    }
                }
                return;
            }
        }
    }

    if !matches!(node.kind(), "class_declaration" | "interface_declaration" | "enum_declaration") {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            extract_definitions(child, source, file_path, results, inside_class);
        }
    }
}

fn extract_name(node: tree_sitter::Node, source: &str, kind: NodeKind) -> Option<String> {
    match kind {
        NodeKind::Import | NodeKind::Module => {
            node.utf8_text(source.as_bytes()).ok().map(|s| s.trim().to_string())
        }
        NodeKind::Constant => {
            // For field_declaration with static final, find the variable_declarator name
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "variable_declarator" {
                    if let Some(n) = child.child_by_field_name("name") {
                        return n.utf8_text(source.as_bytes()).ok().map(|s| s.to_string());
                    }
                }
            }
            node.child_by_field_name("name")
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

    const SAMPLE_JAVA: &str = r#"
package com.example.app;

import java.util.List;
import java.util.Map;

public class Application {
    public static final int MAX_SIZE = 1024;

    private String name;

    public Application(String name) {
        this.name = name;
    }

    public String getName() {
        return name;
    }

    public void process(List<String> items) {
        for (String item : items) {
            System.out.println(item);
        }
    }
}

interface Service {
    void execute();
    String status();
}

enum Priority {
    LOW, MEDIUM, HIGH, CRITICAL
}
"#;

    #[test]
    fn test_parse_java_finds_all() {
        let path = PathBuf::from("App.java");
        let nodes = parse(&path, SAMPLE_JAVA).unwrap();
        let kinds: Vec<_> = nodes.iter().map(|n| n.kind).collect();
        assert!(kinds.contains(&NodeKind::Module), "package");
        assert!(kinds.contains(&NodeKind::Import), "import");
        assert!(kinds.contains(&NodeKind::Class), "class");
        assert!(kinds.contains(&NodeKind::Interface), "interface");
        assert!(kinds.contains(&NodeKind::Enum), "enum");
        assert!(kinds.contains(&NodeKind::Method), "method");
    }

    #[test]
    fn test_parse_java_class() {
        let path = PathBuf::from("App.java");
        let nodes = parse(&path, SAMPLE_JAVA).unwrap();
        let classes: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Class).collect();
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "Application");
    }

    #[test]
    fn test_parse_java_methods() {
        let path = PathBuf::from("App.java");
        let nodes = parse(&path, SAMPLE_JAVA).unwrap();
        let methods: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Method).collect();
        assert!(methods.iter().any(|m| m.name == "getName"));
        assert!(methods.iter().any(|m| m.name == "process"));
    }

    #[test]
    fn test_parse_java_interface() {
        let path = PathBuf::from("App.java");
        let nodes = parse(&path, SAMPLE_JAVA).unwrap();
        let ifaces: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Interface).collect();
        assert_eq!(ifaces.len(), 1);
        assert_eq!(ifaces[0].name, "Service");
    }

    #[test]
    fn test_parse_java_enum() {
        let path = PathBuf::from("App.java");
        let nodes = parse(&path, SAMPLE_JAVA).unwrap();
        let enums: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Enum).collect();
        assert_eq!(enums.len(), 1);
        assert_eq!(enums[0].name, "Priority");
    }

    #[test]
    fn test_parse_java_edges() {
        let path = PathBuf::from("App.java");
        let result = parse_with_edges(&path, SAMPLE_JAVA).unwrap();
        assert!(!result.edges.is_empty());
    }

    #[test]
    fn test_parse_empty_java() {
        let path = PathBuf::from("Empty.java");
        let nodes = parse(&path, "").unwrap();
        assert!(nodes.len() <= 1);
    }
}
