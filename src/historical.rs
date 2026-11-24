use std::collections::HashMap;
use anyhow::{Context, Result, bail};
use reqwest::Client;
use reqwest::header::COOKIE;
use serde::Deserialize;
use chrono::{DateTime, Duration as ChronoDuration, Utc, TimeZone, FixedOffset, NaiveTime};
use crate::models::{ChartResponse, ChartCandle};
use crate::session::format_cookie_header;
use crate::hosts::charting;

const SEARCH_TOKEN_URL: &str   = charting!("/v1/exchanges/symbolsDynamic");
const HISTORICAL_DATA_URL: &str = charting!("/v1/charts/symbolHistoricalData");

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

/// How far back NSE serves intraday bars: whole sessions from about this many
/// calendar days ago. A window reaching further back gets only what is left.
pub const INTRADAY_HISTORY_DAYS: i64 = 30;

/// A parsed `interval` argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interval {
    /// Intraday bars of this many minutes (1 to 375, one session).
    Minutes(i64),
    /// `"D"`, `"W"` or `"M"`.
    Eod(&'static str),
}

impl Interval {
    /// `"1"`, `"5"`, `"60"` … (minutes) or `"D"`, `"W"`, `"M"`. Anything else is
    /// an error rather than a guess.
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "D" => return Ok(Interval::Eod("D")),
            "W" => return Ok(Interval::Eod("W")),
            "M" => return Ok(Interval::Eod("M")),
            _ => {}
        }
        match s.parse::<i64>() {
            Ok(m) if (1..=375).contains(&m) => Ok(Interval::Minutes(m)),
            _ => bail!("unsupported interval '{s}': minutes (\"1\" to \"375\") or \"D\", \"W\", \"M\""),
        }
    }
}

/// Fetch historical candles.
/// `interval` is minutes (`"1"`, `"3"`, `"5"`, `"15"`, `"30"`, `"60"`, … up to
/// `"375"`) or `"D"`, `"W"`, `"M"`; anything else is an error.
/// Intraday candles are filtered to market hours (09:15–15:30 IST); volume is per bar
/// (see [`fix_day_total_bar`] for the one correction applied). Intraday history
/// goes back [`INTRADAY_HISTORY_DAYS`]: a window that ends before that is an
/// error, one that starts before it gets the sessions NSE still has.
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

    let parsed = Interval::parse(interval)?;
    let is_intraday = matches!(parsed, Interval::Minutes(_));
    let ist_tz = FixedOffset::east_opt(IST_OFFSET_SECS).expect("valid IST offset");

    if is_intraday && end_time < Utc::now() - ChronoDuration::days(INTRADAY_HISTORY_DAYS) {
        bail!("NSE keeps about {INTRADAY_HISTORY_DAYS} days of intraday history; {start_time}..{end_time} is older");
    }

    // Correcting each day's day-total bar (see `fix_day_total_bar`) needs
    // every bar of that day, so fetch from the start of the first IST day and
    // trim afterwards. NSE cuts its intraday history by whole sessions, so
    // widening never asks for a day it would only half serve.
    let fetch_start = if is_intraday {
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

    // NSE's own intraday bars above one minute can't be trusted: older days
    // are stamped at each bar's last trade instead of its start, the day's
    // cumulative volume shows up in more than one bar, and a 60-minute bar can
    // hold several times the day's turnover. Its one-minute bars are clean
    // apart from a single day-total row near 15:29, so intraday is always
    // fetched at one minute and built up to the requested interval here.
    let minutes: i64 = match parsed {
        Interval::Minutes(m) => m,
        Interval::Eod(_) => 1,
    };

    // NSE's charting API expects IST Unix timestamps for intraday, UTC for EOD.
    let (chart_type, time_interval, start_ts, end_ts) = if is_intraday {
        (
            "I".to_string(),
            60, // seconds
            fetch_start.timestamp() + IST_OFFSET_SECS as i64,
            end_time.timestamp()    + IST_OFFSET_SECS as i64,
        )
    } else {
        (interval.to_string(), 1, start_time.timestamp(), end_time.timestamp())
    };

    let token = chart_token(token, Utc::now());
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
            ("token",        &token),
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
        let mut minute = to_minute_bars(candles);
        // Trades after the close are not in the day total.
        let close = NaiveTime::from_hms_opt(15, 30, 0).expect("valid time");
        minute.retain(|c| candle_ist(c).is_some_and(|dt| dt.time() < close));
        // Before the market-hours filter: the day total includes pre-open volume.
        fix_day_total_bar(&mut minute);
        filter_market_hours(&mut minute);
        candles = resample(&minute, minutes);
        if fetch_start < start_time {
            let requested_from_ms = (start_time.timestamp() + IST_OFFSET_SECS as i64) * 1000;
            candles.retain(|c| c.time >= requested_from_ms);
        }
    }

    Ok(candles)
}

/// The `token` the history endpoint takes: the scripcode with today's IST
/// date, `"<scripcode>-<DDMMYYYY>"`, base64-encoded, the way NSE's charting
/// page sends it. A bare scripcode is answered with no bars.
fn chart_token(scripcode: &str, now: DateTime<Utc>) -> String {
    let ist = FixedOffset::east_opt(IST_OFFSET_SECS).expect("valid IST offset");
    let raw = format!("{scripcode}-{}", now.with_timezone(&ist).format("%d%m%Y"));
    base64(raw.as_bytes())
}

/// Standard base64 with padding.
fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ABC[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// One bar per minute, stamped at the minute's start. Older days come stamped
/// at the bar's last trade (09:15:55 for the 09:15 minute): floor them, and
/// fold any rows that land on the same minute (an exact repeat is dropped).
fn to_minute_bars(mut rows: Vec<ChartCandle>) -> Vec<ChartCandle> {
    for c in &mut rows {
        c.time -= c.time.rem_euclid(60_000);
    }
    rows.sort_by_key(|c| c.time);
    let mut out: Vec<ChartCandle> = Vec::with_capacity(rows.len());
    for c in rows {
        match out.last_mut() {
            Some(p) if p.time == c.time => {
                if (p.open, p.high, p.low, p.close, p.volume) != (c.open, c.high, c.low, c.close, c.volume) {
                    merge_into(p, &c);
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// One-minute bars into `minutes`-minute bars on the session grid (09:15 IST
/// onward, so a 60-minute day reads 09:15, 10:15, … 15:15).
fn resample(bars: &[ChartCandle], minutes: i64) -> Vec<ChartCandle> {
    if minutes <= 1 {
        return bars.to_vec();
    }
    const OPEN_MS: i64 = (9 * 3600 + 15 * 60) * 1000;
    let step = minutes * 60_000;
    let mut out: Vec<ChartCandle> = Vec::new();
    for c in bars {
        // `time` is IST-shifted, so whole days are IST midnights.
        let anchor = c.time - c.time.rem_euclid(86_400_000) + OPEN_MS;
        let bucket = anchor + (c.time - anchor).div_euclid(step) * step;
        match out.last_mut() {
            Some(p) if p.time == bucket => merge_into(p, c),
            _ => out.push(ChartCandle { time: bucket, ..c.clone() }),
        }
    }
    out
}

/// `c` comes after `into` in time: extend `into` by it.
fn merge_into(into: &mut ChartCandle, c: &ChartCandle) {
    into.high = into.high.max(c.high);
    into.low = into.low.min(c.low);
    into.close = c.close;
    into.volume += c.volume;
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

/// NSE's intraday volume is per bar, except that one bar near the end of each
/// day carries (roughly) the whole day's volume — e.g. a 5-min SBIN session whose
/// regular bars peak at ~0.35M ends with a 7.27M bar.  When a day's last bar
/// looks like that (the day's largest bar, at least half the rest of the day
/// and over three times any other bar), replace its volume with what is left
/// after subtracting the day's other bars (clamped at 0).  The result is an estimate: NSE's day total
/// does not line up exactly with the bars, so this bar's volume is approximate.
/// It is found by size, not position: NSE has put it before an empty 15:30
/// bar and, in one-minute data, at 15:28 or 15:29.
fn fix_day_total_bar(candles: &mut [ChartCandle]) {
    let mut start = 0;
    while start < candles.len() {
        let day = candle_ist(&candles[start]).map(|dt| dt.date());
        let mut end = start + 1;
        while end < candles.len() && candle_ist(&candles[end]).map(|dt| dt.date()) == day {
            end += 1;
        }
        let day = &mut candles[start..end];
        if day.len() >= 3 {
            let big = (0..day.len()).fold(0, |b, i| if day[i].volume > day[b].volume { i } else { b });
            let total = day[big].volume;
            let (sum, max) = day
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != big)
                .fold((0.0, 0.0f64), |(s, m), (_, c)| (s + c.volume, m.max(c.volume)));
            if total >= 0.5 * sum && total > 3.0 * max {
                day[big].volume = (total - sum).max(0.0);
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
    fn chart_token_is_scripcode_and_ist_date_in_base64() {
        // 2026-11-23 20:00 UTC is already 24 Nov in IST.
        let now = Utc.with_ymd_and_hms(2026, 11, 23, 20, 0, 0).unwrap();
        assert_eq!(chart_token("26000", now), "MjYwMDAtMjQxMTIwMjY=");
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"2885-24112026"), "Mjg4NS0yNDExMjAyNg==");
    }

    #[test]
    fn intervals_parse_or_fail_loudly() {
        assert_eq!(Interval::parse("1").unwrap(), Interval::Minutes(1));
        assert_eq!(Interval::parse("60").unwrap(), Interval::Minutes(60));
        assert_eq!(Interval::parse("375").unwrap(), Interval::Minutes(375));
        assert_eq!(Interval::parse("D").unwrap(), Interval::Eod("D"));
        assert_eq!(Interval::parse("M").unwrap(), Interval::Eod("M"));
        for bad in ["", "0", "-5", "376", "1D", "5m", "d", "1.5"] {
            assert!(Interval::parse(bad).is_err(), "{bad:?} accepted");
        }
    }

    #[tokio::test]
    async fn an_interval_typo_fails_before_any_request() {
        let client = Client::new();
        let token = ("X".to_string(), "1".to_string(), "Equity".to_string());
        let now = Utc::now();
        let err = get_historical_candles_for_token(&client, &HashMap::new(), &token, now - ChronoDuration::days(1), now, "1D").await.unwrap_err();
        assert!(err.to_string().contains("unsupported interval '1D'"), "{err:#}");
    }

    #[tokio::test]
    async fn a_window_older_than_the_horizon_is_an_error() {
        let client = Client::new();
        let token = ("X".to_string(), "1".to_string(), "Equity".to_string());
        let end = Utc::now() - ChronoDuration::days(INTRADAY_HISTORY_DAYS + 5);
        let err = get_historical_candles_for_token(&client, &HashMap::new(), &token, end - ChronoDuration::days(3), end, "5").await.unwrap_err();
        assert!(err.to_string().contains("intraday history"), "{err:#}");
    }

    #[test]
    fn minute_bars_are_floored_and_repeats_dropped() {
        let at = |h, m, s: i64| ist_ms(2026, 11, 20, h, m) + s * 1000;
        let c = |time, o: f64, cl: f64, v| ChartCandle { time, open: o, high: o.max(cl), low: o.min(cl), close: cl, volume: v };
        let rows = vec![
            c(at(9, 16, 59), 3.0, 4.0, 20.0),
            c(at(9, 15, 55), 1.0, 2.0, 10.0),
            c(at(15, 28, 10), 5.0, 5.0, 900.0),
            c(at(15, 28, 49), 5.0, 5.0, 900.0), // the same row again
        ];
        let m = to_minute_bars(rows);
        assert_eq!(m.iter().map(|c| c.time).collect::<Vec<_>>(), vec![at(9, 15, 0), at(9, 16, 0), at(15, 28, 0)]);
        assert_eq!(m[2].volume, 900.0);
    }

    #[test]
    fn resamples_on_the_session_grid() {
        let c = |h, m, v| ChartCandle { time: ist_ms(2026, 11, 23, h, m), open: v, high: v, low: v, close: v, volume: v };
        let bars = vec![c(9, 15, 1.0), c(9, 19, 2.0), c(9, 20, 3.0), c(10, 14, 4.0), c(10, 15, 5.0)];
        let five = resample(&bars, 5);
        assert_eq!(five.iter().map(|b| (b.time, b.volume)).collect::<Vec<_>>(), vec![
            (ist_ms(2026, 11, 23, 9, 15), 3.0),
            (ist_ms(2026, 11, 23, 9, 20), 3.0),
            (ist_ms(2026, 11, 23, 10, 10), 4.0),
            (ist_ms(2026, 11, 23, 10, 15), 5.0),
        ]);
        assert_eq!((five[0].open, five[0].high, five[0].low, five[0].close), (1.0, 2.0, 1.0, 2.0));
        let hour = resample(&bars, 60);
        assert_eq!(hour.iter().map(|b| (b.time, b.volume)).collect::<Vec<_>>(), vec![
            (ist_ms(2026, 11, 23, 9, 15), 10.0),
            (ist_ms(2026, 11, 23, 10, 15), 5.0),
        ]);
    }

    #[test]
    fn day_total_found_by_size_not_position() {
        let mut c = vec![
            candle(ist_ms(2026, 11, 20, 15, 26), 300.0),
            candle(ist_ms(2026, 11, 20, 15, 27), 400.0),
            candle(ist_ms(2026, 11, 20, 15, 28), 5_000.0),
            candle(ist_ms(2026, 11, 20, 15, 29), 50.0),
        ];
        fix_day_total_bar(&mut c);
        assert_eq!(c.iter().map(|c| c.volume).collect::<Vec<_>>(), vec![300.0, 400.0, 4_250.0, 50.0]);
    }

    #[test]
    fn corrects_day_total_ahead_of_an_empty_closing_bar() {
        let mut c = vec![
            candle(ist_ms(2026, 11, 23, 15, 15), 300.0),
            candle(ist_ms(2026, 11, 23, 15, 20), 400.0),
            candle(ist_ms(2026, 11, 23, 15, 25), 5_000.0),
            candle(ist_ms(2026, 11, 23, 15, 30), 0.0),
        ];
        fix_day_total_bar(&mut c);
        assert_eq!(c.iter().map(|c| c.volume).collect::<Vec<_>>(), vec![300.0, 400.0, 4_300.0, 0.0]);
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
