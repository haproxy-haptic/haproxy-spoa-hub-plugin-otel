//! Inject outbound W3C TraceContext / Baggage as SPOE return variables.

use haproxy_spoa_hub_plugin_api::RString;
use haproxy_spoa_hub_plugin_api::{SpoeValue, TxnVariable, VarScope};
use opentelemetry::propagation::Injector;

/// W3C TraceContext §3.3.1.3 caps `tracestate` at 512 octets. The SDK's
/// propagator relays whatever was extracted without enforcing the cap,
/// so we apply it here on the outbound boundary.
const TRACESTATE_MAX_LEN: usize = 512;

/// Collects the propagator's writes and produces a `Vec<TxnVariable>`
/// the plugin returns to HAProxy. Each header becomes a session-scoped
/// variable named `<prefix><header>_out` (e.g. `traceparent_out`).
pub struct SpoeVarsInjector {
    prefix: String,
    suffix: &'static str,
    vars: Vec<TxnVariable>,
}

impl SpoeVarsInjector {
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            suffix: "_out",
            vars: Vec::with_capacity(3),
        }
    }

    pub fn into_vars(self) -> Vec<TxnVariable> {
        self.vars
    }
}

impl Injector for SpoeVarsInjector {
    fn set(&mut self, key: &str, value: String) {
        let value = if key.eq_ignore_ascii_case("tracestate") {
            truncate_tracestate(value)
        } else {
            value
        };
        let name = format!("{}{}{}", self.prefix, key, self.suffix);
        // Transaction-scoped: HTTP/2 streams multiplexed on the same
        // TCP session each get their own `txn`, so the outbound
        // TraceContext can't leak between concurrent streams. Session
        // scope would let two parallel streams share the variable slot.
        self.vars.push(TxnVariable {
            scope: VarScope::Transaction,
            name: RString::from(name),
            value: SpoeValue::String(RString::from(value)),
        });
    }
}

/// Apply W3C §3.3.1.3 truncation: drop trailing list-members until the
/// remainder fits in `TRACESTATE_MAX_LEN`. List-members are separated by
/// commas; the leftmost entries (most recent) are preserved.
fn truncate_tracestate(value: String) -> String {
    if value.len() <= TRACESTATE_MAX_LEN {
        return value;
    }
    let mut acc = String::with_capacity(TRACESTATE_MAX_LEN);
    for (i, member) in value.split(',').enumerate() {
        let needed = if i == 0 {
            member.len()
        } else {
            // +1 for the leading comma we'd append.
            acc.len() + 1 + member.len()
        };
        if needed > TRACESTATE_MAX_LEN {
            break;
        }
        if i > 0 {
            acc.push(',');
        }
        acc.push_str(member);
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::propagation::Injector;

    /// Regression test: every variable the propagator injects MUST be
    /// transaction-scoped, not session-scoped. Under HTTP/2 multiple
    /// streams share a single TCP session (= HAProxy `sess`), so a
    /// session-scoped slot would let stream A's `traceparent_out`
    /// overwrite stream B's. Transaction scope (`txn`) gives each
    /// stream its own slot.
    #[test]
    fn all_injected_vars_are_transaction_scoped() {
        let mut inj = SpoeVarsInjector::new("");
        inj.set("traceparent", "00-aa-bb-01".to_string());
        inj.set("tracestate", "vendor=value".to_string());
        inj.set("baggage", "key=value".to_string());

        let vars = inj.into_vars();
        assert!(!vars.is_empty(), "injector emitted nothing");
        for v in &vars {
            assert_eq!(
                v.scope,
                VarScope::Transaction,
                "var `{}` MUST be Transaction-scoped (got {:?}) — see commit log for the HTTP/2 multiplexing bug this guards against",
                v.name.as_str(),
                v.scope
            );
        }
    }

    #[test]
    fn short_tracestate_is_unchanged() {
        let input = "vendor1=a,vendor2=b".to_string();
        let out = truncate_tracestate(input.clone());
        assert_eq!(out, input);
    }

    #[test]
    fn oversized_tracestate_is_truncated_from_the_right() {
        // 64 entries, each ~20 chars, total ~1280 chars.
        let input = (0..64)
            .map(|i| format!("vendor{i:02}=value{i:02}"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(input.len() > TRACESTATE_MAX_LEN);
        let out = truncate_tracestate(input);
        assert!(out.len() <= TRACESTATE_MAX_LEN);
        // Leftmost (most recent) entries are preserved.
        assert!(out.starts_with("vendor00=value00"));
    }

    #[test]
    fn first_entry_alone_fits_even_at_512() {
        let big_one_entry = format!("k={}", "v".repeat(600));
        let out = truncate_tracestate(big_one_entry);
        // The first entry is too big on its own; we drop everything.
        assert_eq!(out, "");
    }
}
