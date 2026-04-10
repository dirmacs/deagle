//! Python language parser using tree-sitter-python.

use deagle_core::{DeagleError, EdgeKind, Language, Node, NodeKind, Result};
use std::path::Path;

use crate::ParseResult;

/// Parse a Python source file and extract definitions.
pub fn parse(path: &Path, content: &str) -> Result<Vec<Node>> {
    parse_with_edges(path, content).map(|r| r.nodes)
}

/// Parse with edge extraction — returns nodes and relationship tuples.
pub fn parse_with_edges(path: &Path, content: &str) -> Result<ParseResult> {
    let mut parser = tree_sitter::Parser::new();
    let language = tree_sitter_python::LANGUAGE;
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

    // Insert file node as index 0
    nodes.push(Node {
        id: 0,
        name: path.file_name().and_then(|n| n.to_str()).unwrap_or("unknown").to_string(),
        kind: NodeKind::File,
        language: Language::Python,
        file_path: file_path.clone(),
        line_start: 1,
        line_end: content.lines().count() as u32,
        content: None,
    });

    extract_definitions(tree.root_node(), content, &file_path, &mut nodes, false);

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
    inside_class: bool,
) {
    let kind = match node.kind() {
        "function_definition" => {
            if inside_class {
                Some(NodeKind::Method)
            } else {
                Some(NodeKind::Function)
            }
        }
        "class_definition" => Some(NodeKind::Class),
        "import_statement" | "import_from_statement" => Some(NodeKind::Import),
        "global_statement" => None, // skip
        "expression_statement" => {
            // Check for top-level assignments (module-level constants)
            if !inside_class {
                if let Some(child) = node.child(0) {
                    if child.kind() == "assignment" {
                        // Only capture UPPER_CASE assignments as constants
                        if let Some(name) = extract_assignment_name(child, source) {
                            if name.chars().all(|c| c.is_uppercase() || c == '_' || c.is_ascii_digit()) && !name.is_empty() {
                                let start = node.start_position();
                                let end = node.end_position();
                                let content = node.utf8_text(source.as_bytes()).ok().map(|s| {
                                    if s.len() > 500 { format!("{}...", &s[..500]) } else { s.to_string() }
                                });
                                results.push(Node {
                                    id: 0,
                                    name,
                                    kind: NodeKind::Constant,
                                    language: Language::Python,
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
            None
        }
        _ => None,
    };

    if let Some(kind) = kind {
        if let Some(name) = extract_name(node, source, kind) {
            let start = node.start_position();
            let end = node.end_position();
            let content = node.utf8_text(source.as_bytes()).ok().map(|s| {
                if s.len() > 500 { format!("{}...", &s[..500]) } else { s.to_string() }
            });

            results.push(Node {
                id: 0,
                name,
                kind,
                language: Language::Python,
                file_path: file_path.to_string(),
                line_start: (start.row + 1) as u32,
                line_end: (end.row + 1) as u32,
                content,
            });
        }

        // If this is a class, recurse into its body to find methods
        if kind == NodeKind::Class {
            if let Some(body) = node.child_by_field_name("body") {
                let mut cursor = body.walk();
                for child in body.children(&mut cursor) {
                    extract_definitions(child, source, file_path, results, true);
                }
            }
            return; // Don't double-recurse into class children
        }
    }

    // Recurse into children (but not into class bodies — handled above)
    if node.kind() != "class_definition" {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            extract_definitions(child, source, file_path, results, inside_class);
        }
    }
}

fn extract_name(node: tree_sitter::Node, source: &str, kind: NodeKind) -> Option<String> {
    match kind {
        NodeKind::Import => {
            // Full import text
            node.utf8_text(source.as_bytes())
                .ok()
                .map(|s| s.trim().to_string())
        }
        _ => {
            // Find the 'name' field (tree-sitter-python uses field names)
            node.child_by_field_name("name")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .map(|s| s.to_string())
        }
    }
}

fn extract_assignment_name(node: tree_sitter::Node, source: &str) -> Option<String> {
    // Assignment left side — could be identifier or pattern
    node.child_by_field_name("left")
        .and_then(|n| {
            if n.kind() == "identifier" {
                n.utf8_text(source.as_bytes()).ok().map(|s| s.to_string())
            } else {
                None
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const SAMPLE_PYTHON: &str = r#"
import os
from pathlib import Path

MAX_SIZE = 1024
DEBUG = True

class Config:
    """Configuration holder."""

    def __init__(self, name: str):
        self.name = name
        self.values = {}

    def get(self, key: str) -> str:
        return self.values.get(key, "")

    @staticmethod
    def default() -> "Config":
        return Config("default")

class Status:
    ACTIVE = "active"
    INACTIVE = "inactive"

def process(data: list) -> dict:
    result = {}
    for item in data:
        result[item] = True
    return result

def main():
    config = Config("test")
    print(config.get("key"))
"#;

    #[test]
    fn test_parse_python_finds_all_definitions() {
        let path = PathBuf::from("test.py");
        let nodes = parse(&path, SAMPLE_PYTHON).unwrap();

        let kinds: Vec<_> = nodes.iter().map(|n| n.kind).collect();
        assert!(kinds.contains(&NodeKind::Import), "should find import");
        assert!(kinds.contains(&NodeKind::Constant), "should find constant");
        assert!(kinds.contains(&NodeKind::Class), "should find class");
        assert!(kinds.contains(&NodeKind::Function), "should find function");
    }

    #[test]
    fn test_parse_python_finds_methods() {
        let path = PathBuf::from("test.py");
        let nodes = parse(&path, SAMPLE_PYTHON).unwrap();

        let methods: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Method).collect();
        assert!(methods.len() >= 3, "should find methods (__init__, get, default), got {}", methods.len());
        assert!(methods.iter().any(|m| m.name == "__init__"));
        assert!(methods.iter().any(|m| m.name == "get"));
        assert!(methods.iter().any(|m| m.name == "default"));
    }

    #[test]
    fn test_parse_python_class_name() {
        let path = PathBuf::from("test.py");
        let nodes = parse(&path, SAMPLE_PYTHON).unwrap();

        let classes: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Class).collect();
        assert_eq!(classes.len(), 2);
        assert!(classes.iter().any(|c| c.name == "Config"));
        assert!(classes.iter().any(|c| c.name == "Status"));
        assert_eq!(classes[0].language, Language::Python);
    }

    #[test]
    fn test_parse_python_constants() {
        let path = PathBuf::from("test.py");
        let nodes = parse(&path, SAMPLE_PYTHON).unwrap();

        let constants: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Constant).collect();
        assert!(constants.iter().any(|c| c.name == "MAX_SIZE"), "should find MAX_SIZE");
        assert!(constants.iter().any(|c| c.name == "DEBUG"), "should find DEBUG");
    }

    #[test]
    fn test_parse_python_line_numbers() {
        let path = PathBuf::from("test.py");
        let nodes = parse(&path, SAMPLE_PYTHON).unwrap();

        let main_fn = nodes.iter().find(|n| n.name == "main" && n.kind == NodeKind::Function);
        assert!(main_fn.is_some(), "should find main function");
        assert!(main_fn.unwrap().line_start > 0, "line numbers should be 1-indexed");
    }

    #[test]
    fn test_parse_python_imports() {
        let path = PathBuf::from("test.py");
        let nodes = parse(&path, SAMPLE_PYTHON).unwrap();

        let imports: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Import).collect();
        assert_eq!(imports.len(), 2, "should find 2 import statements");
        assert!(imports.iter().any(|i| i.name.contains("os")));
        assert!(imports.iter().any(|i| i.name.contains("pathlib")));
    }

    #[test]
    fn test_parse_python_edges() {
        let path = PathBuf::from("test.py");
        let result = parse_with_edges(&path, SAMPLE_PYTHON).unwrap();

        assert!(!result.edges.is_empty(), "should have CONTAINS edges");
        // All edges should be from file node (idx 0)
        for &(from_idx, _, ref kind) in &result.edges {
            assert_eq!(from_idx, 0);
            assert_eq!(*kind, EdgeKind::Contains);
        }
    }

    #[test]
    fn test_parse_empty_python_file() {
        let path = PathBuf::from("empty.py");
        let nodes = parse(&path, "").unwrap();
        assert!(nodes.len() <= 1);
    }

    #[test]
    fn test_parse_python_decorated_function() {
        let source = r#"
import functools

def decorator(f):
    return f

@decorator
def decorated():
    pass

class MyClass:
    @staticmethod
    def static_method():
        pass

    @classmethod
    def class_method(cls):
        pass
"#;
        let path = PathBuf::from("deco.py");
        let nodes = parse(&path, source).unwrap();

        let fns: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Function).collect();
        assert!(fns.iter().any(|f| f.name == "decorator"));
        assert!(fns.iter().any(|f| f.name == "decorated"));

        let methods: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Method).collect();
        assert!(methods.iter().any(|m| m.name == "static_method"));
        assert!(methods.iter().any(|m| m.name == "class_method"));
    }

    #[test]
    fn test_parse_python_nested_class() {
        let source = r#"
class Outer:
    class Inner:
        def inner_method(self):
            pass

    def outer_method(self):
        pass
"#;
        let path = PathBuf::from("nested.py");
        let nodes = parse(&path, source).unwrap();

        let classes: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Class).collect();
        assert!(classes.iter().any(|c| c.name == "Outer"));
    }

    #[test]
    fn test_parse_python_async_function() {
        let source = r#"
import asyncio

async def fetch_data(url: str) -> dict:
    return {}

class Client:
    async def connect(self):
        pass
"#;
        let path = PathBuf::from("async.py");
        let nodes = parse(&path, source).unwrap();

        let fns: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Function).collect();
        assert!(fns.iter().any(|f| f.name == "fetch_data"), "should find async function");

        let methods: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Method).collect();
        assert!(methods.iter().any(|m| m.name == "connect"), "should find async method");
    }

    #[test]
    fn test_parse_python_lowercase_not_constant() {
        let source = r#"
MAX_SIZE = 100
lowercase_var = "not a constant"
_private = True
"#;
        let path = PathBuf::from("vars.py");
        let nodes = parse(&path, source).unwrap();

        let constants: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Constant).collect();
        assert!(constants.iter().any(|c| c.name == "MAX_SIZE"));
        // lowercase should NOT be captured as constant
        assert!(!constants.iter().any(|c| c.name == "lowercase_var"));
        assert!(!constants.iter().any(|c| c.name == "_private"));
    }

    #[test]
    fn test_parse_python_multiple_imports() {
        let source = r#"
import os
import sys
from typing import Dict, List, Optional
from pathlib import Path
from collections import defaultdict
"#;
        let path = PathBuf::from("imports.py");
        let nodes = parse(&path, source).unwrap();

        let imports: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Import).collect();
        assert_eq!(imports.len(), 5, "should find all 5 import statements");
    }
}
