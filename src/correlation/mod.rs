//! In-process correlation map (tasks T010, T011).
//!
//! See `specs/001-otel-plugin/data-model.md` §2 (`InFlightSpan`,
//! `CorrelationStore`) and research item R4.

pub mod handle;
pub mod store;

pub use handle::CorrelationHandle;
pub use store::{CorrelationStore, EvictionReason, InFlightSpan};
