//! Library surface for the `hivemind` CLI, daemon, and MCP shim.
//!
//! M1 process model (`docs/06-prototype-plan.md`, `docs/m1-implementation-notes.md`):
//! - [`daemon`] owns a store and serves a Unix-socket protocol
//! - [`mcp`] is a stdio MCP shim that finds or starts the daemon
//! - [`client`] talks to the daemon; [`service`] handles requests against a [`hivemind_engine::Store`]

pub mod backend;
pub mod client;
pub mod daemon;
pub mod exchange;
pub mod grpc;
pub mod http;
pub mod mcp;
pub mod oauth;
pub mod protocol;
pub mod service;
pub mod sync;
pub mod tenant;
