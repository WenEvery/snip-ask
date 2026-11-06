//! 用量计量（M17）：用内嵌 `resources/pricing.json` + `domain::usage` 估算费用。

use crate::contract::errors::AppResult;
use crate::contract::types::TokenUsage;
use crate::domain::usage::PricingTable;

/// 用量计量器。
#[derive(Debug, Clone)]
pub struct UsageMeter {
    table: PricingTable,
}

impl UsageMeter {
    /// 从内置价格表构造。
    pub fn from_builtin() -> AppResult<Self> {
        Ok(Self {
            table: PricingTable::from_json(crate::resources::PRICING)?,
        })
    }

    /// 从外部 JSON 构造（测试 / 覆盖价格表）。
    pub fn from_json(json: &str) -> AppResult<Self> {
        Ok(Self {
            table: PricingTable::from_json(json)?,
        })
    }

    /// 估算一次调用的费用。
    pub fn estimate(&self, model_id: &str, input: u64, output: u64, cached: u64) -> TokenUsage {
        self.table.estimate(model_id, input, output, cached)
    }

    /// 单次搜索 / 抓取费用。
    pub fn search_cost(&self, service_id: &str) -> f64 {
        self.table.search_cost(service_id)
    }

    /// 价格表引用。
    pub fn table(&self) -> &PricingTable {
        &self.table
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_pricing_is_parseable() {
        let meter = UsageMeter::from_builtin().unwrap();
        let usage = meter.estimate("deepseek-flash", 1_000_000, 0, 0);
        assert!(usage.cost_estimate > 0.0);
        assert!(meter.search_cost("deepseek-search") > 0.0);
    }

    #[test]
    fn external_pricing_overrides() {
        let json = r#"{
            "version": 1, "currency": "CNY", "usdToCny": 7.0,
            "models": [{ "id": "m1", "inputPerMTokens": 2.0, "outputPerMTokens": 4.0, "cachedInputPerMTokens": 0 }],
            "search": []
        }"#;
        let meter = UsageMeter::from_json(json).unwrap();
        let usage = meter.estimate("m1", 1_000_000, 1_000_000, 0);
        assert!((usage.cost_estimate - 6.0).abs() < 1e-9);
    }
}
