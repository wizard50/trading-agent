use crate::tool::Tool;
use crate::utils::with_exponential_backoff;
use async_trait::async_trait;
use reqwest;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::error::Error;
use std::time::Duration;

pub const BASE_URL: &str = "https://gamma-api.polymarket.com";

#[derive(Clone)]
pub struct PolymarketClient {
    client: reqwest::Client,
}

impl PolymarketClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    pub async fn fetch_events(&self) -> Result<Vec<PolymarketEvent>, Box<dyn Error>> {
        let url = format!(
            "{}/events?active=true&closed=false&order=volume_24hr&ascending=false&limit=30&tag_id=21&related_tags=true",
            BASE_URL
        );

        let http_resp = self
            .client
            .get(url)
            .timeout(Duration::from_secs(10))
            .send()
            .await?;

        let events: Vec<PolymarketEvent> = http_resp.json().await?;
        let mut relevant_events = Vec::new();

        for event in &events {
            let title = event.title.as_deref().unwrap_or("").to_lowercase();
            let is_relevant = title.contains("bitcoin")
                || title.contains("btc")
                || title.contains("crypto")
                || title.contains("eth")
                || title.contains("ethereum")
                || title.contains("fed")
                || title.contains("macro");

            if !is_relevant {
                continue;
            }

            let filtered_markets: Vec<PolymarketMarket> = event
                .markets
                .iter()
                .filter(|market| market.active.unwrap_or(false) && !market.closed.unwrap_or(true))
                .cloned()
                .collect();

            if !filtered_markets.is_empty() {
                let mut filtered_event = event.clone();
                filtered_event.markets = filtered_markets;
                relevant_events.push(filtered_event);
            }
        }

        Ok(relevant_events)
    }
}

pub fn build_simple_sentiment(events: &[PolymarketEvent]) -> String {
    let mut all_markets: Vec<(&PolymarketMarket, f64)> = Vec::new();

    // flatten markets
    for event in events {
        for market in &event.markets {
            let vol = market.volume_24hr.unwrap_or(0.0);
            if vol > 0.0 && market.question.is_some() {
                all_markets.push((market, vol));
            }
        }
    }
    all_markets.sort_by(|a, b| b.1.total_cmp(&a.1)); // descending order

    let mut lines = Vec::new();
    for (market, vol) in all_markets.iter().take(10) {
        let [yes_price, _] = market.parse_outcome_prices();
        let question = market.question.as_deref().unwrap_or("?");

        lines.push(format!(
            "\"{}\" ({:.0}%) vol=${:.1}",
            question.chars().take(100).collect::<String>(),
            yes_price * 100.0,
            vol
        ));
    }

    if lines.is_empty() {
        "No relevant Polymarket activity right now".to_string()
    } else {
        lines.join(" | ")
    }
}

#[derive(Clone)]
pub struct PolymarketTool {
    client: PolymarketClient,
}

impl PolymarketTool {
    pub fn new() -> Self {
        Self {
            client: PolymarketClient::new(),
        }
    }
}

#[async_trait(?Send)]
impl Tool for PolymarketTool {
    fn name(&self) -> &str {
        "get_polymarket_sentiment"
    }

    fn description(&self) -> &str {
        "Fetches the latest Polymarket crowd sentiment for BTC/ETH/crypto markets. \
         Returns the top 10 highest-volume active markets with their Yes-probability and volume. \
         Use this tool when you want real-money crowd opinion or when the technical regimes are unclear/contradictory."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    async fn execute(&self, _args: Value) -> Result<Value, Box<dyn Error>> {
        let events =
            with_exponential_backoff(3, || async { self.client.fetch_events().await }).await?;
        let sentiment = build_simple_sentiment(&events);

        Ok(serde_json::json!({
            "sentiment": sentiment,
            "note": "This is real-money crowd wisdom from Polymarket. High volume = higher reliability."
        }))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolymarketEvent {
    pub title: Option<String>,
    #[serde(rename = "volume24hr")]
    pub volume_24hr: Option<f64>,
    pub tags: Vec<PolymarketTag>,
    pub markets: Vec<PolymarketMarket>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolymarketTag {
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolymarketMarket {
    pub question: Option<String>,
    #[serde(rename = "outcomePrices")]
    pub outcome_prices: Option<String>, // '["0.65", "0.35"]' → Yes price first
    #[serde(rename = "volume24hr")]
    pub volume_24hr: Option<f64>,
    pub active: Option<bool>,
    pub closed: Option<bool>,
}

impl PolymarketMarket {
    pub fn parse_outcome_prices(&self) -> [f64; 2] {
        let prices_str = match &self.outcome_prices {
            Some(s) if !s.is_empty() && s != "[]" => s,
            _ => return [0.5, 0.5], // fallback
        };

        let prices: Vec<String> = serde_json::from_str(prices_str)
            .unwrap_or_else(|_| vec!["0.5".to_string(), "0.5".to_string()]);

        let mut iter = prices.into_iter().map(|s| s.parse::<f64>().unwrap_or(0.5));
        [iter.next().unwrap_or(0.5), iter.next().unwrap_or(0.5)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_fetch_events() {
        let client = PolymarketClient::new();
        let events: Vec<PolymarketEvent> = client.fetch_events().await.unwrap();

        println!("{:?}", build_simple_sentiment(&events));
    }
}
