//! Ruby language parser using tree-sitter-ruby.

use deagle_core::{DeagleError, EdgeKind, Language, Node, NodeKind, Result};
use std::path::Path;
use crate::ParseResult;

pub fn parse(path: &Path, content: &str) -> Result<Vec<Node>> {
    parse_with_edges(path, content).map(|r| r.nodes)
}

pub fn parse_with_edges(path: &Path, content: &str) -> Result<ParseResult> {
    let mut parser = tree_sitter::Parser::new();
    let language = tree_sitter_ruby::LANGUAGE;
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
        language: Language::Ruby,
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
        "method" | "singleton_method" => Some(NodeKind::Method),
        "class" => Some(NodeKind::Class),
        "module" => Some(NodeKind::Module),
        "constant_assignment" | "casgn" => Some(NodeKind::Constant),
        "call" => {
            // Detect require/require_relative/include/extend
            if let Some(method) = node.child_by_field_name("method") {
                let method_name = method.utf8_text(source.as_bytes()).unwrap_or("");
                match method_name {
                    "require" | "require_relative" | "include" | "extend" | "attr_accessor"
                    | "attr_reader" | "attr_writer" => Some(NodeKind::Import),
                    _ => None,
                }
            } else {
                None
            }
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
                id: 0, name, kind, language: Language::Ruby,
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
        NodeKind::Class | NodeKind::Module => {
            node.child_by_field_name("name")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .map(|s| s.to_string())
        }
        NodeKind::Method => {
            node.child_by_field_name("name")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .map(|s| s.to_string())
        }
        NodeKind::Constant => {
            // constant_assignment: NAME = value
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "constant" {
                    return child.utf8_text(source.as_bytes()).ok().map(|s| s.to_string());
                }
            }
            None
        }
        NodeKind::Import => {
            // require "name" or require_relative "name"
            node.utf8_text(source.as_bytes()).ok().map(|s| s.trim().to_string())
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

    const SAMPLE_RUBY: &str = r#"
require 'json'
require_relative 'helpers'

MAX_SIZE = 1024
VERSION = "1.0.0"

module Animals
  class Dog
    attr_accessor :name, :breed

    def initialize(name, breed)
      @name = name
      @breed = breed
    end

    def bark
      "Woof! I'm #{@name}"
    end

    def self.species
      "Canis familiaris"
    end
  end

  class Cat
    def initialize(name)
      @name = name
    end

    def meow
      "Meow!"
    end
  end
end

def greet(name)
  puts "Hello, #{name}!"
end
"#;

    #[test]
    fn test_parse_ruby_finds_all() {
        let path = PathBuf::from("app.rb");
        let nodes = parse(&path, SAMPLE_RUBY).unwrap();
        let kinds: Vec<_> = nodes.iter().map(|n| n.kind).collect();
        assert!(kinds.contains(&NodeKind::Import), "should find require");
        assert!(kinds.contains(&NodeKind::Class), "should find class");
        assert!(kinds.contains(&NodeKind::Module), "should find module");
        assert!(kinds.contains(&NodeKind::Method), "should find method");
    }

    #[test]
    fn test_parse_ruby_classes() {
        let path = PathBuf::from("app.rb");
        let nodes = parse(&path, SAMPLE_RUBY).unwrap();
        let classes: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Class).collect();
        assert!(classes.iter().any(|c| c.name == "Dog"), "should find Dog class");
        assert!(classes.iter().any(|c| c.name == "Cat"), "should find Cat class");
    }

    #[test]
    fn test_parse_ruby_module() {
        let path = PathBuf::from("app.rb");
        let nodes = parse(&path, SAMPLE_RUBY).unwrap();
        let mods: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Module).collect();
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].name, "Animals");
    }

    #[test]
    fn test_parse_ruby_methods() {
        let path = PathBuf::from("app.rb");
        let nodes = parse(&path, SAMPLE_RUBY).unwrap();
        let methods: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Method).collect();
        assert!(methods.iter().any(|m| m.name == "initialize"), "should find initialize");
        assert!(methods.iter().any(|m| m.name == "bark"), "should find bark");
        assert!(methods.iter().any(|m| m.name == "greet"), "should find greet");
        assert!(methods.iter().any(|m| m.name == "species"), "should find singleton method species");
    }

    #[test]
    fn test_parse_ruby_edges() {
        let path = PathBuf::from("app.rb");
        let result = parse_with_edges(&path, SAMPLE_RUBY).unwrap();
        assert!(!result.edges.is_empty());
    }

    #[test]
    fn test_parse_empty_ruby() {
        let path = PathBuf::from("empty.rb");
        let nodes = parse(&path, "").unwrap();
        assert!(nodes.len() <= 1);
    }

    #[test]
    fn test_parse_ruby_requires() {
        let path = PathBuf::from("app.rb");
        let nodes = parse(&path, SAMPLE_RUBY).unwrap();
        let imports: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Import).collect();
        assert!(imports.len() >= 2, "should find require and require_relative, got {}", imports.len());
    }
}
