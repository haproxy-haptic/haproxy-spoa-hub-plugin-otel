#![allow(non_camel_case_types, non_local_definitions)]

//! OpenTelemetry plugin for haproxy-spoa-hub.
//!
//! See `specs/001-otel-plugin/` for the feature specification, plan, and
//! contracts that define this plugin's behaviour.

pub mod attribute;
pub mod config;
pub mod correlation;
pub mod dispatch;
pub mod error;
pub mod health;
pub mod pipelines;
pub mod propagation;

mod plugin;
