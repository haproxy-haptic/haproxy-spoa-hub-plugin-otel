//! `otel-group` block (data-model.md §1).

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub name: String,
    pub scopes: Vec<String>,
}
