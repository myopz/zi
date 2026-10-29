// Catalog-derived usage cost (aligned with pi calculateCost).

use crate::types::{Model, Usage, UsageCost};

/// Fill `usage.cost` from the model catalog rates and token counts.
pub fn calculate_cost(model: &Model, usage: &mut Usage) {
    let cost = UsageCost {
        input: model.cost.input * usage.input as f64,
        output: model.cost.output * usage.output as f64,
        cache_read: model.cost.cache_read * usage.cache_read as f64,
        cache_write: model.cost.cache_write * usage.cache_write as f64,
        total: 0.0,
        total_from_provider: None,
    };
    usage.cost = UsageCost {
        total: cost.input + cost.output + cost.cache_read + cost.cache_write,
        ..cost
    };
}
