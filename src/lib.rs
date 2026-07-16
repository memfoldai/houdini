//! Portable, fully-tested core of the AI-usage monitor.
//!
//! Everything here is platform-independent and unit-tested: the streaming
//! detector (the "world model"), the offline redaction layer, the SQLite
//! store, config/paths, and the session recorder that orchestrates
//! redact-then-store. The macOS-native capture + tray shell lives in the
//! binary (`main.rs` + `capture/`), cfg-gated, and depends only on this crate's
//! public API.

pub mod config;
pub mod detector;
pub mod export;
pub mod logging;
pub mod monitor;
pub mod redact;
pub mod session;
pub mod store;

#[cfg(feature = "ner")]
pub mod ner;
