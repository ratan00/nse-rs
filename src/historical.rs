use std::collections::HashMap;
use reqwest::Client;
use reqwest::header::COOKIE;
use serde::Deserialize;
use chrono::{DateTime, Utc, TimeZone, FixedOffset, NaiveTime};
use crate::models::{ChartResponse, ChartCandle};
use crate::session::format_cookie_header;

const SEARCH_TOKEN_URL: &str = "https://charting.nseindia.com/v1/exchanges/symbolsDynamic";
const HISTORICAL_DATA_URL: &str = "https://charting.nseindia.com/v1/charts/symbolHistoricalData";

#[derive(Debug, Clone, Deserialize)]
pub struct SymbolSearchResponse {
    pub data: Vec<SymbolSearchItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SymbolSearchItem {
    pub symbol: String,
    pub scripcode: String,
    #[serde(rename = "type")]
    pub instrument_type: String,
    pub description: String,
}

/// Fetch script token, symbol name, and segment type from symbolsDynamic endpoint
pub async fn get_script_token(
    client: &Client,
    cookies: &HashMap<String, String>,
    symbol: &str,
) -> Result<(String, String, String), Box<dyn std::error::Error + Send + Sync>> {
    let cookie_val = format_cookie_header(cookies);
    
    let resp = client.get(SEARCH_TOKEN_URL)
        .header(COOKIE, cookie_val)
        .query(&[("segment", ""), ("symbol", symbol)])
        .send()
        .await?
        .json::<SymbolSearchResponse>()
        .await?;

    let search_symbol = symbol.to_uppercase();
    
    // 1. Exact match (split by '-')
    if let Some(item) = resp.data.iter().find(|i| {
        let sym_base = i.symbol.split('-').next().unwrap_or("").to_uppercase();
        sym_base == search_symbol
    }) {
        return Ok((item.symbol.clone(), item.scripcode.clone(), item.instrument_type.clone()));
    }
    
    // 2. Starts with match
    if let Some(item) = resp.data.iter().find(|i| {
        let sym_base = i.symbol.split('-').next().unwrap_or("").to_uppercase();
        sym_base.starts_with(&search_symbol)
    }) {
        return Ok((item.symbol.clone(), item.scripcode.clone(), item.instrument_type.clone()));
    }
    
    // 3. Description contains match
    if let Some(item) = resp.data.iter().find(|i| {
        i.description.to_uppercase().contains(&search_symbol)
    }) {
        return Ok((item.symbol.clone(), item.scripcode.clone(), item.instrument_type.clone()));
    }
    
    // Fallback: pick first
    if let Some(item) = resp.data.first() {
        return Ok((item.symbol.clone(), item.scripcode.clone(), item.instrument_type.clone()));
    }
    
    Err("Symbol not found".into())
}

/// Fetches historical data for a symbol resolved via token dynamic search.
/// `interval` can be minutes (e.g. "1", "3", "5", "15", "30", "60") or "D", "W", "M"
pub async fn get_historical_candles(
    client: &Client,
    cookies: &HashMap<String, String>,
    symbol: &str,
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
    interval: &str,
) -> Result<Vec<ChartCandle>, Box<dyn std::error::Error + Send + Sync>> {
    let (real_symbol, token, symbol_type) = get_script_token(client, cookies, symbol).await?;
    
    let is_intraday = match interval {
        "D" | "W" | "M" => false,
        _ => true,
    };
    
    // Handle NSE IST time offset calculation
    let (chart_type, time_interval, start_ts, end_ts) = if is_intraday {
        let ist_offset = 19800; // 5 hours 30 mins
        let int_val = interval.parse::<i32>().unwrap_or(3);
        (
            "I".to_string(),
            int_val,
            start_time.timestamp() + ist_offset,
            end_time.timestamp() + ist_offset,
        )
    } else {
        (
            interval.to_string(),
            1,
            start_time.timestamp(),
            end_time.timestamp(),
        )
    };

    let cookie_val = format_cookie_header(cookies);
    let resp = client.get(HISTORICAL_DATA_URL)
        .header(COOKIE, cookie_val)
        .query(&[
            ("chartType", &chart_type),
            ("fromDate", &start_ts.to_string()),
            ("symbol", &real_symbol),
            ("symbolType", &symbol_type),
            ("timeInterval", &time_interval.to_string()),
            ("toDate", &end_ts.to_string()),
            ("token", &token),
        ])
        .send()
        .await?
        .json::<ChartResponse>()
        .await?;

    let mut candles = resp.data.unwrap_or_default();
    
    // Filter out pre-market and post-market price levels for intraday data
    if is_intraday {
        let ist_tz = FixedOffset::east_opt(5 * 3600 + 30 * 60).unwrap();
        let start_market = NaiveTime::from_hms_opt(9, 15, 0).unwrap();
        let end_market = NaiveTime::from_hms_opt(15, 30, 0).unwrap();
        
        candles.retain(|candle| {
            // Convert millisecond timestamp to IST
            if let Some(dt_utc) = Utc.timestamp_opt(candle.time / 1000, ((candle.time % 1000) * 1_000_000) as u32).single() {
                let dt_ist = dt_utc.with_timezone(&ist_tz);
                let time = dt_ist.time();
                time >= start_market && time < end_market
            } else {
                false
            }
        });
    }

    Ok(candles)
}
