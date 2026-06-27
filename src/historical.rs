use std::collections::HashMap;
use anyhow::{Context, Result, bail};
use reqwest::Client;
use reqwest::header::COOKIE;
use serde::Deserialize;
use chrono::{DateTime, Utc, TimeZone, FixedOffset, NaiveTime};
use crate::models::{ChartResponse, ChartCandle};
use crate::session::format_cookie_header;

const SEARCH_TOKEN_URL: &str   = "https://charting.nseindia.com/v1/exchanges/symbolsDynamic";
const HISTORICAL_DATA_URL: &str = "https://charting.nseindia.com/v1/charts/symbolHistoricalData";

/// IST = UTC+5:30 = 19800 seconds east
const IST_OFFSET_SECS: i32 = 5 * 3600 + 30 * 60;

#[derive(Debug, Clone, Deserialize)]
pub struct SymbolSearchResponse {
    pub data: Vec<SymbolSearchItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SymbolSearchItem {
    pub symbol:    String,
    pub scripcode: String,
    #[serde(rename = "type")]
    pub instrument_type: String,
    pub description: String,
}

/// Look up the charting token for `symbol`.
/// Returns `(charting_symbol, scripcode, instrument_type)`.
pub async fn get_script_token(
    client: &Client,
    cookies: &HashMap<String, String>,
    symbol: &str,
) -> Result<(String, String, String)> {
    let cookie_val = format_cookie_header(cookies);
    let sym_upper = symbol.to_uppercase();
    let resp: SymbolSearchResponse = client
        .get(SEARCH_TOKEN_URL)
        .header(COOKIE, cookie_val)
        .query(&[("segment", ""), ("symbol", &sym_upper)])
        .send()
        .await
        .context("symbol search request")?
        .json()
        .await
        .context("symbol search decode")?;

    let search = symbol.to_uppercase();
    let items = &resp.data;

    // 1. Exact match on the base part before '-'
    if let Some(item) = items.iter().find(|i| {
        i.symbol.split('-').next().unwrap_or("").to_uppercase() == search
    }) {
        return Ok((item.symbol.clone(), item.scripcode.clone(), item.instrument_type.clone()));
    }
    // 2. Starts-with
    if let Some(item) = items.iter().find(|i| {
        i.symbol.split('-').next().unwrap_or("").to_uppercase().starts_with(&search)
    }) {
        return Ok((item.symbol.clone(), item.scripcode.clone(), item.instrument_type.clone()));
    }
    // 3. Description contains
    if let Some(item) = items.iter().find(|i| {
        i.description.to_uppercase().contains(&search)
    }) {
        return Ok((item.symbol.clone(), item.scripcode.clone(), item.instrument_type.clone()));
    }
    // 4. First result as fallback
    if let Some(item) = items.first() {
        return Ok((item.symbol.clone(), item.scripcode.clone(), item.instrument_type.clone()));
    }

    bail!("symbol '{symbol}' not found in NSE charting search")
}

/// Fetch historical candles.
/// `interval` is `"1"`, `"3"`, `"5"`, `"15"`, `"30"`, `"60"` (minutes) or `"D"`, `"W"`, `"M"`.
/// Intraday candles are filtered to market hours (09:15–15:30 IST).
pub async fn get_historical_candles(
    client: &Client,
    cookies: &HashMap<String, String>,
    symbol: &str,
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
    interval: &str,
) -> Result<Vec<ChartCandle>> {
    let (real_symbol, token, symbol_type) =
        get_script_token(client, cookies, symbol).await?;

    let is_intraday = !matches!(interval, "D" | "W" | "M");

    // NSE's charting API expects IST Unix timestamps for intraday, UTC for EOD.
    let (chart_type, time_interval, start_ts, end_ts) = if is_intraday {
        let int_val: i32 = interval.parse().unwrap_or(3);
        (
            "I".to_string(),
            int_val,
            start_time.timestamp() + IST_OFFSET_SECS as i64,
            end_time.timestamp()   + IST_OFFSET_SECS as i64,
        )
    } else {
        (interval.to_string(), 1, start_time.timestamp(), end_time.timestamp())
    };

    let cookie_val = format_cookie_header(cookies);
    let resp: ChartResponse = client
        .get(HISTORICAL_DATA_URL)
        .header(COOKIE, cookie_val)
        .query(&[
            ("chartType",    &chart_type),
            ("fromDate",     &start_ts.to_string()),
            ("symbol",       &real_symbol),
            ("symbolType",   &symbol_type),
            ("timeInterval", &time_interval.to_string()),
            ("toDate",       &end_ts.to_string()),
            ("token",        &token),
        ])
        .send()
        .await
        .context("historical data request")?
        .json()
        .await
        .context("historical data decode")?;

    let mut candles = resp.data.unwrap_or_default();

    if is_intraday {
        let market_open  = NaiveTime::from_hms_opt(9, 15, 0).expect("valid time");
        let market_close = NaiveTime::from_hms_opt(15, 30, 0).expect("valid time");

        candles.retain(|c| {
            Utc.timestamp_opt(c.time / 1000, 0)
                .single()
                .map(|dt| {
                    let t = dt.time();
                    t >= market_open && t < market_close
                })
                .unwrap_or(false)
        });

        // Convert cumulative day volume to discrete per-bar volume
        if !candles.is_empty() {
            let mut prev_vol = candles[0].volume;
            let ist_tz = FixedOffset::east_opt(IST_OFFSET_SECS).expect("valid IST offset");
            for i in 1..candles.len() {
                let cur_vol = candles[i].volume;

                let prev_dt = Utc.timestamp_opt(candles[i - 1].time / 1000, 0)
                    .single()
                    .map(|dt| dt.with_timezone(&ist_tz).date_naive());
                let cur_dt = Utc.timestamp_opt(candles[i].time / 1000, 0)
                    .single()
                    .map(|dt| dt.with_timezone(&ist_tz).date_naive());

                let same_day = prev_dt.zip(cur_dt).map(|(a, b)| a == b).unwrap_or(false);

                if same_day {
                    let diff = cur_vol - prev_vol;
                    candles[i].volume = if diff >= 0.0 { diff } else { cur_vol };
                } else {
                    candles[i].volume = cur_vol;
                }
                prev_vol = cur_vol;
            }
        }
    }

    Ok(candles)
}
