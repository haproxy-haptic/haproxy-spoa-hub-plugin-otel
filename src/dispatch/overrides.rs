//! Per-message overrides of `Scope.spans[].root` and the sampler decision
//! via reserved SPOE arguments. See
//! `specs/002-trust-incoming-span/spec.md` and
//! `specs/002-trust-incoming-span/contracts/spoe-message-contract.md`.

use crate::health::HealthRegistry;
use haproxy_spoa_hub_plugin_api::SpoeMessage;

/// Reserved SPOE argument: per-message override of the active scope's
/// `Scope.spans[].root` setting. Value space: `"true"` | `"false"`.
pub const ARG_ROOT: &str = "_root";

/// Reserved SPOE argument: per-message override of the configured
/// sampler's decision. Value space: `"default"` | `"record"` | `"drop"`.
pub const ARG_SAMPLE: &str = "_sample";

/// Per-message sampling override.
///
/// `UseSampler` defers to the configured tracer pipeline's sampler — the
/// behaviour absent any override. `ForceRecord` and `ForceDrop`
/// short-circuit the sampler by passing an explicit `SamplingResult`
/// to `SpanBuilder::with_sampling_result`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleDecision {
    UseSampler,
    ForceRecord,
    ForceDrop,
}

/// Resolved per-message overrides for one request-side handler call.
/// Lives only on the request-side stack frame; not stored anywhere.
#[derive(Debug, Clone, Copy)]
pub struct OverrideResolution {
    /// Effective `root` setting for this transaction. Per-message
    /// argument wins over per-scope TOML default.
    pub effective_root: bool,
    /// Resolved sampling decision for this transaction.
    pub sample_decision: SampleDecision,
}

impl OverrideResolution {
    /// Resolve `_root` and `_sample` SPOE arguments against the active
    /// scope's TOML defaults. Malformed values are ignored (fail-open)
    /// and counted in the health registry, with one WARN log line per
    /// malformed argument.
    pub fn resolve(
        message: &SpoeMessage,
        scope_root_default: bool,
        scope_name: &str,
        health: &HealthRegistry,
    ) -> Self {
        let effective_root = parse_root(message, scope_name, health).unwrap_or(scope_root_default);
        let sample_decision = parse_sample(message, scope_name, health);
        Self {
            effective_root,
            sample_decision,
        }
    }
}

fn parse_root(message: &SpoeMessage, scope_name: &str, health: &HealthRegistry) -> Option<bool> {
    let raw = message.get_string(ARG_ROOT)?;
    match raw {
        "true" => Some(true),
        "false" => Some(false),
        other => {
            log::warn!(
                "otel plugin: malformed `{}` SPOE arg on scope `{}`: {:?}; falling back to scope default",
                ARG_ROOT,
                scope_name,
                truncate(other, 32)
            );
            health.malformed_override_inc(ARG_ROOT, scope_name);
            None
        }
    }
}

fn parse_sample(
    message: &SpoeMessage,
    scope_name: &str,
    health: &HealthRegistry,
) -> SampleDecision {
    let raw = match message.get_string(ARG_SAMPLE) {
        Some(s) => s,
        None => return SampleDecision::UseSampler,
    };
    match raw {
        "default" => SampleDecision::UseSampler,
        "record" => SampleDecision::ForceRecord,
        "drop" => SampleDecision::ForceDrop,
        other => {
            log::warn!(
                "otel plugin: malformed `{}` SPOE arg on scope `{}`: {:?}; falling back to configured sampler",
                ARG_SAMPLE,
                scope_name,
                truncate(other, 32)
            );
            health.malformed_override_inc(ARG_SAMPLE, scope_name);
            SampleDecision::UseSampler
        }
    }
}

/// Truncate `s` to at most `n` characters at a UTF-8 boundary. Returned
/// reference borrows from `s`; callers should not assume byte length
/// equals char count.
fn truncate(s: &str, n: usize) -> &str {
    if s.chars().count() <= n {
        return s;
    }
    let end = s.char_indices().nth(n).map(|(i, _)| i).unwrap_or(s.len());
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use abi_stable::std_types::{RHashMap, RString};
    use haproxy_spoa_hub_plugin_api::{SpoeMessage, SpoeValue};

    fn msg_with(args: &[(&str, &str)]) -> SpoeMessage {
        let mut m: RHashMap<RString, SpoeValue> = RHashMap::new();
        for (k, v) in args {
            m.insert(RString::from(*k), SpoeValue::String(RString::from(*v)));
        }
        SpoeMessage {
            name: RString::from("test"),
            args: m,
            stream_id: 0,
            frame_id: 0,
        }
    }

    #[test]
    fn absent_returns_scope_default() {
        let health = HealthRegistry::new();
        let msg = msg_with(&[]);
        let r = OverrideResolution::resolve(&msg, false, "test", &health);
        assert!(!r.effective_root);
        assert_eq!(r.sample_decision, SampleDecision::UseSampler);
        let r2 = OverrideResolution::resolve(&msg, true, "test", &health);
        assert!(r2.effective_root);
    }

    #[test]
    fn root_true_overrides_scope_default_false() {
        let health = HealthRegistry::new();
        let msg = msg_with(&[("_root", "true")]);
        let r = OverrideResolution::resolve(&msg, false, "test", &health);
        assert!(r.effective_root);
    }

    #[test]
    fn root_false_overrides_scope_default_true() {
        let health = HealthRegistry::new();
        let msg = msg_with(&[("_root", "false")]);
        let r = OverrideResolution::resolve(&msg, true, "test", &health);
        assert!(!r.effective_root);
    }

    #[test]
    fn root_malformed_falls_back_and_counts() {
        let health = HealthRegistry::new();
        let msg = msg_with(&[("_root", "yes")]);
        let r = OverrideResolution::resolve(&msg, true, "scope-a", &health);
        assert!(r.effective_root, "scope default applies on malformed");
        let snap = health.snapshot();
        assert_eq!(
            snap.malformed_override_total
                .get(&("_root".to_string(), "scope-a".to_string())),
            Some(&1)
        );
    }

    #[test]
    fn sample_record_drop_default() {
        let health = HealthRegistry::new();
        for (input, expected) in &[
            ("record", SampleDecision::ForceRecord),
            ("drop", SampleDecision::ForceDrop),
            ("default", SampleDecision::UseSampler),
        ] {
            let msg = msg_with(&[("_sample", *input)]);
            let r = OverrideResolution::resolve(&msg, false, "test", &health);
            assert_eq!(r.sample_decision, *expected, "input={input}");
        }
    }

    #[test]
    fn sample_malformed_falls_back_and_counts() {
        let health = HealthRegistry::new();
        let msg = msg_with(&[("_sample", "maybe")]);
        let r = OverrideResolution::resolve(&msg, false, "scope-b", &health);
        assert_eq!(r.sample_decision, SampleDecision::UseSampler);
        let snap = health.snapshot();
        assert_eq!(
            snap.malformed_override_total
                .get(&("_sample".to_string(), "scope-b".to_string())),
            Some(&1)
        );
    }

    #[test]
    fn truncate_handles_short_long_and_unicode() {
        assert_eq!(truncate("abc", 5), "abc");
        assert_eq!(truncate("abcdefgh", 4), "abcd");
        // 4 chars but each is 3 bytes → end byte index = 12 not 4
        assert_eq!(truncate("日本語テスト", 4), "日本語テ");
    }
}
