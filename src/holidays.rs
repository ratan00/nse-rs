//! NSE trading-holiday calendar (`/api/holiday-master?type=trading`).

use std::collections::HashMap;
use anyhow::{Context, Result};
use chrono::{Datelike, Duration, NaiveDate, Utc};
use reqwest::header::COOKIE;
use reqwest::Client;
use serde::Deserialize;

use crate::session::format_cookie_header;

const HOLIDAY_URL: &str = "https://www.nseindia.com/api/holiday-master?type=trading";

/// One trading holiday.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NseHoliday {
    pub date: NaiveDate,
    pub description: String,
}

/// Trading holidays per segment, sorted by date and de-duplicated.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TradingHolidays {
    /// Equity (capital market, "CM") holidays.
    pub equity: Vec<NseHoliday>,
    /// Futures & options ("FO") holidays.
    pub fno: Vec<NseHoliday>,
}

#[derive(Deserialize)]
struct RawHoliday {
    #[serde(rename = "tradingDate", default)]
    trading_date: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

/// Current calendar year in IST.
pub fn current_year_ist() -> i32 {
    (Utc::now() + Duration::minutes(330)).year()
}

/// Parse the holiday-master JSON, keeping only holidays in `year` (all years if `None`).
/// Unknown segments, malformed entries and unparsable dates are skipped.
pub fn parse_trading_holidays(json: &str, year: Option<i32>) -> Result<TradingHolidays> {
    let raw: HashMap<String, Vec<RawHoliday>> =
        serde_json::from_str(json).context("holiday-master decode")?;
    let convert = |key: &str| -> Vec<NseHoliday> {
        let mut out: Vec<NseHoliday> = raw
            .get(key)
            .into_iter()
            .flatten()
            .filter_map(|h| {
                let date = NaiveDate::parse_from_str(h.trading_date.as_deref()?.trim(), "%d-%b-%Y").ok()?;
                if year.is_some_and(|y| date.year() != y) {
                    return None;
                }
                Some(NseHoliday { date, description: h.description.clone().unwrap_or_default().trim().to_string() })
            })
            .collect();
        out.sort_by_key(|h| h.date);
        out.dedup_by_key(|h| h.date);
        out
    };
    Ok(TradingHolidays { equity: convert("CM"), fno: convert("FO") })
}

pub async fn fetch_trading_holidays(
    client: &Client,
    cookies: &HashMap<String, String>,
    year: Option<i32>,
) -> Result<TradingHolidays> {
    let body = client
        .get(HOLIDAY_URL)
        .header(COOKIE, format_cookie_header(cookies))
        .send()
        .await
        .context("holiday request")?
        .error_for_status()
        .context("NSE returned an error status (session may have expired)")?
        .text()
        .await
        .context("holiday body")?;
    parse_trading_holidays(&body, year)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
      "CM": [
        {"Sequence": 1, "tradingDate": "26-Jan-2026", "weekDay": "Monday", "description": "Republic Day", "morning_session": "", "evening_session": ""},
        {"tradingDate": "03-Mar-2026", "description": "Holi"},
        {"tradingDate": "31-Dec-2025", "description": "Old"},
        {"tradingDate": "garbage", "description": "Bad"},
        {"description": "No date"}
      ],
      "FO": [
        {"tradingDate": "03-Mar-2026", "description": "Holi"},
        {"tradingDate": "26-Jan-2026", "description": null},
        {"tradingDate": "26-Jan-2026", "description": "dup"}
      ],
      "CD": [{"tradingDate": "01-Jan-2026"}]
    }"#;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate { NaiveDate::from_ymd_opt(y, m, day).unwrap() }

    #[test]
    fn parses_cm_and_fo_for_year() {
        let h = parse_trading_holidays(FIXTURE, Some(2026)).unwrap();
        assert_eq!(h.equity.iter().map(|x| x.date).collect::<Vec<_>>(), vec![d(2026, 1, 26), d(2026, 3, 3)]);
        assert_eq!(h.equity[0].description, "Republic Day");
        assert_eq!(h.fno.iter().map(|x| x.date).collect::<Vec<_>>(), vec![d(2026, 1, 26), d(2026, 3, 3)]);
    }

    #[test]
    fn no_year_filter_keeps_all() {
        let h = parse_trading_holidays(FIXTURE, None).unwrap();
        assert_eq!(h.equity.len(), 3);
        assert_eq!(h.equity[0].date, d(2025, 12, 31));
    }

    #[test]
    fn rejects_non_object_and_tolerates_missing_segments() {
        assert!(parse_trading_holidays("<html>", None).is_err());
        let h = parse_trading_holidays("{}", None).unwrap();
        assert!(h.equity.is_empty() && h.fno.is_empty());
    }
}
