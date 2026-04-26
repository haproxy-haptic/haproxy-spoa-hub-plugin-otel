//! Extract incoming W3C TraceContext / Baggage from SPOE arguments.

use haproxy_spoa_hub_plugin_api::SpoeMessage;
use opentelemetry::propagation::Extractor;

/// Wraps a `SpoeMessage` so the OTel propagator can read header-named
/// SPOE arguments as if they were HTTP headers.
pub struct SpoeArgsExtractor<'a> {
    msg: &'a SpoeMessage,
    /// Optional prefix the operator added to the SPOE arg names. The
    /// addon's `extract <prefix>` directive maps to this.
    prefix: &'a str,
}

impl<'a> SpoeArgsExtractor<'a> {
    pub fn new(msg: &'a SpoeMessage, prefix: &'a str) -> Self {
        Self { msg, prefix }
    }
}

impl<'a> Extractor for SpoeArgsExtractor<'a> {
    fn get(&self, key: &str) -> Option<&str> {
        let name = if self.prefix.is_empty() {
            key.to_string()
        } else {
            format!("{}{}", self.prefix, key)
        };
        self.msg.get_string(&name)
    }

    fn keys(&self) -> Vec<&str> {
        self.msg.args.iter().map(|kv| kv.0.as_str()).collect()
    }
}
