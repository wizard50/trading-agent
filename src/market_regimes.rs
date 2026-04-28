use crate::ta::*;
use std::error::Error;
use talib_rs::{
    momentum::rsi,
    overlap::{bbands, ema},
    statistic::linearreg_slope,
    volatility::atr,
};

pub fn get_regimes(
    df_15m: &DataFrame,
    df_1h: &DataFrame,
    order_book: &OrderBook,
) -> Result<MarketRegimes, Box<dyn Error>> {
    let close_15m = df_15m.close_col();
    let close_1h = df_1h.close_col();

    // Price above EMA 200
    let ema_200: Vec<f64> = ema(close_1h, 200)?;
    let current_close = *close_1h.last().unwrap_or(&0.0);
    let current_ema200 = *ema_200.last().unwrap_or(&0.0);
    let price_above_ema200 = current_close > current_ema200;

    // RSI
    let rsi_values = rsi(close_15m, 14)?;
    let current_rsi = *rsi_values.last().ok_or("No last RSI value")?;
    let rsi_oversold = current_rsi < 30.0;
    let rsi_overbought = current_rsi > 70.0;

    // Regimes
    let trend = trend(close_1h, 20);
    let volatility = volatility(&df_15m);
    let squeeze = squeeze(close_15m);
    let liquidity = liquidity(order_book, 0.008, 40.0, 4.0);

    Ok(MarketRegimes {
        trend,
        volatility,
        liquidity,
        squeeze,
        price_above_ema200,
        rsi_oversold,
        rsi_overbought,
    })
}

fn trend(close: &[f64], period: usize) -> TrendStrength {
    let slope_values: Vec<f64> = match linearreg_slope(close, period) {
        Ok(values) => values,
        Err(_) => return TrendStrength::Ranging,
    };

    let current_slope = *slope_values.last().unwrap_or(&0.0);
    let current_close = *close.last().unwrap_or(&0.0);

    // normalize slope
    let slope_pct_per_bar = if current_close != 0.0 {
        (current_slope / current_close) * 100.0
    } else {
        0.0
    };

    // Decide trend based on normalized slope
    let trend = match slope_pct_per_bar {
        s if s > 0.6 => TrendStrength::StrongUp,    // strong bullish
        s if s > 0.1 => TrendStrength::WeakUp,      // weak bullish
        s if s < -0.6 => TrendStrength::StrongDown, // strong bearish
        s if s < -0.1 => TrendStrength::WeakDown,   // weak bearish
        _ => TrendStrength::Ranging,
    };

    trend
}

fn volatility(df_15: &DataFrame) -> Volatility {
    if df_15.is_empty() {
        return Volatility::Normal;
    }

    let atr_values: Vec<f64> =
        atr(df_15.high_col(), df_15.low_col(), df_15.close_col(), 14).unwrap_or_default();

    let current_atr = *atr_values.last().unwrap_or(&0.0);
    let atr_percentile = percentile(&atr_values, current_atr);

    match atr_percentile {
        p if p > 80.0 => Volatility::High,
        p if p < 30.0 => Volatility::Low,
        _ => Volatility::Normal,
    }
}

fn liquidity(order_book: &OrderBook, depth_pct: f64, n_high: f64, n_low: f64) -> Liquidity {
    let mid_price = match order_book.mid_price() {
        Some(price) if price > 0.0 => price,
        _ => return Liquidity::Medium,
    };

    let total_depth_usd = order_book.total_depth_within_pct(depth_pct);
    let total_depth_coins = total_depth_usd / mid_price;

    // classify based on coin quantity
    match total_depth_coins {
        d if d > n_high => Liquidity::High,
        d if d < n_low => Liquidity::Low,
        _ => Liquidity::Medium,
    }
}

fn squeeze(close: &[f64]) -> Squeeze {
    if close.is_empty() {
        return Squeeze::Normal;
    }

    let (upper, _middle, lower) =
        bbands(close, 20, 2.0, 2.0, talib_rs::MaType::Sma).unwrap_or_default();

    let mut bb_width_values: Vec<f64> = Vec::with_capacity(upper.len());

    for i in 0..upper.len() {
        let width = if upper[i] > lower[i] {
            (upper[i] - lower[i]) / close[i]
        } else {
            0.0
        };
        bb_width_values.push(width);
    }

    let current_width = *bb_width_values.last().unwrap_or(&0.0);
    let width_percentile = percentile(&bb_width_values, current_width);

    match width_percentile {
        p if p < 20.0 => Squeeze::StrongSqueeze,
        p if p > 80.0 => Squeeze::Expansion,
        _ => Squeeze::Normal,
    }
}

#[derive(Debug, Clone)]
pub struct MarketRegimes {
    pub trend: TrendStrength,
    pub volatility: Volatility,
    pub liquidity: Liquidity,
    pub squeeze: Squeeze,

    pub price_above_ema200: bool,
    pub rsi_oversold: bool,
    pub rsi_overbought: bool,
}

impl MarketRegimes {
    pub fn summary(&self) -> String {
        format!(
            "Current market regimes:\n\
             - Trend Strength: {:?}\n\
             - Volatility: {:?}\n\
             - Liquidity: {:?}\n\
             - Squeeze: {:?}\n\
             - Price > EMA200: {}\n\
             - RSI oversold: {}\n\
             - RSI overbought: {}",
            self.trend,
            self.volatility,
            self.liquidity,
            self.squeeze,
            self.price_above_ema200,
            self.rsi_oversold,
            self.rsi_overbought
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TrendStrength {
    StrongUp,
    WeakUp,
    Ranging,
    WeakDown,
    StrongDown,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Volatility {
    Low,
    Normal,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Liquidity {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Squeeze {
    StrongSqueeze,
    Normal,
    Expansion,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kraken_futures::KrakenFuturesRestClient;

    #[tokio::test]
    async fn test_calculate_regimes() {
        let symbol = "PF_XBTUSD"; // BTC/USD on Kraken
        let client: KrakenFuturesRestClient = KrakenFuturesRestClient::demo();

        let df_15m: DataFrame = client
            .get_candles(symbol, "15m", Some(200), None, None)
            .await
            .unwrap()
            .into();

        let df_1h: DataFrame = client
            .get_candles(symbol, "1h", Some(200), None, None)
            .await
            .unwrap()
            .into();

        let order_book: OrderBook = client.get_order_book(symbol).await.unwrap().into();

        println!("Fetching candles and calculating regimes for {}...", symbol);

        match get_regimes(&df_15m, &df_1h, &order_book) {
            Ok(regimes) => {
                println!("\n=== MARKET REGIMES ===");
                println!("Trend Strength     : {:?}", regimes.trend);
                println!("Volatility         : {:?}", regimes.volatility);
                println!("Liquidity          : {:?}", regimes.liquidity);
                println!("Squeeze            : {:?}", regimes.squeeze);
                println!("Price > EMA200     : {}", regimes.price_above_ema200);
                println!("RSI Oversold       : {}", regimes.rsi_oversold);
                println!("RSI Overbought     : {}", regimes.rsi_overbought);

                // Also print the nice LLM-ready summary
                println!("\n--- Summary for Regime Agent ---");
                println!("{}", regimes.summary());
            }
            Err(e) => {
                eprintln!("Failed to calculate regimes: {}", e);
            }
        }
    }
}
