//! Find and download professional CS2 demo archives from hltv.org through a real Chrome.
//!
//! The CLI (`hltv-demo`) and the MCP server (`hltv-demo-mcp`) are thin shells over this library:
//! `search`, `resolve`, and `get` take a [`Source`], never a browser, so the same operations run
//! against Chrome in production and against a fake in tests.
//!
//! ```no_run
//! use hltv_demo::{Chrome, Options, get, with_source};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), hltv_demo::HltvError> {
//!     let url = "https://www.hltv.org/matches/2399723/vitality-vs-9z-pgl-masters-bucharest-2026";
//!     let on_note = |note: &str| eprintln!("{note}");
//!     let options = Options { out: Some("./demos".into()), on_note: Some(&on_note), ..Default::default() };
//!     // One headed Chrome for the duration of the work, holding the profile lock, closed after.
//!     let archives = with_source(&Chrome, async |source| get(source, url, &options).await).await?;
//!     for archive in archives {
//!         println!("{} ({} bytes) from {}", archive.path, archive.bytes, archive.demo_url);
//!     }
//!     Ok(())
//! }
//! ```
//!
//! The same example is in the README; this copy is compiled by `cargo test`, so the two stay true.

pub mod browser;
pub mod cli;
pub mod core;
pub mod hltv;
pub mod mcp;
pub mod process;
pub mod source;

pub use crate::browser::{Chrome, Offline, Session};
pub use crate::core::errors::HltvError;
pub use crate::core::types::{Archive, DemoRef, Match, SearchFilter};
pub use crate::hltv::get::Options;
pub use crate::hltv::{get::get, resolve::resolve, search::search};
pub use crate::source::{Opener, Source, with_source};
