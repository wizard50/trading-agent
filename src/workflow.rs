use crate::agent::Agent;
use crate::kraken_futures::KrakenFuturesRestClient;
use crate::kraken_types::*;
use crate::llm::LlmClient;
use crate::market_regimes::{MarketRegimes, get_regimes};
use crate::polymarket::PolymarketTool;
use crate::ta::{DataFrame, OrderBook};
use crate::tool::{FinalAnswerTool, ToolRegistry};
use crate::trade::{
    EntryConfig, OrderSide, StopConfig, TakeProfitConfig, TradeConfig, TradeManager, TriggerSignal,
};
use serde::Deserialize;
use serde_json::json;
use std::error::Error;
use tracing::{debug, error, info, instrument, warn};

pub struct Workflow {
    pub provider: LlmClient,
    pub cycle_duration: u64,
    pub symbol: String,
    pub kraken_client: KrakenFuturesRestClient,
    pub trade_manager: TradeManager,
}

impl Workflow {
    pub fn new(symbol: &str, provider: LlmClient, kraken_client: KrakenFuturesRestClient) -> Self {
        Workflow {
            provider,
            cycle_duration: 15 * 60 as u64, // default 15min
            symbol: symbol.to_string(),
            kraken_client: kraken_client.clone(),
            trade_manager: TradeManager::new(kraken_client.clone()),
        }
    }

    pub fn with_duration(
        symbol: &str,
        provider: LlmClient,
        kraken_client: KrakenFuturesRestClient,
        cycle_duration: u64,
    ) -> Self {
        Workflow {
            provider,
            cycle_duration,
            symbol: symbol.to_string(),
            kraken_client: kraken_client.clone(),
            trade_manager: TradeManager::new(kraken_client.clone()),
        }
    }

    #[instrument(skip(self), fields(symbol = %self.symbol))]
    pub async fn start_workflow(&mut self) -> Result<(), Box<dyn Error>> {
        let mut regime_agent = create_regime_agent(self.provider.clone());

        loop {
            match self.get_interpretation(&mut regime_agent).await {
                Ok(interpretation) => {
                    if (interpretation.confidence_level == ConfidenceLevel::High
                        || interpretation.confidence_level == ConfidenceLevel::VeryHigh)
                        && interpretation.action != Action::Hold
                    {
                        info!(
                            event = "regime_interpretation",
                            action = ?interpretation.action,
                            confidence = ?interpretation.confidence_level,
                            risk = ?interpretation.risk_level,
                            reason = %interpretation.reason,
                            symbol = %self.symbol,
                            "Strong signal → executing trade"
                        );

                        self.execute_trade(interpretation).await;
                    } else {
                        warn!(
                            event = "low_confidence_or_hold",
                            action = ?interpretation.action,
                            confidence = ?interpretation.confidence_level,
                            risk = ?interpretation.risk_level,
                            reason = %interpretation.reason,
                            "Skipping trade (HOLD or confidence too low)"
                        );
                    }
                }
                Err(e) => {
                    error!(
                        event = "cycle_failed",
                        error = %e,
                        "Failed to get regime interpretation - skipping this cycle"
                    );
                }
            }

            tokio::time::sleep(tokio::time::Duration::from_secs(self.cycle_duration)).await;
        }
    }

    async fn execute_trade(&mut self, interpretation: RegimeInterpretation) {
        let trade_config: TradeConfig = match self.build_trade_config(&interpretation, 0.005).await
        {
            Ok(config) => config,
            Err(e) => {
                error!(
                    event = "build_trade_config_failed",
                    error = %e,
                    symbol = %self.symbol,
                    "Failed to build trade config"
                );
                return;
            }
        };

        let symbol = trade_config.symbol.clone();
        let side = trade_config.side;
        let size = trade_config.size;

        match self
            .trade_manager
            .execute_trade_decision(interpretation, trade_config)
            .await
        {
            Ok(_) => {
                info!(
                    event = "trade_execution_success",
                    symbol = %symbol,
                    side = ?side,
                    size = size,
                    "Trade executed successfully"
                );
            }
            Err(e) => {
                error!(
                    event = "trade_execution_failed",
                    error = %e,
                    symbol = %symbol,
                    "Failed to execute trade"
                );
            }
        }
    }

    async fn get_interpretation(
        &mut self,
        regime_agent: &mut Agent,
    ) -> Result<RegimeInterpretation, Box<dyn Error>> {
        regime_agent.clear_history();

        let df_15m: DataFrame = self
            .kraken_client
            .get_candles(&self.symbol, "15m", Some(200), None, None)
            .await?
            .into();

        let df_1h: DataFrame = self
            .kraken_client
            .get_candles(&self.symbol, "1h", Some(200), None, None)
            .await?
            .into();

        let order_book: OrderBook = self
            .kraken_client
            .get_order_book(&self.symbol)
            .await?
            .into();

        let market_regimes: MarketRegimes = get_regimes(&df_15m, &df_1h, &order_book)?;

        let prompt = format!(
            "Current technical regimes for {}:\n{}",
            &self.symbol,
            market_regimes.summary()
        );

        debug!(
            event = "llm_prompt",
            prompt = %prompt,
            "Sending regime analysis prompt to LLM"
        );
        let raw_response = regime_agent.run(&prompt).await?;
        debug!(
            event = "llm_response_raw",
            response = %raw_response,
            "Received raw response from LLM"
        );

        let interpretation: RegimeInterpretation = serde_json::from_str(&raw_response)
            .map_err(|e| format!("Failed to parse Regime JSON: {}", e))?;
        info!(
            event = "regime_interpretation",
            action = ?interpretation.action,
            confidence = ?interpretation.confidence_level,
            risk = ?interpretation.risk_level,
            reason = %interpretation.reason,
            "Successfully parsed regime interpretation"
        );

        Ok(interpretation)
    }

    pub async fn build_trade_config(
        &self,
        interpretation: &RegimeInterpretation,
        trigger_price_buffer: f64,
    ) -> Result<TradeConfig, Box<dyn Error>> {
        let ticker: Ticker = self
            .kraken_client
            .get_current_price(&self.symbol)
            .await
            .map_err(|e| format!("Failed to get current price: {e} — skipping trade"))?;

        let side = interpretation
            .action
            .to_order_side()
            .ok_or("Hold — no trade action")?;

        let (leverage, size, sl_distance, tp_distance) = match interpretation.risk_level {
            RiskLevel::Low => (Some(1u8), 0.01, 0.025, 0.060), // safe & wide
            RiskLevel::Medium => (Some(3u8), 0.02, 0.015, 0.045), // balanced
            RiskLevel::High => (Some(5u8), 0.03, 0.010, 0.040), // aggressive
        };

        // Calculate trigger prices relative to current price
        let current_price = ticker.last;
        let (sl_trigger_price, tp_trigger_price) = match side {
            OrderSide::Buy => (
                current_price * (1.0 - sl_distance), // SL below for long
                current_price * (1.0 + tp_distance), // TP above for long
            ),
            OrderSide::Sell => (
                current_price * (1.0 + sl_distance), // SL above for short
                current_price * (1.0 - tp_distance), // TP below for short
            ),
        };

        let sl_limit_price = sl_trigger_price * (1.0 - trigger_price_buffer);
        let tp_limit_price = tp_trigger_price * (1.0 + trigger_price_buffer);

        let trade_config = TradeConfig {
            symbol: self.symbol.to_string(),
            side: side,
            size: size,
            entry: EntryConfig::Market,
            stop_loss: Some(StopConfig {
                trigger_price: sl_trigger_price,
                trigger_signal: TriggerSignal::Mark,
                limit_price: Some(sl_limit_price),
            }),
            take_profit: Some(TakeProfitConfig {
                trigger_price: tp_trigger_price,
                trigger_signal: TriggerSignal::Mark,
                limit_price: Some(tp_limit_price),
            }),
            leverage: leverage,
        };

        Ok(trade_config)
    }
}

pub fn create_regime_agent(provider: LlmClient) -> Agent {
    let final_answer_schema = json!({
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": ["BUY", "SELL", "HOLD"]
            },
            "reason": {
                "type": "string",
                "description": "One very short sentence (max 15 words)"
            },
            "confidence_level": {
                "type": "string",
                "enum": ["VERYLOW", "LOW", "MEDIUM", "HIGH", "VERYHIGH"]
            },
            "risk_level": {
                "type": "string",
                "enum": ["LOW", "MEDIUM", "HIGH"]
            }
        },
        "required": ["action", "reason", "confidence_level", "risk_level"],
        "additionalProperties": false
    });

    let system_prompt = r#"You are an extremely concise, battle-tested crypto regime interpreter.

You receive ONLY technical regimes as input.
You have access to two tools:
- get_polymarket_sentiment (optional)
- final_answer (mandatory for final output)

YOUR JOB:
1. Analyze the technical regimes provided.
2. If the regime is unclear, ranging, weak, mixed or contradictory → call get_polymarket_sentiment (at most once).
3. Once you have enough information, you MUST call the final_answer tool.

CRITICAL RULES:
- Call final_answer EXACTLY once at the very end.
- Do NOT call any more tools after final_answer.
- Never mention "Polymarket" or tool names in the "reason" field.
- Never output normal text, markdown, or JSON directly — only use tool calls.

Rules for confidence_level and risk_level are the same as before.

Examples of correct final_answer tool call:
{"action":"BUY","reason":"strong bullish EMA200 bias with volatility expansion","confidence_level":"VERYHIGH","risk_level":"HIGH"}
{"action":"HOLD","reason":"ranging market with no clear directional bias","confidence_level":"MEDIUM","risk_level":"LOW"}
"#;

    let mut tools = ToolRegistry::new();
    tools.register(PolymarketTool::new());
    tools.register(FinalAnswerTool::new(final_answer_schema));

    // Schema stays None (we use the tool instead)
    Agent::new("Regime Agent", system_prompt, provider, tools, None)
}

#[derive(Debug, Clone, Deserialize)]
pub struct RegimeInterpretation {
    pub action: Action, // "BUY", "SELL", "HOLD"
    pub reason: String, // short 1-sentence explanation

    pub confidence_level: ConfidenceLevel, // ← 5 levels, your new filter
    pub risk_level: RiskLevel,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub enum Action {
    Buy,
    Sell,
    Hold,
}

impl Action {
    pub fn to_order_side(&self) -> Option<OrderSide> {
        match self {
            Action::Buy => Some(OrderSide::Buy),
            Action::Sell => Some(OrderSide::Sell),
            Action::Hold => None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub enum ConfidenceLevel {
    VeryLow,
    Low,
    Medium,
    High,
    VeryHigh,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}
