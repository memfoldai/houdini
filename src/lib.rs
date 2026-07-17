//! Portable, fully-tested core of the AI-usage monitor.
//!
//! Everything here is platform-independent and unit-tested: transcript
//! ingestion (Layer A), provider attribution / entity grouping, the offline
//! redaction layer, the SQLite store, config/paths, and time helpers. The
//! macOS-native pieces — the network-presence poller (Layer B) and the tray
//! shell — live in the binary (`main.rs`), cfg-gated, and depend only on this
//! crate's public API.

pub mod attribution;
pub mod config;
pub mod export;
pub mod ingest;
pub mod logging;
pub mod redact;
pub mod store;
pub mod timestamp;

#[cfg(feature = "ner")]
pub mod ner;
