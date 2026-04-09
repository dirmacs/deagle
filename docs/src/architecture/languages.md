# Language Support

Deagle uses tree-sitter grammars for parsing. Each language has its own parser module.

## Current Support

| Language | Status | Extensions | Parser |
|----------|--------|------------|--------|
| Rust | Implemented | `.rs` | `tree-sitter-rust` |
| Python | Planned | `.py` | `tree-sitter-python` |
| Go | Planned | `.go` | `tree-sitter-go` |
| TypeScript | Planned | `.ts`, `.tsx` | `tree-sitter-typescript` |
| JavaScript | Planned | `.js`, `.jsx` | `tree-sitter-javascript` |
| Java | Planned | `.java` | `tree-sitter-java` |
| C/C++ | Planned | `.c`, `.h`, `.cpp` | `tree-sitter-c`, `tree-sitter-cpp` |

## What Gets Extracted

For each supported language, deagle extracts:
- **Definitions**: functions, methods, classes/structs, enums, traits/interfaces, constants
- **Imports**: use/import statements with full path
- **Location**: file path, start line, end line
- **Content**: source code excerpt (truncated to 500 chars)
