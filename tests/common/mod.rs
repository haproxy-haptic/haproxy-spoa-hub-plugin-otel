//! Shared test fixtures (task T015).
//!
//! Re-exports the in-process fake OTLP collector. Each integration test
//! file uses `mod common;` to bring this in scope.

#![allow(dead_code)]

pub mod otlp_collector;
