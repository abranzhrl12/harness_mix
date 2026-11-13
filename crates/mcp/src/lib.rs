//! MCP service and bounded input utilities for applications that provide their own runtime.
//!
//! Construct [`McpServer`] with an application and optional job service, then pass it to
//! an `rmcp` transport. This crate does not create a transport or read process environment.
mod server;

/// MCP service configured around an injected application and optional job service.
pub use server::McpServer;
/// Async reader that rejects an incoming MCP line once it exceeds the configured limit.
pub use server::input::{BoundedLines, MAX_MESSAGE_BYTES};
