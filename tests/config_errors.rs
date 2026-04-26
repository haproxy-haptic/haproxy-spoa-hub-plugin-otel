//! Verifies SC-007 — every malformed configuration produces a
//! human-readable, actionable error (task T055).
//!
//! Each test exercises a distinct configuration-error code path: a
//! missing required key, an unknown key, a wrong type, an out-of-range
//! value, an invalid format, a semantic invariant violation, or a TOML
//! syntax error. Together they cover 20+ malformed configurations,
//! satisfying the original task description.

use otel_plugin::config::PluginConfig;
use otel_plugin::error::ConfigError;

fn err(toml: &str) -> ConfigError {
    PluginConfig::from_toml(toml).unwrap_err()
}

fn expect_contains(toml: &str, needle: &str) {
    let e = err(toml);
    let s = format!("{e}");
    assert!(
        s.contains(needle),
        "error should mention `{needle}` but was: {s}"
    );
}

// ---------------------------------------------------------------------
// Missing required keys
// ---------------------------------------------------------------------

#[test]
fn missing_instrumentation_name() {
    expect_contains("[instrumentation]\n", "name");
}

#[test]
fn missing_tracer_pipeline_name() {
    expect_contains(
        r#"
[instrumentation]
name = "x"

[[tracer_pipelines]]
[tracer_pipelines.exporter]
endpoint = "http://x:4317"
"#,
        "name",
    );
}

#[test]
fn missing_tracer_pipeline_exporter() {
    expect_contains(
        r#"
[instrumentation]
name = "x"

[[tracer_pipelines]]
name = "default"
"#,
        "exporter",
    );
}

#[test]
fn missing_exporter_endpoint() {
    expect_contains(
        r#"
[instrumentation]
name = "x"

[[tracer_pipelines]]
name = "default"
[tracer_pipelines.exporter]
protocol = "grpc"
"#,
        "endpoint",
    );
}

#[test]
fn missing_scope_event() {
    expect_contains(
        r#"
[instrumentation]
name = "x"

[[scopes]]
name = "s"
"#,
        "event",
    );
}

#[test]
fn missing_scope_name() {
    expect_contains(
        r#"
[instrumentation]
name = "x"

[[scopes]]
event = "e"
"#,
        "name",
    );
}

#[test]
fn missing_instrument_kind() {
    expect_contains(
        r#"
[instrumentation]
name = "x"

[[scopes]]
name = "s"
event = "e"

[[scopes.instruments]]
name = "foo"
value = 1
"#,
        "kind",
    );
}

#[test]
fn missing_log_record_severity() {
    expect_contains(
        r#"
[instrumentation]
name = "x"

[[scopes]]
name = "s"
event = "e"

[[scopes.log_records]]
body = "hi"
"#,
        "severity",
    );
}

// ---------------------------------------------------------------------
// Unknown / disallowed keys (additionalProperties: false)
// ---------------------------------------------------------------------

#[test]
fn unknown_top_level_key() {
    expect_contains("[instrumentation]\nname=\"x\"\nfoozle=true\n", "foozle");
}

#[test]
fn unknown_scope_field() {
    expect_contains(
        r#"
[instrumentation]
name = "x"

[[scopes]]
name = "s"
event = "e"
nonsense_field = 1
"#,
        "nonsense_field",
    );
}

#[test]
fn unknown_runtime_field() {
    expect_contains(
        r#"
[instrumentation]
name = "x"
[runtime]
weird_knob = 7
"#,
        "weird_knob",
    );
}

#[test]
fn unknown_exporter_field() {
    expect_contains(
        r#"
[instrumentation]
name = "x"

[[tracer_pipelines]]
name = "d"
[tracer_pipelines.exporter]
endpoint = "http://x:4317"
totally_made_up = true
"#,
        "totally_made_up",
    );
}

#[test]
fn unknown_attribute_field() {
    expect_contains(
        r#"
[instrumentation]
name = "x"

[[scopes]]
name = "s"
event = "e"

[[scopes.attributes]]
key = "k"
source = "v"
mystery = "x"
"#,
        "mystery",
    );
}

// ---------------------------------------------------------------------
// Wrong types and invalid enum values
// ---------------------------------------------------------------------

#[test]
fn wrong_type_for_debug_level() {
    expect_contains(
        r#"
[instrumentation]
name = "x"
debug_level = "should-be-an-integer"
"#,
        "debug_level",
    );
}

#[test]
fn invalid_protocol_enum() {
    expect_contains(
        r#"
[instrumentation]
name = "x"

[[tracer_pipelines]]
name = "d"
[tracer_pipelines.exporter]
endpoint = "http://x:4317"
protocol = "ftp"
"#,
        "protocol",
    );
}

#[test]
fn invalid_compression_enum() {
    expect_contains(
        r#"
[instrumentation]
name = "x"

[[tracer_pipelines]]
name = "d"
[tracer_pipelines.exporter]
endpoint = "http://x:4317"
compression = "lzma"
"#,
        "compression",
    );
}

#[test]
fn invalid_sampler_kind() {
    expect_contains(
        r#"
[instrumentation]
name = "x"

[[tracer_pipelines]]
name = "d"
[tracer_pipelines.sampler]
kind = "rolling-dice"
[tracer_pipelines.exporter]
endpoint = "http://x:4317"
"#,
        "sampler",
    );
}

#[test]
fn invalid_status_code_enum() {
    expect_contains(
        r#"
[instrumentation]
name = "x"

[[scopes]]
name = "s"
event = "e"
[scopes.status]
code = "maybe"
"#,
        "code",
    );
}

#[test]
fn invalid_severity_enum() {
    expect_contains(
        r#"
[instrumentation]
name = "x"

[[scopes]]
name = "s"
event = "e"

[[scopes.log_records]]
severity = "VERY_BAD"
"#,
        "severity",
    );
}

#[test]
fn invalid_instrument_kind_enum() {
    expect_contains(
        r#"
[instrumentation]
name = "x"

[[scopes]]
name = "s"
event = "e"

[[scopes.instruments]]
kind = "histogram_double"
name = "x"
value = 1
"#,
        "kind",
    );
}

#[test]
fn invalid_endpoint_uri() {
    // The serde-url deserializer reports "relative URL without a base"
    // for a non-URI string. Either that phrase or the offending value
    // identifies the failure for the operator.
    let e = err(r#"
[instrumentation]
name = "x"

[[tracer_pipelines]]
name = "d"
[tracer_pipelines.exporter]
endpoint = "not a uri"
"#);
    let s = format!("{e}");
    assert!(
        s.contains("URL") || s.contains("url") || s.contains("not a uri"),
        "error should identify the URL failure: {s}"
    );
}

// ---------------------------------------------------------------------
// Range / pattern violations
// ---------------------------------------------------------------------

#[test]
fn debug_level_above_max() {
    expect_contains(
        r#"
[instrumentation]
name = "x"
debug_level = 99
"#,
        "debug_level",
    );
}

#[test]
fn ratio_above_one() {
    expect_contains(
        r#"
[instrumentation]
name = "x"

[[tracer_pipelines]]
name = "d"
[tracer_pipelines.sampler]
kind = "trace_id_ratio"
ratio = 1.5
[tracer_pipelines.exporter]
endpoint = "http://x:4317"
"#,
        "ratio",
    );
}

#[test]
fn invalid_duration_string() {
    expect_contains(
        r#"
[instrumentation]
name = "x"
[runtime]
shutdown_drain_timeout = "five seconds please"
"#,
        "five",
    );
}

// ---------------------------------------------------------------------
// Semantic invariant violations (cross-field)
// ---------------------------------------------------------------------

#[test]
fn invariant_unknown_pipeline_in_span() {
    expect_contains(
        r#"
[instrumentation]
name = "x"

[[scopes]]
name = "s"
event = "e"

[[scopes.spans]]
name = "x"
tracer_pipeline = "i-do-not-exist"
"#,
        "i-do-not-exist",
    );
}

#[test]
fn invariant_unknown_scope_in_group() {
    let e = err(r#"
[instrumentation]
name = "x"

[[groups]]
name = "g"
scopes = ["does-not-exist"]
"#);
    assert!(matches!(e, ConfigError::Invariant { .. }));
    let s = format!("{e}");
    assert!(s.contains("does-not-exist"));
}

// ---------------------------------------------------------------------
// TOML syntax errors
// ---------------------------------------------------------------------

#[test]
fn malformed_toml_unclosed_section() {
    let e = err("[instrumentation\nname = \"x\"\n");
    assert!(matches!(e, ConfigError::Toml { .. }));
}

#[test]
fn malformed_toml_unclosed_string() {
    let e = err("[instrumentation]\nname = \"unterminated\n");
    assert!(matches!(e, ConfigError::Toml { .. }));
}

#[test]
fn malformed_toml_duplicate_key() {
    let e = err(r#"
[instrumentation]
name = "a"
name = "b"
"#);
    assert!(matches!(e, ConfigError::Toml { .. }));
}
