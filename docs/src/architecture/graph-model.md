# Graph Model

Deagle models code as a directed graph of **nodes** (code entities) and **edges** (relationships).

## Node Types

| Kind | Description | Example |
|------|-------------|---------|
| `file` | Source file | `src/main.rs` |
| `module` | Module declaration | `mod config;` |
| `function` | Top-level function | `fn main()` |
| `method` | Impl method | `fn new() -> Self` |
| `struct` | Struct definition | `struct Config` |
| `enum` | Enum definition | `enum Status` |
| `trait` | Trait definition | `trait Handler` |
| `interface` | Interface (non-Rust) | `interface Props` |
| `constant` | Const/static | `const MAX: usize` |
| `type_alias` | Type alias | `type Result<T>` |
| `import` | Use/import statement | `use std::io` |

## Edge Types

| Kind | Description |
|------|-------------|
| `calls` | Function/method invocation |
| `imports` | Import/use relationship |
| `contains` | Parent-child (file→function) |
| `inherits` | Class/struct inheritance |
| `implements` | Trait/interface implementation |
| `references` | Type annotation reference |
| `depends_on` | Module dependency |

## Storage

SQLite with two tables (`nodes`, `edges`) and indexes on name, kind, file_path, and edge endpoints.
