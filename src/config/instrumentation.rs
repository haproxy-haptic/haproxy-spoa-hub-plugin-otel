//! `otel-instrumentation` block (data-model.md §1).

use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Instrumentation {
    #[serde(default)]
    pub name: String,

    /// Optional default pipeline this instrumentation activates.
    #[serde(default)]
    pub config_pipeline: Option<String>,

    #[serde(default)]
    pub acls: Vec<AclRule>,

    #[serde(default)]
    pub log: Option<LogPolicy>,

    #[serde(default)]
    pub groups: Vec<String>,

    #[serde(default)]
    pub scopes: Vec<String>,

    #[serde(default)]
    pub rate_limit: Option<RateLimit>,

    #[serde(default)]
    pub debug_level: u8,

    #[serde(default)]
    pub option: Option<InstrumentationOption>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstrumentationOption {
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub dontlog_normal: bool,
    #[serde(default)]
    pub hard_errors: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AclRule {
    pub name: String,
    pub criterion: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogPolicy {
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RateLimit {
    pub count: u64,
    #[serde(with = "humantime_serde")]
    pub window: std::time::Duration,
}
