//! Scope-rule helpers shared by request/response/group dispatch
//! (task T029).

use super::{ARG_CORRELATION, RuntimeState};
use crate::attribute;
use crate::config::{
    AttributeRule, InstrumentKind, InstrumentRule, LogRecordRule, Scope, ScopeKind, SeverityNumber,
    ValueSource,
};
use abi_stable::std_types::RString;
use haproxy_spoa_hub_plugin_api::{ProcessingResult, SpoeMessage, SpoeValue};
use opentelemetry::KeyValue;
use opentelemetry::logs::{AnyValue, LogRecord, Logger, LoggerProvider, Severity};
use opentelemetry::metrics::MeterProvider;
use opentelemetry::trace::TraceContextExt;

pub fn dispatch_scope(
    state: &RuntimeState,
    scope: &Scope,
    message: &SpoeMessage,
) -> ProcessingResult {
    let role = scope
        .kind
        .or_else(|| infer_role(scope, message))
        .unwrap_or(ScopeKind::Request);
    match role {
        ScopeKind::Request => super::request::handle(state, scope, message),
        ScopeKind::Response | ScopeKind::Group => super::response::handle(state, scope, message),
    }
}

/// Heuristic role inference when the operator did not declare `kind`:
/// a scope that has any `finish` rules is treated as response-side; one
/// that sees a `correlation` argument is response-side; otherwise it is
/// request-side.
fn infer_role(scope: &Scope, message: &SpoeMessage) -> Option<ScopeKind> {
    if !scope.finish.is_empty() {
        return Some(ScopeKind::Response);
    }
    if message.get_string(ARG_CORRELATION).is_some() {
        return Some(ScopeKind::Response);
    }
    if scope.spans.is_empty() && scope.status.is_some() {
        // Pure status-applying scope (no span open) reads as response-side.
        return Some(ScopeKind::Response);
    }
    None
}

/// Build a `KeyValue` from an attribute rule and the current SPOE message.
/// Returns `None` if the value cannot be sourced or coerced (caller
/// omits the attribute and increments any per-rule error counter it owns).
pub fn attribute_kv(rule: &AttributeRule, message: &SpoeMessage) -> Option<KeyValue> {
    let value = source_value(&rule.source, message)?;
    attribute::key_value(rule.key.clone(), &value, rule.r#type)
}

/// Resolve a `ValueSource` to a `SpoeValue`. Literals are converted
/// inline; `$arg.<name>` is looked up on the current message.
pub fn source_value(src: &ValueSource, message: &SpoeMessage) -> Option<SpoeValue> {
    if let Some(arg) = src.arg_ref() {
        return message.get(arg).cloned();
    }
    Some(match src {
        ValueSource::Bool(b) => SpoeValue::Bool(*b),
        ValueSource::Int(i) => SpoeValue::Int64(*i),
        ValueSource::Float(_) => return None, // OTel attribute values handle floats but SpoeValue lacks a Float variant
        ValueSource::Str(s) => SpoeValue::String(RString::from(s.clone())),
    })
}

/// Evaluate a TOML-string condition. v1 supports a small DSL of
/// `$arg.<name> <op> <literal>` tests; anything else returns true so
/// the rule fires (the addon's parser uses HAProxy ACLs which we don't
/// re-implement here).
pub fn condition_holds(cond: &Option<String>, message: &SpoeMessage) -> bool {
    let Some(expr) = cond else {
        return true;
    };
    let trimmed = expr.trim();
    // Supported: `$arg.<n> >= <int>`, `<= <int>`, `> <int>`, `< <int>`,
    // `== <int|str>`, `!= <int|str>`. Falls through to true.
    for op in [">=", "<=", "==", "!=", ">", "<"] {
        if let Some((lhs, rhs)) = trimmed.split_once(op) {
            let lhs = lhs.trim();
            let rhs = rhs.trim();
            let Some(arg_name) = lhs.strip_prefix("$arg.") else {
                return true;
            };
            let Some(actual) = message.get(arg_name) else {
                return false;
            };
            return compare(actual, op, rhs);
        }
    }
    true
}

/// Emit all log-record rules in this scope (tasks T044, T045). When a
/// span context is supplied (i.e. the rule fired during a sampled
/// in-flight transaction) the log record is annotated with the
/// trace_id and span_id of the active span.
pub fn emit_log_records(
    state: &RuntimeState,
    scope: &Scope,
    message: &SpoeMessage,
    active_context: Option<&opentelemetry::Context>,
) {
    if scope.log_records.is_empty() {
        return;
    }
    for rule in &scope.log_records {
        let provider = match state
            .pipelines
            .logger_or_default(rule.logger_pipeline.as_deref())
        {
            Some(p) => p,
            None => continue,
        };
        emit_one_log(rule, message, provider, active_context);
    }
}

fn emit_one_log(
    rule: &LogRecordRule,
    message: &SpoeMessage,
    provider: &opentelemetry_sdk::logs::LoggerProvider,
    active_context: Option<&opentelemetry::Context>,
) {
    let logger = provider.logger("otel-plugin");
    let mut record = logger.create_log_record();
    record.set_severity_number(map_severity(rule.severity));
    record.set_severity_text(severity_text(rule.severity));

    if let Some(body_src) = &rule.body {
        if let Some(value) = source_value(body_src, message) {
            if let Some(av) = spoe_value_to_anyvalue(&value) {
                record.set_body(av);
            }
        }
    }

    for a in &rule.attributes {
        if let Some(kv) = attribute_kv(a, message) {
            if let Some(av) = otel_value_to_anyvalue(&kv.value) {
                record.add_attribute(kv.key, av);
            }
        }
    }

    if let Some(ctx) = active_context {
        let span = ctx.span();
        let sc = span.span_context();
        if sc.is_valid() {
            record.set_trace_context(sc.trace_id(), sc.span_id(), Some(sc.trace_flags()));
        }
    }

    logger.emit(record);
}

fn map_severity(s: SeverityNumber) -> Severity {
    match s {
        SeverityNumber::TRACE => Severity::Trace,
        SeverityNumber::TRACE2 => Severity::Trace2,
        SeverityNumber::TRACE3 => Severity::Trace3,
        SeverityNumber::TRACE4 => Severity::Trace4,
        SeverityNumber::DEBUG => Severity::Debug,
        SeverityNumber::DEBUG2 => Severity::Debug2,
        SeverityNumber::DEBUG3 => Severity::Debug3,
        SeverityNumber::DEBUG4 => Severity::Debug4,
        SeverityNumber::INFO => Severity::Info,
        SeverityNumber::INFO2 => Severity::Info2,
        SeverityNumber::INFO3 => Severity::Info3,
        SeverityNumber::INFO4 => Severity::Info4,
        SeverityNumber::WARN => Severity::Warn,
        SeverityNumber::WARN2 => Severity::Warn2,
        SeverityNumber::WARN3 => Severity::Warn3,
        SeverityNumber::WARN4 => Severity::Warn4,
        SeverityNumber::ERROR => Severity::Error,
        SeverityNumber::ERROR2 => Severity::Error2,
        SeverityNumber::ERROR3 => Severity::Error3,
        SeverityNumber::ERROR4 => Severity::Error4,
        SeverityNumber::FATAL => Severity::Fatal,
        SeverityNumber::FATAL2 => Severity::Fatal2,
        SeverityNumber::FATAL3 => Severity::Fatal3,
        SeverityNumber::FATAL4 => Severity::Fatal4,
    }
}

fn severity_text(s: SeverityNumber) -> &'static str {
    match s {
        SeverityNumber::TRACE
        | SeverityNumber::TRACE2
        | SeverityNumber::TRACE3
        | SeverityNumber::TRACE4 => "TRACE",
        SeverityNumber::DEBUG
        | SeverityNumber::DEBUG2
        | SeverityNumber::DEBUG3
        | SeverityNumber::DEBUG4 => "DEBUG",
        SeverityNumber::INFO
        | SeverityNumber::INFO2
        | SeverityNumber::INFO3
        | SeverityNumber::INFO4 => "INFO",
        SeverityNumber::WARN
        | SeverityNumber::WARN2
        | SeverityNumber::WARN3
        | SeverityNumber::WARN4 => "WARN",
        SeverityNumber::ERROR
        | SeverityNumber::ERROR2
        | SeverityNumber::ERROR3
        | SeverityNumber::ERROR4 => "ERROR",
        SeverityNumber::FATAL
        | SeverityNumber::FATAL2
        | SeverityNumber::FATAL3
        | SeverityNumber::FATAL4 => "FATAL",
    }
}

fn spoe_value_to_anyvalue(v: &SpoeValue) -> Option<AnyValue> {
    match v {
        SpoeValue::String(s) => Some(AnyValue::String(s.as_str().to_string().into())),
        SpoeValue::Int32(i) => Some(AnyValue::Int(i64::from(*i))),
        SpoeValue::Int64(i) => Some(AnyValue::Int(*i)),
        SpoeValue::Uint32(u) => Some(AnyValue::Int(i64::from(*u))),
        SpoeValue::Uint64(u) => i64::try_from(*u).ok().map(AnyValue::Int),
        SpoeValue::Bool(b) => Some(AnyValue::Boolean(*b)),
        _ => None,
    }
}

fn otel_value_to_anyvalue(v: &opentelemetry::Value) -> Option<AnyValue> {
    match v {
        opentelemetry::Value::String(s) => Some(AnyValue::String(s.as_str().to_string().into())),
        opentelemetry::Value::Bool(b) => Some(AnyValue::Boolean(*b)),
        opentelemetry::Value::I64(i) => Some(AnyValue::Int(*i)),
        opentelemetry::Value::F64(f) => Some(AnyValue::Double(*f)),
        _ => None,
    }
}

/// Emit all instrument rules in this scope against the active meter
/// pipeline (task T037, T038, T040).
pub fn emit_instruments(state: &RuntimeState, scope: &Scope, message: &SpoeMessage) {
    if scope.instruments.is_empty() {
        return;
    }
    for rule in &scope.instruments {
        let provider = match state
            .pipelines
            .meter_or_default(rule.meter_pipeline.as_deref())
        {
            Some(p) => p,
            None => continue,
        };
        emit_one(rule, message, provider);
    }
}

fn emit_one(
    rule: &InstrumentRule,
    message: &SpoeMessage,
    provider: &opentelemetry_sdk::metrics::SdkMeterProvider,
) {
    let meter = provider.meter("otel-plugin");
    let value = match source_value(&rule.value, message) {
        Some(v) => v,
        None => return,
    };
    let dimensions: Vec<KeyValue> = rule
        .attributes
        .iter()
        .filter_map(|a| attribute_kv(a, message))
        .collect();

    match rule.kind {
        InstrumentKind::CntInt => {
            let counter = meter.u64_counter(rule.name.clone()).build();
            if let Some(n) = as_u64(&value) {
                counter.add(n, &dimensions);
            }
        }
        InstrumentKind::UdcntInt => {
            let counter = meter.i64_up_down_counter(rule.name.clone()).build();
            if let Some(n) = as_i64(&value) {
                counter.add(n, &dimensions);
            }
        }
        InstrumentKind::HistInt => {
            let histogram = meter.u64_histogram(rule.name.clone()).build();
            if let Some(n) = as_u64(&value) {
                histogram.record(n, &dimensions);
            }
        }
        InstrumentKind::GaugeInt => {
            let gauge = meter.i64_gauge(rule.name.clone()).build();
            if let Some(n) = as_i64(&value) {
                gauge.record(n, &dimensions);
            }
        }
    }
}

fn as_u64(v: &SpoeValue) -> Option<u64> {
    match v {
        SpoeValue::Int32(i) => u64::try_from(*i).ok(),
        SpoeValue::Int64(i) => u64::try_from(*i).ok(),
        SpoeValue::Uint32(u) => Some(u64::from(*u)),
        SpoeValue::Uint64(u) => Some(*u),
        SpoeValue::Bool(b) => Some(u64::from(*b)),
        _ => None,
    }
}

fn as_i64(v: &SpoeValue) -> Option<i64> {
    match v {
        SpoeValue::Int32(i) => Some(i64::from(*i)),
        SpoeValue::Int64(i) => Some(*i),
        SpoeValue::Uint32(u) => Some(i64::from(*u)),
        SpoeValue::Uint64(u) => i64::try_from(*u).ok(),
        SpoeValue::Bool(b) => Some(i64::from(*b)),
        _ => None,
    }
}

fn compare(actual: &SpoeValue, op: &str, rhs: &str) -> bool {
    // Try integer compare first; fall back to string compare for `==` / `!=`.
    if let Ok(rhs_i) = rhs.parse::<i64>() {
        let actual_i = match actual {
            SpoeValue::Int32(i) => Some(i64::from(*i)),
            SpoeValue::Int64(i) => Some(*i),
            SpoeValue::Uint32(u) => Some(i64::from(*u)),
            SpoeValue::Uint64(u) => i64::try_from(*u).ok(),
            _ => None,
        };
        if let Some(a) = actual_i {
            return match op {
                ">=" => a >= rhs_i,
                "<=" => a <= rhs_i,
                ">" => a > rhs_i,
                "<" => a < rhs_i,
                "==" => a == rhs_i,
                "!=" => a != rhs_i,
                _ => true,
            };
        }
    }
    if matches!(op, "==" | "!=") {
        let actual_s = match actual {
            SpoeValue::String(s) => Some(s.as_str().to_string()),
            _ => None,
        };
        let rhs_s = rhs.trim_matches('"');
        if let Some(a) = actual_s {
            return match op {
                "==" => a == rhs_s,
                "!=" => a != rhs_s,
                _ => true,
            };
        }
    }
    true
}
