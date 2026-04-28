use crate::ta::{Candle, OrderBook};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KrakenEnvironment {
    #[default]
    Live,
    Demo,
}

#[derive(Serialize, Debug, Clone)]
pub struct SendOrder {
    pub symbol: String,
    pub side: String, // [buy, sell]
    pub size: f64,

    #[serde(rename = "orderType")]
    pub order_type: String, // [lmt, post, ioc, mkt, stp, take_profit, trailing_stop, fok]

    #[serde(rename = "limitPrice", skip_serializing_if = "Option::is_none")]
    pub limit_price: Option<f64>,

    #[serde(rename = "stopPrice", skip_serializing_if = "Option::is_none")]
    pub stop_price: Option<f64>,

    #[serde(rename = "triggerSignal", skip_serializing_if = "Option::is_none")]
    pub trigger_signal: Option<String>,

    #[serde(rename = "reduceOnly", skip_serializing_if = "Option::is_none")]
    pub reduce_only: Option<bool>,

    #[serde(rename = "cliOrdId", skip_serializing_if = "Option::is_none")]
    pub cli_ord_id: Option<String>,
}

#[derive(Serialize, Debug, Clone)]
pub struct EditOrder {
    #[serde(rename = "orderId", skip_serializing_if = "Option::is_none")]
    pub order_id: Option<String>,

    #[serde(rename = "cliOrdId", skip_serializing_if = "Option::is_none")]
    pub cli_ord_id: Option<String>,

    #[serde(rename = "limitPrice", skip_serializing_if = "Option::is_none")]
    pub limit_price: Option<f64>,

    #[serde(rename = "stopPrice", skip_serializing_if = "Option::is_none")]
    pub stop_price: Option<f64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
}

#[derive(Serialize, Debug, Clone)]
pub struct CancelOrder {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order_id: Option<String>,

    #[serde(rename = "cliOrdId", skip_serializing_if = "Option::is_none")]
    pub cli_ord_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendOrderResponse {
    pub result: String,
    pub send_status: SendStatus,
    pub server_time: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SendStatus {
    pub status: String,
    pub order_id: Option<String>,

    #[serde(rename = "orderEvents")]
    pub order_events: Option<Vec<Value>>,

    #[serde(rename = "receivedTime")]
    pub received_time: String,

    #[serde(rename = "cliOrdId", default)]
    pub cli_ord_id: Option<String>,
}

#[derive(Serialize, Debug, Clone, Default)]
pub struct CancelAllOrders {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct LeveragePreference {
    pub symbol: String,

    #[serde(rename = "maxLeverage", skip_serializing_if = "Option::is_none")]
    pub max_leverage: Option<u8>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeverageResponse {
    pub result: String,
    pub server_time: String,

    #[serde(default)]
    pub errors: Vec<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TickersResponse {
    pub result: String,
    pub server_time: String,
    pub ticker: Ticker,

    #[serde(default)]
    pub errors: Vec<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ticker {
    pub symbol: String,
    pub last: f64,
    pub mark_price: f64,
    pub index_price: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenPositionsResponse {
    pub result: String,
    pub open_positions: Vec<OpenPosition>,
    pub server_time: String,

    #[serde(default)]
    pub errors: Vec<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenPosition {
    pub symbol: String,
    pub side: String, // [long, short]
    pub size: f64,    // position size
    pub price: f64,   // average entry price

    #[serde(default)]
    pub fill_time: Option<String>, // deprecated, but still returned

    #[serde(default)]
    pub unrealized_funding: Option<f64>,

    #[serde(default)]
    pub pnl_currency: Option<String>, // [USD, EUR, GBP, USDC, USDT, BTC, ETH]

    #[serde(default)]
    pub max_fixed_leverage: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KrakenMarketCandlesResponse {
    pub candles: Vec<KrakenFutureCandle>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KrakenFutureCandle {
    pub time: i64,
    pub open: String,
    pub high: String,
    pub low: String,
    pub close: String,
    pub volume: String,
}

impl TryFrom<KrakenFutureCandle> for Candle {
    type Error = String;

    fn try_from(raw: KrakenFutureCandle) -> Result<Self, Self::Error> {
        let parse = |val: &str, field: &str| {
            val.parse()
                .map_err(|e| format!("Failed to parse {}: {} (value was '{}')", field, e, val))
        };

        Ok(Candle {
            time: raw.time,
            open: parse(&raw.open, "open")?,
            high: parse(&raw.high, "high")?,
            low: parse(&raw.low, "low")?,
            close: parse(&raw.close, "close")?,
            volume: parse(&raw.volume, "volume")?,
        })
    }
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OrderBookResponse {
    pub result: String,
    pub server_time: String,
    pub order_book: KrakenFuturesOrderBook,

    #[serde(default)]
    pub errors: Vec<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KrakenFuturesOrderBook {
    pub bids: Vec<(f64, f64)>,
    pub asks: Vec<(f64, f64)>,
}

impl From<KrakenFuturesOrderBook> for OrderBook {
    fn from(kraken: KrakenFuturesOrderBook) -> Self {
        Self {
            bids: kraken.bids,
            asks: kraken.asks,
        }
    }
}
