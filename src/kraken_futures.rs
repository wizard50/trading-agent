use crate::ta::Candle;
use crate::{kraken_types::*, utils::with_exponential_backoff};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use hmac::{Hmac, KeyInit, Mac};
use reqwest::{Method, RequestBuilder, Response};
use secrecy::{ExposeSecret, SecretString};
use serde::de::DeserializeOwned;
use serde_json::Value;
use sha2::{Digest, Sha256, Sha512};
use std::{error::Error, time::SystemTime};

type HmacSha512 = Hmac<Sha512>;

pub const LIVE_BASE_URL: &str = "https://futures.kraken.com";
pub const DEMO_BASE_URL: &str = "https://demo-futures.kraken.com";

#[derive(Debug, Clone)]
pub struct KrakenFuturesRestClient {
    base_url: String,
    client: reqwest::Client,
    api_key: Option<SecretString>,
    api_secret: Option<SecretString>,
}

impl KrakenFuturesRestClient {
    pub fn new() -> Self {
        Self::with_url(LIVE_BASE_URL.into(), None, None)
    }

    pub fn demo() -> Self {
        Self::with_url(DEMO_BASE_URL.into(), None, None)
    }

    pub fn with_auth(
        env: KrakenEnvironment,
        api_key: impl Into<SecretString>,
        api_secret: impl Into<SecretString>,
    ) -> Self {
        let base_url = match env {
            KrakenEnvironment::Live => LIVE_BASE_URL,
            KrakenEnvironment::Demo => DEMO_BASE_URL,
        };

        Self::with_url(
            base_url.to_string(),
            Some(api_key.into()),
            Some(api_secret.into()),
        )
    }

    fn with_url(
        base_url: String,
        api_key: Option<SecretString>,
        api_secret: Option<SecretString>,
    ) -> Self {
        KrakenFuturesRestClient {
            base_url,
            client: reqwest::Client::new(),
            api_key,
            api_secret,
        }
    }

    fn nonce() -> Result<u64, Box<dyn Error>> {
        Ok(SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_millis() as u64)
    }

    fn sign(
        &self,
        post_data: &str,
        nonce: &str,
        endpoint_path: &str,
    ) -> Result<String, Box<dyn Error>> {
        let secret = self.api_secret.as_ref().ok_or("No api secret found.")?;
        let secret_bytes = BASE64
            .decode(secret.expose_secret())
            .map_err(|e| format!("Invalid base64 API secret: {e}"))?;

        let message = format!("{}{}{}", post_data, nonce, endpoint_path);
        let sha256_digest = Sha256::new() //
            .chain_update(message.as_bytes())
            .finalize();

        let result = HmacSha512::new_from_slice(&secret_bytes)
            .map_err(|e| format!("HMAC key error: {e}"))?
            .chain_update(&sha256_digest) // ← this works after creation
            .finalize()
            .into_bytes();

        Ok(BASE64.encode(result))
    }

    async fn private_request<T: serde::Serialize, R: DeserializeOwned>(
        &self,
        method: Method,
        endpoint: &str,
        payload: Option<&T>,
    ) -> Result<R, Box<dyn Error>> {
        let api_key = self
            .api_key
            .as_ref()
            .ok_or("Client must be authenticated – use KrakenFuturesRestClient::authenticated()")?
            .expose_secret();

        let post_data = match payload {
            Some(p) if method == Method::POST || method == Method::PUT => {
                serde_urlencoded::to_string(p)
                    .map_err(|e| format!("Failed to urlencode payload for {}: {}", endpoint, e))?
            }
            _ => String::new(),
        };

        let nonce_str = Self::nonce()?.to_string();
        let auth_path = format!("/api/v3/{}", endpoint);
        let authent = self.sign(&post_data, &nonce_str, &auth_path)?;
        let url = format!("{}/derivatives/api/v3/{}", self.base_url, endpoint);

        let mut builder = self
            .client
            .request(method, &url)
            .header("APIKey", api_key)
            .header("Nonce", &nonce_str)
            .header("Authent", authent);

        if !post_data.is_empty() {
            builder = builder
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(post_data);
        }

        let resp = builder.send().await?;

        Self::handle_response::<R>(resp).await
    }

    async fn private_get<R: DeserializeOwned>(&self, endpoint: &str) -> Result<R, Box<dyn Error>> {
        with_exponential_backoff(3, || async {
            self.private_request::<(), R>(Method::GET, endpoint, None)
                .await
        })
        .await
    }

    async fn private_post<T: serde::Serialize, R: DeserializeOwned>(
        &self,
        endpoint: &str,
        payload: &T,
    ) -> Result<R, Box<dyn Error>> {
        with_exponential_backoff(3, || async {
            self.private_request(Method::POST, endpoint, Some(payload))
                .await
        })
        .await
    }

    async fn private_put<T: serde::Serialize, R: DeserializeOwned>(
        &self,
        endpoint: &str,
        payload: &T,
    ) -> Result<R, Box<dyn Error>> {
        with_exponential_backoff(3, || async {
            self.private_request(Method::PUT, endpoint, Some(payload))
                .await
        })
        .await
    }

    async fn public_get<R: DeserializeOwned>(
        &self,
        url: String,
        query_params: Vec<(&'static str, String)>,
    ) -> Result<R, Box<dyn Error>> {
        with_exponential_backoff(3, || async {
            let mut builder = self.client.get(&url);

            if !query_params.is_empty() {
                builder = builder.query(&query_params);
            }

            let http_resp = builder
                .send()
                .await
                .map_err(|e| Box::new(e) as Box<dyn Error>)?;

            Self::handle_response::<R>(http_resp).await
        })
        .await
    }

    async fn handle_response<R: DeserializeOwned>(resp: Response) -> Result<R, Box<dyn Error>> {
        if !resp.status().is_success() {
            let status = resp.status();
            let error_text = resp.text().await.unwrap_or_default();
            return Err(format!("HTTP {status}: {error_text}").into());
        }

        let body_text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read response body: {}", e))?;

        serde_json::from_str::<R>(&body_text)
            .map_err(|e| format!("JSON parse error: {}\n\nRaw body was:\n{}", e, &body_text).into())
    }

    //---------------------------
    // Private Endpoints
    // --------------------------
    pub async fn send_order(&self, order: &SendOrder) -> Result<SendOrderResponse, Box<dyn Error>> {
        self.private_post("sendorder", &order).await
    }

    pub async fn cancel_order(&self, order_id: &str) -> Result<Value, Box<dyn Error>> {
        let payload = CancelOrder {
            order_id: Some(order_id.to_string()),
            cli_ord_id: None,
        };
        self.private_post("cancelorder", &payload).await
    }

    pub async fn cancel_all_orders(&self, symbol: Option<&str>) -> Result<Value, Box<dyn Error>> {
        let payload = CancelAllOrders {
            symbol: symbol.map(|s| s.to_string()),
        };
        self.private_post("cancelallorders", &payload).await
    }

    pub async fn set_leverage(
        &self,
        symbol: &str,
        leverage: Option<u8>, // leverage None => cross
    ) -> Result<LeverageResponse, Box<dyn Error>> {
        let payload = LeveragePreference {
            symbol: symbol.to_string(),
            max_leverage: leverage,
        };
        self.private_put("leveragepreferences", &payload).await
    }

    pub async fn get_open_positions(&self) -> Result<OpenPositionsResponse, Box<dyn Error>> {
        self.private_get("openpositions").await
    }

    //---------------------------
    // Public Endpoints
    // --------------------------
    pub async fn get_current_price(&self, symbol: &str) -> Result<Ticker, Box<dyn Error>> {
        let url = format!("{}/derivatives/api/v3/ticker/{}", self.base_url, symbol);

        let resp: TickersResponse = self
            .public_get(url, vec![]) // no query params
            .await?;

        if resp.result != "success" {
            return Err(format!("Kraken ticker error for {}: {:?}", symbol, resp.error).into());
        }

        Ok(resp.ticker)
    }

    pub async fn get_candles(
        &self,
        symbol: &str,       // e.g. "PF_XBTUSD"
        resolution: &str,   // "1m", "5m", "15m", "60m", "4h", "1d" ...
        count: Option<u32>, // max number of candles
        from: Option<i64>,  // unix timestamp in seconds
        to: Option<i64>,    // unix timestamp in seconds
    ) -> Result<Vec<Candle>, Box<dyn Error>> {
        let url = format!(
            "{}/api/charts/v1/trade/{}/{}",
            self.base_url, symbol, resolution
        );

        let mut query = vec![];
        if let Some(c) = count {
            query.push(("count", c.to_string()));
        }
        if let Some(f) = from {
            query.push(("from", f.to_string()));
        }
        if let Some(t) = to {
            query.push(("to", t.to_string()));
        }

        let resp: KrakenMarketCandlesResponse = self.public_get(url, query).await?;
        let candles: Vec<Candle> = resp
            .candles
            .into_iter()
            .map(TryFrom::try_from)
            .collect::<Result<Vec<Candle>, _>>()?;

        Ok(candles)
    }

    pub async fn get_order_book(
        &self,
        symbol: &str,
    ) -> Result<KrakenFuturesOrderBook, Box<dyn Error>> {
        let url = format!("{}/derivatives/api/v3/orderbook", self.base_url);

        let resp: OrderBookResponse = self
            .public_get(url, vec![("symbol", symbol.to_string())])
            .await?;

        if resp.result != "success" {
            return Err(format!("Kraken orderbook error for {}: {:?}", symbol, resp.error).into());
        }

        Ok(resp.order_book)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[tokio::test]
    async fn test_get_futures_candles() -> Result<(), Box<dyn Error>> {
        let client = KrakenFuturesRestClient::demo();

        let candles = client
            .get_candles("PF_XBTUSD", "15m", Some(100), None, None)
            .await?;

        println!("✅ Loaded {} candles", candles.len());
        if let Some(c) = candles.first() {
            println!("First candle: time={} close={}", c.time, c.close);
        }

        assert!(!candles.is_empty());
        Ok(())
    }

    #[tokio::test]
    #[ignore = "Requires kraken CLI installed + demo account. Run manually only."]
    async fn test_place_single_buy_order_on_demo() {
        let config = Config::load().expect("No config found");

        let client = KrakenFuturesRestClient::with_auth(
            KrakenEnvironment::Demo,
            config.kraken_futures_api_key,
            config.kraken_futures_api_secret,
        );

        let order = SendOrder {
            order_type: "mkt".to_string(), // ← market order
            symbol: "PF_XBTUSD".to_string(),
            side: "buy".to_string(),
            size: 0.001,
            limit_price: None,
            stop_price: None,
            trigger_signal: None,
            reduce_only: Some(false),
            cli_ord_id: None,
        };

        let result = client.send_order(&order).await;

        match result {
            Ok(value) => {
                println!("✅ Single market BUY order placed successfully!");
                println!("Response: {:#?}", value);
            }
            Err(e) => panic!("Single order failed: {}", e),
        }
    }

    #[tokio::test]
    #[ignore = "Requires kraken CLI installed + demo account. Run manually only."]
    async fn test_get_open_positions_on_demo() {
        let config = Config::load().expect("No config found");

        let client = KrakenFuturesRestClient::with_auth(
            KrakenEnvironment::Demo,
            config.kraken_futures_api_key,
            config.kraken_futures_api_secret,
        );

        let response = client
            .get_open_positions()
            .await
            .expect("Failed to get open positions from Kraken");

        // Print the full result once (clean and complete)
        println!("✅ get_open_positions succeeded on demo:");
        println!("{:#?}", response);

        // Basic assertions
        assert_eq!(response.result, "success", "Kraken should return success");
        assert!(
            response.open_positions.len() <= 100,
            "Sanity check: too many positions"
        );
    }

    #[tokio::test]
    #[ignore = "Requires kraken CLI installed + demo account. Run manually only."]
    async fn test_set_leverage_on_demo() {
        let config = Config::load().expect("No config found");

        let client = KrakenFuturesRestClient::with_auth(
            KrakenEnvironment::Demo,
            config.kraken_futures_api_key,
            config.kraken_futures_api_secret,
        );

        let symbol = "PF_XBTUSD";
        let leverage = Some(1u8);

        let response = client
            .set_leverage(symbol, leverage)
            .await
            .expect("Failed to set leverage on Kraken Futures");

        println!("{:#?}", &response);

        // Basic assertions
        assert_eq!(response.result, "success", "Kraken should return success");
    }
}
