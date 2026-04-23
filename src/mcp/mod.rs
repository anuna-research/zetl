//! MCP (Model Context Protocol) server for ztl (SPEC-021).
//!
//! Gated behind the `mcp` Cargo feature. Exposes vault graph traversal,
//! search, and reasoning as typed MCP tools over stdio and HTTP transports.

pub mod auth;
pub mod delegate;
pub mod resources;
pub mod server;
pub mod tools;
pub mod transport;
pub mod types;
