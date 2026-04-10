//! C++ language parser using tree-sitter-cpp.

use deagle_core::{DeagleError, EdgeKind, Language, Node, NodeKind, Result};
use std::path::Path;
use crate::ParseResult;

pub fn parse(path: &Path, content: &str) -> Result<Vec<Node>> {
    parse_with_edges(path, content).map(|r| r.nodes)
}

pub fn parse_with_edges(path: &Path, content: &str) -> Result<ParseResult> {
    let mut parser = tree_sitter::Parser::new();
    let language = tree_sitter_cpp::LANGUAGE;
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
        language: Language::Cpp,
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
            if node.child_by_field_name("declarator")
                .map(|d| d.kind() == "function_declarator")
                .unwrap_or(false)
            {
                Some(NodeKind::Function)
            } else {
                None
            }
        }
        "class_specifier" => {
            if node.child_by_field_name("body").is_some() {
                Some(NodeKind::Class)
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
        "namespace_definition" => Some(NodeKind::Module),
        "type_definition" => Some(NodeKind::TypeAlias),
        "preproc_include" => Some(NodeKind::Import),
        "preproc_def" => Some(NodeKind::Constant),
        "template_declaration" => {
            // Look inside for class or function
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "class_specifier" | "struct_specifier" => return extract_template(node, child, source, file_path, results, NodeKind::Class),
                    "function_definition" => return extract_template(node, child, source, file_path, results, NodeKind::Function),
                    "declaration" => return extract_template(node, child, source, file_path, results, NodeKind::Function),
                    _ => {}
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
                crate::truncate_content(s, 500)
            });
            results.push(Node {
                id: 0, name, kind, language: Language::Cpp,
                file_path: file_path.to_string(),
                line_start: (start.row + 1) as u32,
                line_end: (end.row + 1) as u32,
                content,
            });
        }
    }

    // Recurse into children (but skip template_declaration children since handled above)
    if node.kind() != "template_declaration" {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            extract_definitions(child, source, file_path, results);
        }
    }
}

fn extract_template(
    template_node: tree_sitter::Node,
    inner_node: tree_sitter::Node,
    source: &str,
    file_path: &str,
    results: &mut Vec<Node>,
    kind: NodeKind,
) {
    if let Some(name) = extract_name(inner_node, source, kind) {
        let start = template_node.start_position();
        let end = template_node.end_position();
        let content = template_node.utf8_text(source.as_bytes()).ok().map(|s| {
            crate::truncate_content(s, 500)
        });
        results.push(Node {
            id: 0, name, kind, language: Language::Cpp,
            file_path: file_path.to_string(),
            line_start: (start.row + 1) as u32,
            line_end: (end.row + 1) as u32,
            content,
        });
    }
    // Also recurse into the inner node for nested definitions (e.g., methods inside template class)
    let mut cursor = inner_node.walk();
    for child in inner_node.children(&mut cursor) {
        extract_definitions(child, source, file_path, results);
    }
}

fn extract_name(node: tree_sitter::Node, source: &str, kind: NodeKind) -> Option<String> {
    match kind {
        NodeKind::Import => node.utf8_text(source.as_bytes()).ok().map(|s| s.trim().to_string()),
        NodeKind::Constant => {
            node.child_by_field_name("name")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .map(|s| s.to_string())
        }
        NodeKind::Function => {
            fn find_fn_name(n: tree_sitter::Node, src: &str) -> Option<String> {
                if n.kind() == "identifier" || n.kind() == "field_identifier" || n.kind() == "destructor_name" {
                    return n.utf8_text(src.as_bytes()).ok().map(|s| s.to_string());
                }
                // Handle qualified names like ClassName::method
                if n.kind() == "qualified_identifier" || n.kind() == "scoped_identifier" {
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
        NodeKind::Class | NodeKind::Struct | NodeKind::Enum | NodeKind::Module => {
            node.child_by_field_name("name")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .map(|s| s.to_string())
        }
        NodeKind::TypeAlias => {
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

    const SAMPLE_CPP: &str = r#"
#include <iostream>
#include <vector>

#define MAX_SIZE 1024

namespace math {

class Vector {
public:
    double x, y, z;

    Vector(double x, double y, double z) : x(x), y(y), z(z) {}

    double magnitude() const {
        return std::sqrt(x*x + y*y + z*z);
    }

    Vector operator+(const Vector& other) const {
        return Vector(x + other.x, y + other.y, z + other.z);
    }
};

struct Point {
    int x;
    int y;
};

enum class Color {
    Red,
    Green,
    Blue
};

template<typename T>
class Container {
    T value;
public:
    Container(T v) : value(v) {}
    T get() const { return value; }
};

template<typename T>
T add(T a, T b) {
    return a + b;
}

} // namespace math

int main(int argc, char* argv[]) {
    math::Vector v(1, 2, 3);
    std::cout << v.magnitude() << std::endl;
    return 0;
}
"#;

    #[test]
    fn test_parse_cpp_finds_all() {
        let path = PathBuf::from("main.cpp");
        let nodes = parse(&path, SAMPLE_CPP).unwrap();
        let kinds: Vec<_> = nodes.iter().map(|n| n.kind).collect();
        assert!(kinds.contains(&NodeKind::Import), "should find #include");
        assert!(kinds.contains(&NodeKind::Constant), "should find #define");
        assert!(kinds.contains(&NodeKind::Class), "should find class");
        assert!(kinds.contains(&NodeKind::Struct), "should find struct");
        assert!(kinds.contains(&NodeKind::Enum), "should find enum");
        assert!(kinds.contains(&NodeKind::Function), "should find function");
        assert!(kinds.contains(&NodeKind::Module), "should find namespace");
    }

    #[test]
    fn test_parse_cpp_class() {
        let path = PathBuf::from("main.cpp");
        let nodes = parse(&path, SAMPLE_CPP).unwrap();
        let classes: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Class).collect();
        assert!(classes.iter().any(|c| c.name == "Vector"), "should find Vector class");
        assert!(classes.iter().any(|c| c.name == "Container"), "should find Container template class");
    }

    #[test]
    fn test_parse_cpp_namespace() {
        let path = PathBuf::from("main.cpp");
        let nodes = parse(&path, SAMPLE_CPP).unwrap();
        let ns: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Module).collect();
        assert_eq!(ns.len(), 1);
        assert_eq!(ns[0].name, "math");
    }

    #[test]
    fn test_parse_cpp_functions() {
        let path = PathBuf::from("main.cpp");
        let nodes = parse(&path, SAMPLE_CPP).unwrap();
        let fns: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Function).collect();
        assert!(fns.iter().any(|f| f.name == "main"), "should find main");
    }

    #[test]
    fn test_parse_cpp_edges() {
        let path = PathBuf::from("main.cpp");
        let result = parse_with_edges(&path, SAMPLE_CPP).unwrap();
        assert!(!result.edges.is_empty());
    }

    #[test]
    fn test_parse_empty_cpp() {
        let path = PathBuf::from("empty.cpp");
        let nodes = parse(&path, "").unwrap();
        assert!(nodes.len() <= 1);
    }

    #[test]
    fn test_parse_cpp_enum_class() {
        let path = PathBuf::from("main.cpp");
        let nodes = parse(&path, SAMPLE_CPP).unwrap();
        let enums: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Enum).collect();
        assert!(enums.iter().any(|e| e.name == "Color"), "should find enum class Color");
    }
}
