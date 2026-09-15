# Rust Coding Standards

## General
- Use Rust 2024 edition
- Max line width: 120
- Use 4-space indentation
- Use Unix line endings

## Error Handling
- NEVER use `.unwrap()` or `.expect()` in production code
- Use `anyhow::Result` for fallible functions
- Use `thiserror` for library error types
- Use `?` operator for propagation

## Naming
- Types: PascalCase
- Functions/variables: snake_case
- Constants: SCREAMING_SNAKE_CASE
- Avoid `module_name_repetitions` (e.g. `device_monitor::DeviceMonitor`)

## Imports
- Group: std → external crates → crate
- Sort alphabetically within groups
- No `use crate::*` wildcard imports

## Documentation
- All public items must have doc comments (`///`)
- Use `//!` for module-level docs
- Document errors and panic conditions

## Concurrency
- Use `tokio` as async runtime
- Use `tokio::sync::Mutex` for async contexts
- Prefer message passing over shared state

## Testing
- Unit tests in `#[cfg(test)] mod tests`
- Use `anyhow::Result` return type in tests
- Use `assert_eq!` / `assert!` with meaningful messages

## Dependencies
- Prefer `rusqlite` with `bundled` feature for SQLite
- Use `serde` + `serde_json` for serialization
- Use `tracing` for logging (not `println!`)
- Use `axum` for HTTP/WebSocket
- Use `tower-http` for CORS and static file serving
