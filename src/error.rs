//! Plugin error types.
//!
//! Configuration errors carry enough context (file, key path, line, human
//! explanation) to satisfy spec FR-018 and SC-007. Runtime errors are
//! coarse-grained and never propagated to HAProxy: per FR-014 the plugin
//! fails open with respect to traffic.

use std::fmt;

/// Errors surfaced from `init()` — configuration parsing and validation.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// TOML parse error (syntactic).
    #[error("invalid TOML in plugin configuration: {message}")]
    Toml { message: String },

    /// JSON Schema validation error.
    #[error("configuration rejected at `{key_path}`: {message}")]
    Schema { key_path: String, message: String },

    /// Operator used a TOML key path corresponding to a built-in-addon
    /// directive the plugin does not implement (FR-017).
    #[error(
        "unsupported directive `{addon_directive}` (TOML key path: `{key_path}`); see contracts/addon-directive-mapping.md"
    )]
    UnsupportedDirective {
        addon_directive: String,
        key_path: String,
    },

    /// Cross-field semantic validation failure (e.g. a scope references a
    /// pipeline name that doesn't exist).
    #[error("configuration invariant violated: {message}")]
    Invariant { message: String },
}

/// Errors surfaced from message processing. These are logged but never
/// propagated as HAProxy-visible failures.
#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("required SPOE argument `{name}` was missing or had wrong type")]
    MissingArg { name: String },

    #[error("scope `{scope}` not found for event `{event}`")]
    UnknownScope { scope: String, event: String },

    #[error("correlation handle `{handle}` not found in store")]
    UnknownCorrelation { handle: String },

    #[error("internal error: {0}")]
    Internal(String),
}

/// Convenience alias for `Result<T, Box<dyn Error + Send + Sync>>` —
/// matches the signature the plugin SDK's `define_plugin!` macro expects.
pub type PluginResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

impl ConfigError {
    /// Render an error with full context for the operator. Used by
    /// `init()` to satisfy SC-007.
    pub fn render(&self) -> String {
        format!("{self}")
    }
}

/// Helper for converting any `Display` value into a `ProcessError::Internal`.
pub fn internal<E: fmt::Display>(e: E) -> ProcessError {
    ProcessError::Internal(e.to_string())
}
