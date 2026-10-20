//! retake: record LLM API calls once, replay them deterministically in tests.
//!
//! The binary is the main interface; this library exposes the same pieces for
//! embedding and for integration tests.

pub mod cassette;
pub mod check;
pub mod config;
pub mod live;
pub mod matcher;
pub mod proxy;
pub mod redact;
pub mod runner;
pub mod sse;
pub mod store;
pub mod ui;
