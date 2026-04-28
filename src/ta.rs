use serde::Deserialize;

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Candle {
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

type Series<T> = Vec<T>;

#[derive(Debug, Clone)]
pub struct DataFrame {
    pub timestamp: Series<i64>,
    pub open: Series<f64>,
    pub high: Series<f64>,
    pub low: Series<f64>,
    pub close: Series<f64>,
    pub volume: Series<f64>,
}

impl From<Vec<Candle>> for DataFrame {
    fn from(candles: Vec<Candle>) -> Self {
        let len = candles.len();

        let mut timestamp = Vec::with_capacity(len);
        let mut open = Vec::with_capacity(len);
        let mut high = Vec::with_capacity(len);
        let mut low = Vec::with_capacity(len);
        let mut close = Vec::with_capacity(len);
        let mut volume = Vec::with_capacity(len);

        for c in candles {
            timestamp.push(c.time);
            open.push(c.open);
            high.push(c.high);
            low.push(c.low);
            close.push(c.close);
            volume.push(c.volume);
        }

        DataFrame {
            timestamp,
            open,
            high,
            low,
            close,
            volume,
        }
    }
}

impl DataFrame {
    pub fn timestamp_col(&self) -> &[i64] {
        &self.timestamp
    }

    pub fn open_col(&self) -> &[f64] {
        &self.open
    }

    pub fn high_col(&self) -> &[f64] {
        &self.high
    }

    pub fn low_col(&self) -> &[f64] {
        &self.low
    }

    pub fn close_col(&self) -> &[f64] {
        &self.close
    }

    pub fn len(&self) -> usize {
        self.close.len()
    }

    pub fn is_empty(&self) -> bool {
        self.close.is_empty()
    }

    pub fn last_n(&self, n: usize) -> DataFrame {
        let n = n.min(self.len());
        let start = self.len() - n;

        DataFrame {
            timestamp: self.timestamp[start..].to_vec(),
            open: self.open[start..].to_vec(),
            high: self.high[start..].to_vec(),
            low: self.low[start..].to_vec(),
            close: self.close[start..].to_vec(),
            volume: self.volume[start..].to_vec(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct OrderBook {
    pub bids: Vec<(f64, f64)>, // (price, volume)
    pub asks: Vec<(f64, f64)>,
}

impl OrderBook {
    pub fn mid_price(&self) -> Option<f64> {
        let best_bid = self.bids.first()?.0;
        let best_ask = self.asks.first()?.0;
        Some((best_bid + best_ask) / 2.0)
    }

    pub fn total_depth_within_pct(&self, max_dist_pct: f64) -> f64 {
        let bid_depth = self.bid_depth_within_pct(max_dist_pct);
        let ask_depth = self.ask_depth_within_pct(max_dist_pct);
        bid_depth + ask_depth
    }

    pub fn bid_depth_within_pct(&self, max_dist_pct: f64) -> f64 {
        self.calc_depth(&self.bids, max_dist_pct)
    }

    pub fn ask_depth_within_pct(&self, max_dist_pct: f64) -> f64 {
        self.calc_depth(&self.asks, max_dist_pct)
    }

    fn calc_depth(&self, items: &Vec<(f64, f64)>, max_dist_pct: f64) -> f64 {
        let mid = self.mid_price().unwrap_or(0.0);
        if mid == 0.0 {
            return 0.0;
        }
        let threshold = mid * (1.0 - max_dist_pct);
        items
            .iter()
            .take_while(|(p, _)| *p >= threshold)
            .map(|(p, q)| p * q)
            .sum()
    }
}

pub fn percentile(values: &[f64], value: f64) -> f64 {
    if values.is_empty() {
        return 50.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let pos = sorted.partition_point(|&x| x < value) as f64;
    (pos / sorted.len() as f64) * 100.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kraken_futures::KrakenFuturesRestClient;
    use std::error::Error;
    use talib_rs::overlap::sma;

    #[tokio::test]
    async fn test_parse_ohlcv() -> Result<(), Box<dyn Error>> {
        let client = KrakenFuturesRestClient::demo();

        let candles = client
            .get_candles("PF_XBTUSD", "15m", Some(100), None, None)
            .await?;

        let df: DataFrame = DataFrame::from(candles);
        let close = df.close_col();

        let sma_ds = sma(close, 20).unwrap();
        println!("{}, {}", sma_ds.len(), close.len());
        println!("sma: {:?}", sma_ds[20..40].to_vec());

        Ok(())
    }
}
