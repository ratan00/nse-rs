use std::collections::HashMap;
use anyhow::{Context, Result, bail};
use reqwest::Client;
use reqwest::header::COOKIE;
use serde::Deserialize;
use chrono::{DateTime, Duration as ChronoDuration, Utc, TimeZone, FixedOffset, NaiveTime};
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
        .error_for_status()
        .context("NSE charting returned an error status")?
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
/// Intraday candles are filtered to market hours (09:15–15:30 IST); volume is per bar
/// (see [`fix_day_total_bar`] for the one correction applied).
pub async fn get_historical_candles(
    client: &Client,
    cookies: &HashMap<String, String>,
    symbol: &str,
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
    interval: &str,
) -> Result<Vec<ChartCandle>> {
    let token = get_script_token(client, cookies, symbol).await?;
    get_historical_candles_for_token(client, cookies, &token, start_time, end_time, interval).await
}

/// Same as [`get_historical_candles`] but with a token from [`get_script_token`],
/// skipping the symbol search request.
pub async fn get_historical_candles_for_token(
    client: &Client,
    cookies: &HashMap<String, String>,
    token: &(String, String, String),
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
    interval: &str,
) -> Result<Vec<ChartCandle>> {
    let (real_symbol, token, symbol_type) = token;

    let is_intraday = !matches!(interval, "D" | "W" | "M");
    let ist_tz = FixedOffset::east_opt(IST_OFFSET_SECS).expect("valid IST offset");

    // Correcting each day's final bar (see `fix_day_total_bar`) needs every bar
    // of that day, so fetch from the start of the first IST day and trim
    // afterwards.  Skipped for windows near NSE's 30-day intraday limit so
    // widening cannot exceed it.
    let fetch_start = if is_intraday && end_time - start_time <= ChronoDuration::days(29) {
        start_time
            .with_timezone(&ist_tz)
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .and_then(|midnight| ist_tz.from_local_datetime(&midnight).single())
            .map(|midnight| midnight.with_timezone(&Utc).min(start_time))
            .unwrap_or(start_time)
    } else {
        start_time
    };

    // NSE's charting API expects IST Unix timestamps for intraday, UTC for EOD.
    let (chart_type, time_interval, start_ts, end_ts) = if is_intraday {
        let int_val: i32 = interval.parse().unwrap_or(3);
        (
            "I".to_string(),
            int_val,
            fetch_start.timestamp() + IST_OFFSET_SECS as i64,
            end_time.timestamp()    + IST_OFFSET_SECS as i64,
        )
    } else {
        (interval.to_string(), 1, start_time.timestamp(), end_time.timestamp())
    };

    let cookie_val = format_cookie_header(cookies);
    let resp: ChartResponse = client
        .get(HISTORICAL_DATA_URL)
        .header(COOKIE, cookie_val)
        .query(&[
            ("chartType",    chart_type.as_str()),
            ("fromDate",     &start_ts.to_string()),
            ("symbol",       real_symbol),
            ("symbolType",   symbol_type),
            ("timeInterval", &time_interval.to_string()),
            ("toDate",       &end_ts.to_string()),
            ("token",        token),
        ])
        .send()
        .await
        .context("historical data request")?
        .error_for_status()
        .context("NSE charting returned an error status")?
        .json()
        .await
        .context("historical data decode")?;

    let mut candles = resp.data.unwrap_or_default();

    if is_intraday {
        // Before the market-hours filter: the day total includes pre-open volume.
        fix_day_total_bar(&mut candles);
        filter_market_hours(&mut candles);
        if fetch_start < start_time {
            let requested_from_ms = (start_time.timestamp() + IST_OFFSET_SECS as i64) * 1000;
            candles.retain(|c| c.time >= requested_from_ms);
        }
    }

    Ok(candles)
}

/// IST calendar date and time of an intraday candle.
/// `c.time` is IST-shifted epoch milliseconds: `(real_utc + IST_OFFSET_SECS) * 1000`.
fn candle_ist(c: &ChartCandle) -> Option<chrono::NaiveDateTime> {
    let ist_tz = FixedOffset::east_opt(IST_OFFSET_SECS).expect("valid IST offset");
    let true_utc_secs = (c.time / 1000) - IST_OFFSET_SECS as i64;
    Utc.timestamp_opt(true_utc_secs, 0)
        .single()
        .map(|dt| dt.with_timezone(&ist_tz).naive_local())
}

/// Keep only candles inside 09:15–15:30 IST.
fn filter_market_hours(candles: &mut Vec<ChartCandle>) {
    let market_open  = NaiveTime::from_hms_opt(9, 15, 0).expect("valid time");
    let market_close = NaiveTime::from_hms_opt(15, 30, 0).expect("valid time");
    candles.retain(|c| {
        candle_ist(c)
            .map(|dt| dt.time() >= market_open && dt.time() < market_close)
            .unwrap_or(false)
    });
}

/// NSE's intraday volume is per bar, except that the final bar of each day
/// carries (roughly) the whole day's volume — e.g. a 5-min SBIN session whose
/// regular bars peak at ~0.35M ends with a 7.27M bar.  When a day's last bar
/// looks like that, replace its volume with what is left after subtracting the
/// day's other bars (clamped at 0).  The result is an estimate: NSE's day total
/// does not line up exactly with the bars, so this bar's volume is approximate.
fn fix_day_total_bar(candles: &mut [ChartCandle]) {
    let mut start = 0;
    while start < candles.len() {
        let day = candle_ist(&candles[start]).map(|dt| dt.date());
        let mut end = start + 1;
        while end < candles.len() && candle_ist(&candles[end]).map(|dt| dt.date()) == day {
            end += 1;
        }
        if let Some((last, others)) = candles[start..end].split_last_mut()
            && others.len() >= 2
        {
            let sum: f64 = others.iter().map(|c| c.volume).sum();
            let max = others.iter().map(|c| c.volume).fold(0.0, f64::max);
            if last.volume >= 0.5 * sum && last.volume > 3.0 * max {
                last.volume = (last.volume - sum).max(0.0);
            }
        }
        start = end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// IST-shifted epoch ms for an IST wall-clock time, matching NSE's encoding.
    fn ist_ms(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        let naive = chrono::NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_opt(h, mi, 0).unwrap();
        naive.and_utc().timestamp() * 1000
    }

    fn candle(time: i64, volume: f64) -> ChartCandle {
        ChartCandle { time, open: 1.0, high: 1.0, low: 1.0, close: 1.0, volume }
    }

    #[test]
    fn corrects_day_total_in_final_bar() {
        let mut c = vec![
            candle(ist_ms(2026, 1, 5, 9, 15), 100.0),
            candle(ist_ms(2026, 1, 5, 9, 20), 150.0),
            candle(ist_ms(2026, 1, 5, 9, 25), 120.0),
            candle(ist_ms(2026, 1, 5, 15, 25), 1000.0), // day total: 370 + 630
            candle(ist_ms(2026, 1, 6, 9, 15), 80.0),
            candle(ist_ms(2026, 1, 6, 9, 20), 90.0),
            candle(ist_ms(2026, 1, 6, 9, 25), 70.0),   // ordinary last bar: untouched
        ];
        fix_day_total_bar(&mut c);
        let v: Vec<f64> = c.iter().map(|c| c.volume).collect();
        assert_eq!(v, vec![100.0, 150.0, 120.0, 630.0, 80.0, 90.0, 70.0]);
    }

    #[test]
    fn day_total_below_other_bars_clamps_to_zero() {
        let mut c = vec![
            candle(ist_ms(2026, 1, 5, 9, 15), 100.0),
            candle(ist_ms(2026, 1, 5, 9, 20), 100.0),
            candle(ist_ms(2026, 1, 5, 9, 25), 100.0),
            candle(ist_ms(2026, 1, 5, 9, 30), 100.0),
            candle(ist_ms(2026, 1, 5, 9, 35), 100.0),
            candle(ist_ms(2026, 1, 5, 15, 29), 400.0), // total 400 < 500 of other bars
        ];
        fix_day_total_bar(&mut c);
        assert_eq!(c[5].volume, 0.0);
    }

    #[test]
    fn drops_bars_outside_market_hours() {
        let mut c = vec![
            candle(ist_ms(2026, 1, 5, 9, 10), 1.0),
            candle(ist_ms(2026, 1, 5, 9, 15), 1.0),
            candle(ist_ms(2026, 1, 5, 15, 25), 1.0),
            candle(ist_ms(2026, 1, 5, 15, 30), 1.0),
        ];
        filter_market_hours(&mut c);
        assert_eq!(c.len(), 2);
    }
}
