//! TypeScript/TSX language parser using tree-sitter-typescript.

use deagle_core::{DeagleError, EdgeKind, Language, Node, NodeKind, Result};
use std::path::Path;

use crate::ParseResult;

/// Parse a TypeScript source file and extract definitions.
pub fn parse(path: &Path, content: &str) -> Result<Vec<Node>> {
    parse_with_edges(path, content).map(|r| r.nodes)
}

/// Parse with edge extraction.
pub fn parse_with_edges(path: &Path, content: &str) -> Result<ParseResult> {
    let mut parser = tree_sitter::Parser::new();

    // Use TSX parser (superset of TS — handles both .ts and .tsx)
    let language = tree_sitter_typescript::LANGUAGE_TSX;
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
    let lang = Language::TypeScript;

    nodes.push(Node {
        id: 0,
        name: path.file_name().and_then(|n| n.to_str()).unwrap_or("unknown").to_string(),
        kind: NodeKind::File,
        language: lang,
        file_path: file_path.clone(),
        line_start: 1,
        line_end: content.lines().count() as u32,
        content: None,
    });

    extract_definitions(tree.root_node(), content, &file_path, lang, &mut nodes, false);

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
    lang: Language,
    results: &mut Vec<Node>,
    inside_class: bool,
) {
    let kind = match node.kind() {
        "function_declaration" => Some(NodeKind::Function),
        "method_definition" => Some(NodeKind::Method),
        "class_declaration" => Some(NodeKind::Class),
        "interface_declaration" => Some(NodeKind::Interface),
        "type_alias_declaration" => Some(NodeKind::TypeAlias),
        "enum_declaration" => Some(NodeKind::Enum),
        "import_statement" => Some(NodeKind::Import),
        "export_statement" => None, // recurse into children
        "lexical_declaration" => {
            // const/let/var — check for arrow functions or UPPER_CASE constants
            if !inside_class {
                extract_lexical(node, source, file_path, lang, results);
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
                crate::truncate_content(s, 500)
            });

            results.push(Node {
                id: 0,
                name,
                kind,
                language: lang,
                file_path: file_path.to_string(),
                line_start: (start.row + 1) as u32,
                line_end: (end.row + 1) as u32,
                content,
            });

            // Recurse into class body for methods
            if kind == NodeKind::Class {
                if let Some(body) = node.child_by_field_name("body") {
                    let mut cursor = body.walk();
                    for child in body.children(&mut cursor) {
                        extract_definitions(child, source, file_path, lang, results, true);
                    }
                }
                return;
            }
        }
    }

    // Recurse
    if node.kind() != "class_declaration" {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            extract_definitions(child, source, file_path, lang, results, inside_class);
        }
    }
}

fn extract_lexical(
    node: tree_sitter::Node,
    source: &str,
    file_path: &str,
    lang: Language,
    results: &mut Vec<Node>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            if let Some(name_node) = child.child_by_field_name("name") {
                let name = name_node.utf8_text(source.as_bytes()).unwrap_or_default().to_string();
                // Check if value is an arrow function
                let is_arrow = child.child_by_field_name("value")
                    .map(|v| v.kind() == "arrow_function")
                    .unwrap_or(false);

                let kind = if is_arrow {
                    NodeKind::Function
                } else if name.chars().all(|c| c.is_uppercase() || c == '_' || c.is_ascii_digit()) && !name.is_empty() {
                    NodeKind::Constant
                } else {
                    return; // skip regular variables
                };

                let start = node.start_position();
                let end = node.end_position();
                let content = node.utf8_text(source.as_bytes()).ok().map(|s| {
                    crate::truncate_content(s, 500)
                });

                results.push(Node {
                    id: 0,
                    name,
                    kind,
                    language: lang,
                    file_path: file_path.to_string(),
                    line_start: (start.row + 1) as u32,
                    line_end: (end.row + 1) as u32,
                    content,
                });
            }
        }
    }
}

fn extract_name(node: tree_sitter::Node, source: &str, kind: NodeKind) -> Option<String> {
    match kind {
        NodeKind::Import => {
            node.utf8_text(source.as_bytes())
                .ok()
                .map(|s| s.trim().to_string())
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

    const SAMPLE_TS: &str = r#"
import { Router } from 'express';
import type { Request, Response } from 'express';

const MAX_SIZE = 1024;

interface Config {
    name: string;
    values: Record<string, string>;
}

type Status = 'active' | 'inactive';

enum Direction {
    Up,
    Down,
    Left,
    Right,
}

class Server {
    private config: Config;

    constructor(config: Config) {
        this.config = config;
    }

    start(): void {
        console.log('starting');
    }

    getConfig(): Config {
        return this.config;
    }
}

function createServer(name: string): Server {
    return new Server({ name, values: {} });
}

const handler = (req: Request, res: Response) => {
    res.send('ok');
};

export function main() {
    const server = createServer('test');
    server.start();
}
"#;

    #[test]
    fn test_parse_ts_finds_all_definitions() {
        let path = PathBuf::from("app.ts");
        let nodes = parse(&path, SAMPLE_TS).unwrap();
        let kinds: Vec<_> = nodes.iter().map(|n| n.kind).collect();
        assert!(kinds.contains(&NodeKind::Import), "should find import");
        assert!(kinds.contains(&NodeKind::Constant), "should find constant");
        assert!(kinds.contains(&NodeKind::Interface), "should find interface");
        assert!(kinds.contains(&NodeKind::TypeAlias), "should find type alias");
        assert!(kinds.contains(&NodeKind::Enum), "should find enum");
        assert!(kinds.contains(&NodeKind::Class), "should find class");
        assert!(kinds.contains(&NodeKind::Function), "should find function");
    }

    #[test]
    fn test_parse_ts_class_methods() {
        let path = PathBuf::from("app.ts");
        let nodes = parse(&path, SAMPLE_TS).unwrap();
        let methods: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Method).collect();
        assert!(methods.iter().any(|m| m.name == "start"));
        assert!(methods.iter().any(|m| m.name == "getConfig"));
        assert!(methods.iter().any(|m| m.name == "constructor"));
    }

    #[test]
    fn test_parse_ts_arrow_function() {
        let path = PathBuf::from("app.ts");
        let nodes = parse(&path, SAMPLE_TS).unwrap();
        let fns: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Function).collect();
        assert!(fns.iter().any(|f| f.name == "handler"), "arrow function should be captured");
        assert!(fns.iter().any(|f| f.name == "createServer"));
        assert!(fns.iter().any(|f| f.name == "main"));
    }

    #[test]
    fn test_parse_ts_interface() {
        let path = PathBuf::from("app.ts");
        let nodes = parse(&path, SAMPLE_TS).unwrap();
        let ifaces: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Interface).collect();
        assert_eq!(ifaces.len(), 1);
        assert_eq!(ifaces[0].name, "Config");
    }

    #[test]
    fn test_parse_ts_enum() {
        let path = PathBuf::from("app.ts");
        let nodes = parse(&path, SAMPLE_TS).unwrap();
        let enums: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Enum).collect();
        assert_eq!(enums.len(), 1);
        assert_eq!(enums[0].name, "Direction");
    }

    #[test]
    fn test_parse_ts_edges() {
        let path = PathBuf::from("app.ts");
        let result = parse_with_edges(&path, SAMPLE_TS).unwrap();
        assert!(!result.edges.is_empty());
        for &(from, _, ref kind) in &result.edges {
            assert_eq!(from, 0);
            assert_eq!(*kind, EdgeKind::Contains);
        }
    }

    #[test]
    fn test_parse_empty_ts() {
        let path = PathBuf::from("empty.ts");
        let nodes = parse(&path, "").unwrap();
        assert!(nodes.len() <= 1);
    }
}
