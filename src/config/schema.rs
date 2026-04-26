//! JSON Schema for the plugin's TOML configuration (task T008).
//!
//! The schema is the canonical source of truth for the configuration
//! surface and is shipped alongside the plugin so the hub can validate
//! the operator's TOML before `init()` is called.

use crate::error::ConfigError;

/// Embedded copy of `specs/001-otel-plugin/contracts/plugin-config-schema.json`.
pub const SCHEMA_JSON: &str =
    include_str!("../../specs/001-otel-plugin/contracts/plugin-config-schema.json");

/// Validate a configuration value against the embedded schema.
pub fn validate(value: &serde_json::Value) -> Result<(), ConfigError> {
    let schema_value: serde_json::Value =
        serde_json::from_str(SCHEMA_JSON).map_err(|e| ConfigError::Schema {
            key_path: "$schema".to_string(),
            message: format!("embedded schema is malformed: {e}"),
        })?;

    let validator = jsonschema::validator_for(&schema_value).map_err(|e| ConfigError::Schema {
        key_path: "$schema".to_string(),
        message: format!("schema compilation failed: {e}"),
    })?;

    if let Err(error) = validator.validate(value) {
        return Err(ConfigError::Schema {
            key_path: error.instance_path.to_string(),
            message: error.to_string(),
        });
    }

    Ok(())
}
