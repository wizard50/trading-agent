use crate::kraken_futures::*;
use crate::kraken_types::*;
use crate::workflow::{Action, RegimeInterpretation};
use std::error::Error;
use tracing::{error, info, instrument, warn};

#[derive(Debug, Clone)]
pub struct TradeManager {
    client: KrakenFuturesRestClient,
}

impl TradeManager {
    pub fn new(client: KrakenFuturesRestClient) -> Self {
        Self { client }
    }

    pub async fn execute_trade_decision(
        &self,
        interpretation: RegimeInterpretation,
        trade_config: TradeConfig,
    ) -> Result<(), Box<dyn Error>> {
        let current_pos = self.get_position(&trade_config.symbol).await?;

        match interpretation.action {
            Action::Buy => self.handle_buy(current_pos, &trade_config).await?,
            Action::Sell => self.handle_sell(current_pos, &trade_config).await?,
            Action::Hold => {
                info!(
                    event = "hold_position",
                    symbol = %trade_config.symbol,
                    reason = "Agent decided to hold based on interpretation",
                    "Holding current position"
                );
            }
        }

        Ok(())
    }

    async fn handle_buy(
        &self,
        current_pos: Option<OpenPosition>,
        trade_config: &TradeConfig,
    ) -> Result<(), Box<dyn Error>> {
        if let Some(pos) = &current_pos {
            if pos.side == "long" {
                info!(
                    event = "position_already_aligned",
                    current_side = "long",
                    desired_side = "long",
                    symbol = %trade_config.symbol,
                    "Position already aligned with signal, no action needed"
                );
                return Ok(());
            } else {
                info!(
                    event = "position_reversal",
                    from_side = "short",
                    to_side = "long",
                    symbol = %trade_config.symbol,
                    "Reversing position: closing short to open long"
                );
                self.close_trade(pos).await?;
            }
        }

        self.open_trade(trade_config).await?;
        info!(
            event = "buy_executed",
            symbol = %trade_config.symbol,
            size = trade_config.size,
            side = "buy",
            "Successfully executed BUY order"
        );
        Ok(())
    }

    async fn handle_sell(
        &self,
        current_pos: Option<OpenPosition>,
        trade_config: &TradeConfig,
    ) -> Result<(), Box<dyn Error>> {
        if let Some(pos) = &current_pos {
            if pos.side == "short" {
                info!(
                    event = "position_already_aligned",
                    current_side = "short",
                    desired_side = "short",
                    symbol = %trade_config.symbol,
                    "Position already aligned with signal, no action needed"
                );
                return Ok(());
            } else {
                info!(
                    event = "position_reversal",
                    from_side = "long",
                    to_side = "short",
                    symbol = %trade_config.symbol,
                    "Reversing position: closing long to open short"
                );
                self.close_trade(pos).await?;
            }
        }

        self.open_trade(trade_config).await?;
        info!(
            event = "sell_executed",
            symbol = %trade_config.symbol,
            size = trade_config.size,
            side = "sell",
            "Successfully executed SELL order"
        );
        Ok(())
    }

    pub async fn get_position(&self, symbol: &str) -> Result<Option<OpenPosition>, Box<dyn Error>> {
        let resp: OpenPositionsResponse = self.client.get_open_positions().await?;
        Ok(resp.open_positions.into_iter().find(|p| p.symbol == symbol))
    }

    #[instrument(skip(self, trade_config), fields(symbol = %trade_config.symbol, side = ?trade_config.side))]
    pub async fn open_trade(&self, trade_config: &TradeConfig) -> Result<Trade, Box<dyn Error>> {
        info!(
            event = "trade_open_start",
            symbol = %trade_config.symbol,
            side = ?trade_config.side,
            size = trade_config.size,
            entry_type = ?trade_config.entry,
            "Initiating trade opening sequence"
        );

        self.client
            .set_leverage(&trade_config.symbol, trade_config.leverage.clone())
            .await?;

        info!(
            event = "leverage_set",
            leverage = ?trade_config.leverage,
            "Leverage configured"
        );

        let sl_order = self.get_stop_order(trade_config);
        if let Some(sl) = &sl_order {
            let sl_resp: SendOrderResponse = self.client.send_order(sl).await?;
            if sl_resp.send_status.status != "placed" {
                error!(
                    event = "stop_loss_failed",
                    status = %sl_resp.send_status.status,
                    symbol = %trade_config.symbol,
                    "Failed to place stop loss order"
                );
                match self
                    .client
                    .cancel_all_orders(Some(&trade_config.symbol))
                    .await
                {
                    Ok(_) => warn!(
                        event = "cleanup_after_failed_sl",
                        symbol = %trade_config.symbol,
                        "Cancelled all orders after failed SL placement"
                    ),
                    Err(e) => error!(
                        event = "cleanup_failed",
                        symbol = %trade_config.symbol,
                        error = %e,
                        "Failed to cancel orders after SL failure"
                    ),
                }
                return Err(format!("Failed to place SL: {}", sl_resp.send_status.status).into());
            }
        }

        let tp_order = self.get_take_profit_order(trade_config);
        if let Some(tp) = &tp_order {
            let tp_resp: SendOrderResponse = self.client.send_order(tp).await?;
            if tp_resp.send_status.status != "placed" {
                error!(
                    event = "take_profit_failed",
                    status = %tp_resp.send_status.status,
                    symbol = %trade_config.symbol,
                    "Failed to place take profit order"
                );
                match self
                    .client
                    .cancel_all_orders(Some(&trade_config.symbol))
                    .await
                {
                    Ok(_) => warn!(
                        event = "cleanup_after_failed_tp",
                        symbol = %trade_config.symbol,
                        "Cancelled all orders after failed TP placement"
                    ),
                    Err(e) => error!(
                        event = "cleanup_failed",
                        symbol = %trade_config.symbol,
                        error = %e,
                        "Failed to cancel orders after TP failure"
                    ),
                }
                return Err(format!("Failed to place TP: {}", tp_resp.send_status.status).into());
            }

            info!(
                event = "take_profit_placed",
                order_id = ?tp_resp.send_status.order_id,
                trigger_price = tp.stop_price,
                symbol = %trade_config.symbol,
                "Take profit order placed successfully"
            );
        }

        let entry_order = self.get_entry_order(trade_config);
        let entry_resp: SendOrderResponse = self.client.send_order(&entry_order).await?;
        if entry_resp.send_status.status != "placed" {
            error!(
                event = "entry_order_failed",
                status = %entry_resp.send_status.status,
                symbol = %trade_config.symbol,
                "Failed to place entry order"
            );
            match self
                .client
                .cancel_all_orders(Some(&trade_config.symbol))
                .await
            {
                Ok(_) => warn!(
                    event = "cleanup_after_failed_entry",
                    symbol = %trade_config.symbol,
                    "Cancelled all orders after failed Entry placement"
                ),
                Err(e) => error!(
                    event = "cleanup_failed",
                    symbol = %trade_config.symbol,
                    error = %e,
                    "Failed to cancel orders after Entry failure"
                ),
            }
            return Err(format!("Failed to place entry: {}", entry_resp.send_status.status).into());
        }

        info!(
            event = "trade_open_complete",
            entry_order_id = ?entry_resp.send_status.order_id,
            sl_placed = sl_order.is_some(),
            tp_placed = tp_order.is_some(),
            symbol = %trade_config.symbol,
            side = ?trade_config.side,
            size = trade_config.size,
            "Trade opened successfully with entry, SL, and TP orders"
        );

        Ok(Trade {
            config: trade_config.clone(),
            entry: entry_order,
            stop_loss: sl_order,
            take_profit: tp_order,
        })
    }

    #[instrument(skip(self, position), fields(symbol = %position.symbol, side = %position.side, size = position.size))]
    pub async fn close_trade(&self, position: &OpenPosition) -> Result<(), Box<dyn Error>> {
        let symbol = position.symbol.as_str();

        info!(
            event = "position_close_start",
            symbol = %symbol,
            side = %position.side,
            size = position.size,
            price = position.price,
            "Initiating position close sequence"
        );

        let close_side = match position.side.as_str() {
            "long" => "sell", // close long by selling
            "short" => "buy", // close short by buying
            other => {
                error!(
                    event = "unknown_position_side",
                    side = other,
                    symbol = %symbol,
                    "Unknown position side encountered"
                );
                return Err(format!("Unknown position side: {}", other).into());
            }
        };

        let close_order = SendOrder {
            order_type: "mkt".into(),
            symbol: symbol.to_string(),
            side: close_side.to_string(),
            size: position.size.abs(),
            limit_price: None,
            stop_price: None,
            trigger_signal: None,
            reduce_only: Some(true),
            cli_ord_id: None,
        };

        let resp = self.client.send_order(&close_order).await?;
        if resp.send_status.status != "placed" {
            error!(
                event = "close_order_failed",
                status = %resp.send_status.status,
                symbol = %symbol,
                "Failed to place close order"
            );
            return Err(format!("Failed to place close order: {}", resp.send_status.status).into());
        }

        info!(
            event = "close_order_placed",
            order_id = ?resp.send_status.order_id,
            symbol = %symbol,
            "Closing market order placed successfully"
        );

        match self.client.cancel_all_orders(Some(symbol)).await {
            Ok(_) => info!(
                event = "cleanup_orders_cancelled",
                symbol = %symbol,
                "Cancelled all open orders (SL/TP/unfilled) after position close"
            ),
            Err(e) => {
                warn!(
                    event = "cleanup_orders_failed",
                    symbol = %symbol,
                    error = %e,
                    "Failed to cancel orders after position close"
                );
            }
        }

        info!(
            event = "position_close_complete",
            symbol = %symbol,
            "Position close sequence completed successfully"
        );

        Ok(())
    }

    pub fn get_entry_order(&self, trade_config: &TradeConfig) -> SendOrder {
        match &trade_config.entry {
            EntryConfig::Market => SendOrder {
                order_type: "mkt".into(),
                symbol: trade_config.symbol.clone(),
                side: trade_config.side_str().into(),
                size: trade_config.size,
                limit_price: None,
                stop_price: None,
                trigger_signal: None,
                reduce_only: Some(false),
                cli_ord_id: None,
            },
            EntryConfig::Limit { limit_price } => SendOrder {
                order_type: "lmt".into(),
                symbol: trade_config.symbol.clone(),
                side: trade_config.side_str().into(),
                size: trade_config.size,
                limit_price: Some(*limit_price),
                stop_price: None,
                trigger_signal: None,
                reduce_only: Some(false),
                cli_ord_id: None,
            },
        }
    }

    pub fn get_stop_order(&self, trade_config: &TradeConfig) -> Option<SendOrder> {
        let sl: &StopConfig = trade_config.stop_loss.as_ref()?;

        Some(SendOrder {
            order_type: "stp".into(),
            symbol: trade_config.symbol.clone(),
            side: trade_config.toggled_side_str().into(),
            size: trade_config.size,
            limit_price: sl.limit_price,
            stop_price: Some(sl.trigger_price),
            trigger_signal: Some(sl.trigger_signal.as_str().into()),
            reduce_only: Some(false),
            cli_ord_id: None,
        })
    }

    pub fn get_take_profit_order(&self, trade_config: &TradeConfig) -> Option<SendOrder> {
        let tp: &TakeProfitConfig = trade_config.take_profit.as_ref()?;

        Some(SendOrder {
            order_type: "take_profit".into(),
            symbol: trade_config.symbol.clone(),
            side: trade_config.toggled_side_str().into(),
            size: trade_config.size,
            limit_price: tp.limit_price,
            stop_price: Some(tp.trigger_price),
            trigger_signal: Some(tp.trigger_signal.as_str().into()),
            reduce_only: Some(true),
            cli_ord_id: None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Trade {
    pub config: TradeConfig,
    pub entry: SendOrder,
    pub stop_loss: Option<SendOrder>,
    pub take_profit: Option<SendOrder>,
}

#[derive(Debug, Clone)]
pub struct TradeConfig {
    pub symbol: String,
    pub side: OrderSide,
    pub size: f64,
    pub entry: EntryConfig,
    pub stop_loss: Option<StopConfig>,
    pub take_profit: Option<TakeProfitConfig>,
    pub leverage: Option<u8>,
}

impl TradeConfig {
    fn side_str(&self) -> &'static str {
        match self.side {
            OrderSide::Buy => "buy",
            OrderSide::Sell => "sell",
        }
    }

    fn toggled_side_str(&self) -> &'static str {
        match self.side {
            OrderSide::Buy => "sell",
            OrderSide::Sell => "buy",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OrderSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone)]
pub enum EntryConfig {
    Market,
    Limit { limit_price: f64 },
}

#[derive(Debug, Clone)]
pub struct StopConfig {
    pub trigger_price: f64,
    pub trigger_signal: TriggerSignal,
    pub limit_price: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct TakeProfitConfig {
    pub trigger_price: f64,
    pub trigger_signal: TriggerSignal,
    pub limit_price: Option<f64>,
}

#[derive(Debug, Clone, Copy)]
pub enum TriggerSignal {
    Mark,
    Index,
    Last,
}

impl TriggerSignal {
    fn as_str(&self) -> &'static str {
        match self {
            TriggerSignal::Mark => "mark",
            TriggerSignal::Index => "index",
            TriggerSignal::Last => "last",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::kraken_futures::KrakenFuturesRestClient;

    #[tokio::test]
    #[ignore = "Requires kraken CLI installed + demo account. Run manually only."]
    async fn test_open_trade_on_demo() {
        let config = Config::load().expect("No config found");

        let client = KrakenFuturesRestClient::with_auth(
            KrakenEnvironment::Demo,
            config.kraken_futures_api_key,
            config.kraken_futures_api_secret,
        );

        let trade_config = TradeConfig {
            symbol: "PF_XBTUSD".to_string(),
            side: OrderSide::Buy,
            size: 0.001,
            entry: EntryConfig::Market,
            stop_loss: Some(StopConfig {
                trigger_price: 76000.0,
                trigger_signal: TriggerSignal::Mark,
                limit_price: Some(75000.0),
            }),
            take_profit: Some(TakeProfitConfig {
                trigger_price: 80000.0,
                trigger_signal: TriggerSignal::Mark,
                limit_price: Some(81000.0),
            }),
            leverage: Some(1),
        };

        let trade_manager = TradeManager::new(client);
        let result = trade_manager.open_trade(&trade_config).await;

        match result {
            Ok(value) => {
                println!("✅ open_trade succeeded on demo:");
                println!("{:#?}", value);
            }
            Err(e) => panic!("open_trade failed: {}", e),
        }
    }

    #[tokio::test]
    #[ignore = "Requires kraken CLI installed + demo account. Run manually only."]
    async fn test_close_trade_on_demo() {
        let config = Config::load().expect("No config found");

        let client = KrakenFuturesRestClient::with_auth(
            KrakenEnvironment::Demo,
            config.kraken_futures_api_key,
            config.kraken_futures_api_secret,
        );

        let trade_manager = TradeManager::new(client);
        let current_pos = trade_manager
            .get_position("PF_XBTUSD")
            .await
            .unwrap()
            .unwrap();

        let result = trade_manager.close_trade(&current_pos).await;

        match result {
            Ok(()) => println!("✅ close_trade succeeded on demo"),
            Err(e) => panic!("close_trade failed: {}", e),
        }
    }
}
