//! An MCP server for Raywright: an agent (Claude Code, any MCP client) launches desktop
//! apps, reads windows as text snapshots with refs, clicks, types, checks, takes pictures,
//! records animations and turns the session into a Rust test.
//!
//! Build your own server with extra tools:
//!
//! ```no_run
//! # async fn f() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//! use raywright_mcp::{RaywrightServer, Policy};
//! let rw = raywright::Raywright::builder().driver(raywright::mock::MockDriver::new().into_arc()).build()?;
//! RaywrightServer::new(rw, Policy::default()).serve_stdio().await?;
//! # Ok(()) }
//! ```

mod policy;
mod server;
mod session;
mod snapshot;
mod tools;

pub use policy::Policy;
pub use server::RaywrightServer;
pub use session::Session;
pub use snapshot::{Target, render as render_snapshot};
pub use tools::{BUILTINS, Builtin, McpTool, ToolResult, builtins};
