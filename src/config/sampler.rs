//! Sampler config (data-model.md §1; addon vocabulary in
//! contracts/addon-directive-mapping.md §4.1).

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Sampler {
    AlwaysOn,
    AlwaysOff,
    TraceIdRatio { ratio: f64 },
    ParentBased { root: Box<Sampler> },
}

impl Default for Sampler {
    /// Default per research item R11: `parent_based(trace_id_ratio_based(1.0))`.
    fn default() -> Self {
        Sampler::ParentBased {
            root: Box::new(Sampler::TraceIdRatio { ratio: 1.0 }),
        }
    }
}
