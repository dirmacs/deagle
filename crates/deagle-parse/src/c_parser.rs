//! C language parser using tree-sitter-c.

use deagle_core::{DeagleError, EdgeKind, Language, Node, NodeKind, Result};
use std::path::Path;
use crate::ParseResult;

pub fn parse(path: &Path, content: &str) -> Result<Vec<Node>> {
    parse_with_edges(path, content).map(|r| r.nodes)
}

pub fn parse_with_edges(path: &Path, content: &str) -> Result<ParseResult> {
    let mut parser = tree_sitter::Parser::new();
    let language = tree_sitter_c::LANGUAGE;
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
    let lang = if path.extension().and_then(|e| e.to_str()) == Some("h") {
        Language::C
    } else {
        Language::C
    };

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

    extract_definitions(tree.root_node(), content, &file_path, &mut nodes);

    let mut edges = Vec::new();
    for i in 1..nodes.len() {
        edges.push((0, i, EdgeKind::Contains));
    }
    Ok(ParseResult { nodes, edges })
}

fn extract_definitions(node: tree_sitter::Node, source: &str, file_path: &str, results: &mut Vec<Node>) {
    let kind = match node.kind() {
        "function_definition" => Some(NodeKind::Function),
        "declaration" => {
            // Check if it's a function declaration (prototype) or variable
            if node.child_by_field_name("declarator").map(|d| d.kind() == "function_declarator").unwrap_or(false) {
                Some(NodeKind::Function)
            } else {
                None
            }
        }
        "struct_specifier" => {
            if node.child_by_field_name("body").is_some() {
                Some(NodeKind::Struct)
            } else {
                None
            }
        }
        "enum_specifier" => {
            if node.child_by_field_name("body").is_some() {
                Some(NodeKind::Enum)
            } else {
                None
            }
        }
        "type_definition" => Some(NodeKind::TypeAlias),
        "preproc_include" => Some(NodeKind::Import),
        "preproc_def" => Some(NodeKind::Constant),
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
                id: 0, name, kind, language: Language::C,
                file_path: file_path.to_string(),
                line_start: (start.row + 1) as u32,
                line_end: (end.row + 1) as u32,
                content,
            });
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_definitions(child, source, file_path, results);
    }
}

fn extract_name(node: tree_sitter::Node, source: &str, kind: NodeKind) -> Option<String> {
    match kind {
        NodeKind::Import => node.utf8_text(source.as_bytes()).ok().map(|s| s.trim().to_string()),
        NodeKind::Constant => {
            // #define NAME ...
            node.child_by_field_name("name")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .map(|s| s.to_string())
        }
        NodeKind::Function => {
            // function_definition → declarator → function_declarator → declarator → identifier
            fn find_fn_name(n: tree_sitter::Node, src: &str) -> Option<String> {
                if n.kind() == "identifier" {
                    return n.utf8_text(src.as_bytes()).ok().map(|s| s.to_string());
                }
                if let Some(d) = n.child_by_field_name("declarator") {
                    return find_fn_name(d, src);
                }
                let mut c = n.walk();
                for child in n.children(&mut c) {
                    if let Some(name) = find_fn_name(child, src) {
                        return Some(name);
                    }
                }
                None
            }
            find_fn_name(node, source)
        }
        NodeKind::Struct | NodeKind::Enum => {
            node.child_by_field_name("name")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .map(|s| s.to_string())
        }
        NodeKind::TypeAlias => {
            // typedef ... name;  — last identifier before semicolon
            node.child_by_field_name("declarator")
                .and_then(|n| {
                    if n.kind() == "type_identifier" {
                        n.utf8_text(source.as_bytes()).ok().map(|s| s.to_string())
                    } else {
                        None
                    }
                })
        }
        _ => node.child_by_field_name("name")
            .and_then(|n| n.utf8_text(source.as_bytes()).ok())
            .map(|s| s.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const SAMPLE_C: &str = r#"
#include <stdio.h>
#include <stdlib.h>

#define MAX_SIZE 1024
#define VERSION "1.0"

typedef unsigned int uint;

struct Point {
    int x;
    int y;
};

enum Color {
    RED,
    GREEN,
    BLUE
};

int add(int a, int b) {
    return a + b;
}

void print_point(struct Point p) {
    printf("(%d, %d)\n", p.x, p.y);
}

int main(int argc, char *argv[]) {
    struct Point p = {1, 2};
    print_point(p);
    printf("%d\n", add(p.x, p.y));
    return 0;
}
"#;

    #[test]
    fn test_parse_c_finds_all() {
        let path = PathBuf::from("main.c");
        let nodes = parse(&path, SAMPLE_C).unwrap();
        let kinds: Vec<_> = nodes.iter().map(|n| n.kind).collect();
        assert!(kinds.contains(&NodeKind::Import), "should find #include");
        assert!(kinds.contains(&NodeKind::Constant), "should find #define");
        assert!(kinds.contains(&NodeKind::Struct), "should find struct");
        assert!(kinds.contains(&NodeKind::Enum), "should find enum");
        assert!(kinds.contains(&NodeKind::Function), "should find function");
    }

    #[test]
    fn test_parse_c_functions() {
        let path = PathBuf::from("main.c");
        let nodes = parse(&path, SAMPLE_C).unwrap();
        let fns: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Function).collect();
        assert!(fns.iter().any(|f| f.name == "add"));
        assert!(fns.iter().any(|f| f.name == "print_point"));
        assert!(fns.iter().any(|f| f.name == "main"));
    }

    #[test]
    fn test_parse_c_struct() {
        let path = PathBuf::from("main.c");
        let nodes = parse(&path, SAMPLE_C).unwrap();
        let structs: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Struct).collect();
        assert_eq!(structs.len(), 1);
        assert_eq!(structs[0].name, "Point");
    }

    #[test]
    fn test_parse_c_defines() {
        let path = PathBuf::from("main.c");
        let nodes = parse(&path, SAMPLE_C).unwrap();
        let consts: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Constant).collect();
        assert!(consts.iter().any(|c| c.name == "MAX_SIZE"));
        assert!(consts.iter().any(|c| c.name == "VERSION"));
    }

    #[test]
    fn test_parse_c_edges() {
        let path = PathBuf::from("main.c");
        let result = parse_with_edges(&path, SAMPLE_C).unwrap();
        assert!(!result.edges.is_empty());
    }

    #[test]
    fn test_parse_empty_c() {
        let path = PathBuf::from("empty.c");
        let nodes = parse(&path, "").unwrap();
        assert!(nodes.len() <= 1);
    }
}
