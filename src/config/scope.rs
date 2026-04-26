//! `otel-scope` block (data-model.md §1).

use crate::attribute::AttributeType;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub name: String,
    pub event: String,

    /// Optional explicit role of this scope. When absent the dispatcher
    /// infers role from the presence of the `correlation` SPOE argument
    /// (request-side messages don't carry one; response-side and
    /// group-trigger messages do).
    #[serde(default)]
    pub kind: Option<ScopeKind>,

    #[serde(default)]
    pub event_condition: Option<EventCondition>,

    #[serde(default, with = "humantime_serde::option")]
    pub idle_timeout: Option<std::time::Duration>,

    #[serde(default)]
    pub spans: Vec<SpanRule>,
    #[serde(default)]
    pub attributes: Vec<AttributeRule>,
    #[serde(default)]
    pub events: Vec<EventRule>,
    #[serde(default)]
    pub baggage: Vec<BaggageRule>,
    #[serde(default)]
    pub status: Option<StatusRule>,
    #[serde(default)]
    pub links: Vec<LinkRule>,
    #[serde(default)]
    pub inject: Option<InjectRule>,
    #[serde(default)]
    pub extract: Option<ExtractRule>,
    #[serde(default)]
    pub finish: Vec<FinishRule>,
    #[serde(default)]
    pub instruments: Vec<InstrumentRule>,
    #[serde(default)]
    pub instrument_updates: Vec<InstrumentUpdate>,
    #[serde(default)]
    pub log_records: Vec<LogRecordRule>,
}

/// Optional explicit role tag on a scope. Mirrors the implicit
/// request/response position that the addon infers from the lifecycle
/// event name; the plugin can't infer it because SPOE message names are
/// operator-chosen, so we let the operator declare it.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeKind {
    Request,
    Response,
    Group,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventCondition {
    #[serde(default)]
    pub r#if: Option<String>,
    #[serde(default)]
    pub unless: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpanRule {
    pub name: String,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub link: Vec<String>,
    #[serde(default)]
    pub root: bool,
    #[serde(default)]
    pub tracer_pipeline: Option<String>,
}

/// Source of a value for an attribute, instrument, or log body.
///
/// Per research item R6, the only run-time source in v1 is `$arg.<name>`
/// (a SPOE message argument). TOML literals (string, integer, float,
/// bool) are also accepted.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ValueSource {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
}

impl ValueSource {
    /// If the value is a `$arg.<name>` reference, return the argument name.
    pub fn arg_ref(&self) -> Option<&str> {
        match self {
            ValueSource::Str(s) => s.strip_prefix("$arg."),
            _ => None,
        }
    }

    /// True if the value is a literal (no `$arg.`/`$var.` prefix).
    pub fn is_literal(&self) -> bool {
        match self {
            ValueSource::Str(s) => !s.starts_with('$'),
            _ => true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttributeRule {
    pub key: String,
    pub source: ValueSource,
    #[serde(default)]
    pub r#type: Option<AttributeType>,
    #[serde(default)]
    pub condition: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventRule {
    pub name: String,
    #[serde(default)]
    pub attributes: Vec<AttributeRule>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BaggageRule {
    pub key: String,
    pub source: ValueSource,
    #[serde(default)]
    pub r#type: Option<AttributeType>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusRule {
    pub code: StatusCode,
    #[serde(default)]
    pub condition: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StatusCode {
    Ok,
    Error,
    Unset,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkRule {
    pub span: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InjectRule {
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub use_vars: bool,
    #[serde(default = "default_true")]
    pub use_headers: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractRule {
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub use_vars: bool,
    #[serde(default = "default_true")]
    pub use_headers: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinishRule {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstrumentRule {
    pub kind: InstrumentKind,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub unit: Option<String>,
    pub value: ValueSource,
    #[serde(default)]
    pub bounds: Vec<f64>,
    #[serde(default)]
    pub aggr: Option<crate::config::pipeline::AggregationKind>,
    #[serde(default)]
    pub attributes: Vec<AttributeRule>,
    #[serde(default)]
    pub meter_pipeline: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentKind {
    /// `cnt_int` — counter (integer values).
    CntInt,
    /// `udcnt_int` — up-down counter.
    UdcntInt,
    /// `hist_int` — histogram.
    HistInt,
    /// `gauge_int` — gauge.
    GaugeInt,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstrumentUpdate {
    pub name: String,
    #[serde(default)]
    pub attributes: Vec<AttributeRule>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogRecordRule {
    pub severity: SeverityNumber,
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub event_name: Option<String>,
    #[serde(default)]
    pub span: Option<String>,
    #[serde(default)]
    pub body: Option<ValueSource>,
    #[serde(default)]
    pub attributes: Vec<AttributeRule>,
    #[serde(default)]
    pub logger_pipeline: Option<String>,
}

/// Mirrors the addon's severity vocabulary.
#[derive(Debug, Clone, Copy, Deserialize)]
#[allow(clippy::upper_case_acronyms)]
pub enum SeverityNumber {
    TRACE,
    TRACE2,
    TRACE3,
    TRACE4,
    DEBUG,
    DEBUG2,
    DEBUG3,
    DEBUG4,
    INFO,
    INFO2,
    INFO3,
    INFO4,
    WARN,
    WARN2,
    WARN3,
    WARN4,
    ERROR,
    ERROR2,
    ERROR3,
    ERROR4,
    FATAL,
    FATAL2,
    FATAL3,
    FATAL4,
}
