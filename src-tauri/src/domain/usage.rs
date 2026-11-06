//! 费用估算（说明书 §6.6 / M17）。
//!
//! 价格表来自内嵌 `resources/pricing.json`：`models[]` 为每百万 token 价格（人民币），
//! `search[]` 为每次检索/抓取价格；`usdToCny` 供未来以美元计价的供应商换算。
//! 未知模型 / 价格为 0 的本地模型一律记 0 成本。

use serde::Deserialize;

use crate::contract::errors::AppResult;
use crate::contract::types::{Currency, TokenUsage};

fn default_currency() -> String {
    "CNY".to_string()
}

/// 价格表中的单模型价格（每百万 token，人民币）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPrice {
    /// 模型 id。
    pub id: String,
    /// 输入单价 / 百万 token。
    #[serde(default)]
    pub input_per_m_tokens: f64,
    /// 输出单价 / 百万 token。
    #[serde(default)]
    pub output_per_m_tokens: f64,
    /// 缓存命中输入单价 / 百万 token（0 表示按输入价计）。
    #[serde(default)]
    pub cached_input_per_m_tokens: f64,
}

/// 价格表中的搜索/抓取单价（每次，人民币）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPrice {
    /// 服务 id。
    pub id: String,
    /// 每次调用价格。
    #[serde(default)]
    pub per_call: f64,
}

/// `resources/pricing.json` 文件结构（容忍未知字段）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PricingFile {
    /// 版本。
    #[serde(default)]
    pub version: u32,
    /// 展示货币。
    #[serde(default = "default_currency")]
    pub currency: String,
    /// 美元 → 人民币汇率。
    #[serde(default)]
    pub usd_to_cny: f64,
    /// 模型价格。
    #[serde(default)]
    pub models: Vec<ModelPrice>,
    /// 搜索价格。
    #[serde(default)]
    pub search: Vec<SearchPrice>,
}

/// 价格表（解析后只读）。
#[derive(Debug, Clone)]
pub struct PricingTable {
    file: PricingFile,
}

impl PricingTable {
    /// 从 JSON 文本解析。
    pub fn from_json(json: &str) -> AppResult<Self> {
        let file: PricingFile = serde_json::from_str(json)?;
        Ok(Self { file })
    }

    /// 汇率（美元 → 人民币）；未配置时回落到 1.0。
    pub fn usd_to_cny(&self, usd: f64) -> f64 {
        if self.file.usd_to_cny > 0.0 {
            usd * self.file.usd_to_cny
        } else {
            usd
        }
    }

    /// 展示货币。
    pub fn currency(&self) -> &str {
        &self.file.currency
    }

    /// 查模型价格。
    pub fn model_price(&self, model_id: &str) -> Option<&ModelPrice> {
        self.file
            .models
            .iter()
            .find(|price| price.id == model_id)
            .or_else(|| {
                // 兼容「供应商前缀 + 模型名」的写法，如 `deepseek/deepseek-flash`。
                self.file
                    .models
                    .iter()
                    .find(|price| model_id.ends_with(&price.id))
            })
    }

    /// 估算一次 LLM 调用的费用（人民币）。
    pub fn estimate(&self, model_id: &str, input: u64, output: u64, cached: u64) -> TokenUsage {
        let cost = match self.model_price(model_id) {
            Some(price) => {
                let cached_price = if price.cached_input_per_m_tokens > 0.0 {
                    price.cached_input_per_m_tokens
                } else {
                    price.input_per_m_tokens
                };
                // cached 是 input 的子集：未命中部分按 input 价，命中部分按缓存价。
                let uncached_input = input.saturating_sub(cached);
                (uncached_input as f64 * price.input_per_m_tokens
                    + cached as f64 * cached_price
                    + output as f64 * price.output_per_m_tokens)
                    / 1_000_000.0
            }
            None => 0.0,
        };
        TokenUsage {
            input_tokens: input,
            output_tokens: output,
            cached_input_tokens: cached,
            cost_estimate: round6(cost),
            currency: Currency::Cny,
        }
    }

    /// 单次搜索 / 抓取的费用（人民币）。
    pub fn search_cost(&self, service_id: &str) -> f64 {
        self.file
            .search
            .iter()
            .find(|price| price.id == service_id)
            .map(|price| round6(price.per_call))
            .unwrap_or(0.0)
    }
}

fn round6(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRICING: &str = r#"{
        "version": 1,
        "currency": "CNY",
        "usdToCny": 7.1,
        "updatedAt": "2026-11-04",
        "models": [
            { "id": "deepseek-flash", "inputPerMTokens": 1.07, "outputPerMTokens": 4.26, "cachedInputPerMTokens": 0.02 },
            { "id": "qwen-mt-lite", "inputPerMTokens": 0.6, "outputPerMTokens": 1.6, "cachedInputPerMTokens": 0 },
            { "id": "free-local", "inputPerMTokens": 0, "outputPerMTokens": 0, "cachedInputPerMTokens": 0 }
        ],
        "search": [
            { "id": "deepseek-search", "perCall": 0.0053 }
        ]
    }"#;

    fn table() -> PricingTable {
        PricingTable::from_json(PRICING).unwrap()
    }

    #[test]
    fn parses_pricing_with_unknown_fields() {
        let table = table();
        assert_eq!(table.currency(), "CNY");
        assert!(table.model_price("deepseek-flash").is_some());
        assert!(table.model_price("missing").is_none());
        assert!((table.usd_to_cny(1.0) - 7.1).abs() < 1e-9);
    }

    #[test]
    fn estimate_uses_million_token_prices() {
        let usage = table().estimate("deepseek-flash", 1_000_000, 0, 0);
        assert_eq!(usage.input_tokens, 1_000_000);
        assert!((usage.cost_estimate - 1.07).abs() < 1e-9);
        assert_eq!(usage.currency, Currency::Cny);

        let usage = table().estimate("deepseek-flash", 0, 1_000_000, 0);
        assert!((usage.cost_estimate - 4.26).abs() < 1e-9);

        let usage = table().estimate("deepseek-flash", 500_000, 500_000, 0);
        assert!((usage.cost_estimate - (0.535 + 2.13)).abs() < 1e-9);
    }

    #[test]
    fn cached_tokens_use_cached_price() {
        let usage = table().estimate("deepseek-flash", 1_000_000, 0, 1_000_000);
        assert!((usage.cost_estimate - 0.02).abs() < 1e-9);
        assert_eq!(usage.cached_input_tokens, 1_000_000);

        // 缓存价缺失（0）时回落到输入价。
        let usage = table().estimate("qwen-mt-lite", 1_000_000, 0, 1_000_000);
        assert!((usage.cost_estimate - 0.6).abs() < 1e-9);
    }

    #[test]
    fn zero_and_unknown_prices_are_free() {
        let usage = table().estimate("free-local", 10_000, 10_000, 0);
        assert_eq!(usage.cost_estimate, 0.0);
        let usage = table().estimate("unknown-model", 10_000, 10_000, 0);
        assert_eq!(usage.cost_estimate, 0.0);
        assert_eq!(usage.input_tokens, 10_000);
    }

    #[test]
    fn search_cost_per_call() {
        let table = table();
        assert!((table.search_cost("deepseek-search") - 0.0053).abs() < 1e-9);
        assert_eq!(table.search_cost("missing"), 0.0);
    }
}
