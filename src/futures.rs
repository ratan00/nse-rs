//! Daily futures history per contract and continuous (stitched) futures series.
//!
//! NSE's charting API drops expired contracts, so stitching uses the
//! `historicalOR/foCPV` endpoint instead: daily OHLC, settle, volume and OI for
//! every contract, back to at least 2010.  Intraday stitching is not possible —
//! only currently listed contracts have intraday candles.

use std::collections::{BTreeMap, HashMap};
use anyhow::{Context, Result};
use chrono::{Datelike, NaiveDate, Weekday};
use reqwest::Client;
use reqwest::header::COOKIE;
use serde::{Deserialize, Deserializer, Serialize};
use crate::session::format_cookie_header;

const FO_CPV_URL: &str = "https://www.nseindia.com/api/historicalOR/foCPV";

/// foCPV returns at most this many rows per request, keeping the most recent.
/// A response this long may be truncated and its window must be split.
pub const FO_CPV_ROW_CAP: usize = 70;

/// Index underlyings, which trade as `FUTIDX`; everything else is `FUTSTK`.
const INDEX_UNDERLYINGS: &[&str] = &["NIFTY", "BANKNIFTY", "FINNIFTY", "MIDCPNIFTY", "NIFTYNXT50"];

/// One futures contract on one trading day.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FuturesDailyRecord {
    pub symbol:       String,
    pub date:         NaiveDate,
    pub expiry:       NaiveDate,
    pub open:         f64,
    pub high:         f64,
    pub low:          f64,
    pub close:        f64,
    pub settle:       f64,
    /// Traded quantity in units (divide by `lot_size` for contracts).
    pub volume:       f64,
    /// Open interest in units.
    pub oi:           f64,
    pub change_in_oi: f64,
    pub lot_size:     f64,
    /// Spot value of the underlying that day.
    pub underlying:   f64,
}

#[derive(Debug, Deserialize)]
struct FoCpvResponse {
    #[serde(default)]
    data: Vec<FoCpvRow>,
}

#[derive(Debug, Deserialize)]
struct FoCpvRow {
    #[serde(rename = "FH_SYMBOL")]            symbol:       String,
    #[serde(rename = "FH_TIMESTAMP")]         date:         String,
    #[serde(rename = "FH_EXPIRY_DT")]         expiry:       String,
    #[serde(rename = "FH_OPENING_PRICE",    default, deserialize_with = "lenient_f64")] open:   f64,
    #[serde(rename = "FH_TRADE_HIGH_PRICE", default, deserialize_with = "lenient_f64")] high:   f64,
    #[serde(rename = "FH_TRADE_LOW_PRICE",  default, deserialize_with = "lenient_f64")] low:    f64,
    #[serde(rename = "FH_CLOSING_PRICE",    default, deserialize_with = "lenient_f64")] close:  f64,
    #[serde(rename = "FH_SETTLE_PRICE",     default, deserialize_with = "lenient_f64")] settle: f64,
    #[serde(rename = "FH_TOT_TRADED_QTY",   default, deserialize_with = "lenient_f64")] volume: f64,
    #[serde(rename = "FH_OPEN_INT",         default, deserialize_with = "lenient_f64")] oi:     f64,
    #[serde(rename = "FH_CHANGE_IN_OI",     default, deserialize_with = "lenient_f64")] change_in_oi: f64,
    #[serde(rename = "FH_MARKET_LOT",       default, deserialize_with = "lenient_f64")] lot_size: f64,
    #[serde(rename = "FH_UNDERLYING_VALUE", default, deserialize_with = "lenient_f64")] underlying: f64,
}

/// Accept a number, a numeric string, or null/"-" (→ 0).
fn lenient_f64<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0),
        serde_json::Value::String(s) => s.trim().replace(',', "").parse().unwrap_or(0.0),
        _ => 0.0,
    })
}

fn parse_nse_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s.trim(), "%d-%b-%Y").ok()
}

/// `FUTIDX` for index underlyings, `FUTSTK` otherwise.
pub fn futures_instrument_type(symbol: &str) -> &'static str {
    if INDEX_UNDERLYINGS.iter().any(|s| s.eq_ignore_ascii_case(symbol)) {
        "FUTIDX"
    } else {
        "FUTSTK"
    }
}

/// Daily records for every futures contract of `symbol` between `from` and `to`
/// (one request).  Keep windows short — see [`FO_CPV_ROW_CAP`].
pub async fn fetch_futures_daily_window(
    client: &Client,
    cookies: &HashMap<String, String>,
    symbol: &str,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<FuturesDailyRecord>> {
    let resp: FoCpvResponse = client
        .get(FO_CPV_URL)
        .header(COOKIE, format_cookie_header(cookies))
        .query(&[
            ("from",           from.format("%d-%m-%Y").to_string().as_str()),
            ("to",             to.format("%d-%m-%Y").to_string().as_str()),
            ("instrumentType", futures_instrument_type(symbol)),
            ("symbol",         symbol.to_uppercase().as_str()),
        ])
        .send()
        .await
        .context("futures history request")?
        .error_for_status()
        .context("NSE returned an error status (session may have expired)")?
        .json()
        .await
        .context("futures history decode")?;

    Ok(resp
        .data
        .into_iter()
        .filter_map(|r| {
            Some(FuturesDailyRecord {
                date:         parse_nse_date(&r.date)?,
                expiry:       parse_nse_date(&r.expiry)?,
                symbol:       r.symbol,
                open:         r.open,
                high:         r.high,
                low:          r.low,
                close:        r.close,
                settle:       r.settle,
                volume:       r.volume,
                oi:           r.oi,
                change_in_oi: r.change_in_oi,
                lot_size:     r.lot_size,
                underlying:   r.underlying,
            })
        })
        .collect())
}

// ── Continuous series ─────────────────────────────────────────────────────────

/// When to switch from the expiring contract to the next one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollRule {
    /// Hold each contract through its expiry day.
    Expiry,
    /// Switch on the first day with fewer than `n` trading days left before
    /// expiry: `1` switches on expiry day, `3` two days before it, `0` is the same as `Expiry`.
    DaysBeforeExpiry(u32),
    /// Switch on the first day the next contract's open interest exceeds the current one's.
    OpenInterest,
    /// Switch on the first day the next contract's traded volume exceeds the current one's.
    Volume,
}

/// How to remove the price jump at each roll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Adjustment {
    /// Raw prices; the series jumps at every roll.
    None,
    /// Back-adjust by the price difference at each roll (keeps point moves; old prices can drift far).
    Difference,
    /// Back-adjust by the price ratio at each roll (keeps percentage returns; never goes negative).
    Ratio,
}

#[derive(Debug, Clone, Copy)]
pub struct ContinuousOptions {
    pub roll:       RollRule,
    pub adjustment: Adjustment,
}

impl Default for ContinuousOptions {
    fn default() -> Self {
        Self { roll: RollRule::Expiry, adjustment: Adjustment::Ratio }
    }
}

/// One day of a continuous series.  Prices are adjusted; `volume` / `oi` are the
/// held contract's own values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContinuousBar {
    pub date:     NaiveDate,
    pub open:     f64,
    pub high:     f64,
    pub low:      f64,
    pub close:    f64,
    pub settle:   f64,
    pub volume:   f64,
    pub oi:       f64,
    /// Expiry of the contract this bar comes from.
    pub contract: NaiveDate,
    /// Unadjusted close of that contract.
    pub raw_close: f64,
}

/// A switch from one contract to the next.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RollEvent {
    /// First day the new contract is held.
    pub date:       NaiveDate,
    pub from:       NaiveDate,
    pub to:         NaiveDate,
    /// Closes of both contracts on the last day the old one was held.
    pub from_close: f64,
    pub to_close:   f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContinuousFutures {
    pub symbol: String,
    pub bars:   Vec<ContinuousBar>,
    pub rolls:  Vec<RollEvent>,
}

/// A traded price for a record: close, or settle when NSE reports no close.
fn price(r: &FuturesDailyRecord) -> f64 {
    if r.close > 0.0 { r.close } else { r.settle }
}

/// Mon–Fri days in `(from, to]` — used for expiries beyond the fetched data.
fn weekdays_between(from: NaiveDate, to: NaiveDate) -> u32 {
    from.iter_days()
        .skip(1)
        .take_while(|d| *d <= to)
        .filter(|d| !matches!(d.weekday(), Weekday::Sat | Weekday::Sun))
        .count() as u32
}

/// Stitch per-contract daily records into one front-month series.
pub fn stitch_continuous(
    symbol: &str,
    records: &[FuturesDailyRecord],
    options: ContinuousOptions,
) -> ContinuousFutures {
    // date → contracts trading that day, sorted by expiry.
    let mut by_date: BTreeMap<NaiveDate, Vec<&FuturesDailyRecord>> = BTreeMap::new();
    for r in records.iter().filter(|r| r.expiry >= r.date && price(r) > 0.0) {
        by_date.entry(r.date).or_default().push(r);
    }
    for day in by_date.values_mut() {
        day.sort_by_key(|r| r.expiry);
    }
    let dates: Vec<NaiveDate> = by_date.keys().copied().collect();
    let last_date = dates.last().copied();

    // Trading days strictly after `date` up to and including `expiry`.
    let trading_days_left = |date: NaiveDate, expiry: NaiveDate| -> u32 {
        match last_date {
            Some(last) if expiry <= last => {
                let lo = dates.partition_point(|d| *d <= date);
                let hi = dates.partition_point(|d| *d <= expiry);
                (hi - lo) as u32
            }
            _ => weekdays_between(date, expiry),
        }
    };

    let mut held: Vec<(NaiveDate, &FuturesDailyRecord)> = Vec::new();
    let mut rolls = Vec::new();
    let mut current: Option<NaiveDate> = None;

    for (&date, day) in &by_date {
        let cur_rec = current.and_then(|exp| day.iter().copied().find(|r| r.expiry == exp));
        let next_rec = |exp: NaiveDate| day.iter().copied().find(|r| r.expiry > exp);

        let switch_to = match (current, cur_rec) {
            (None, _) => Some(day[0]),
            // The held contract expired or did not trade: take the nearest one
            // not older than it, so an early roll never goes backwards.
            (Some(exp), None) => day.iter().copied().find(|r| r.expiry >= exp),
            (Some(exp), Some(cur)) => next_rec(exp).filter(|next| match options.roll {
                RollRule::Expiry              => false,
                RollRule::DaysBeforeExpiry(n) => trading_days_left(date, exp) < n,
                RollRule::OpenInterest        => next.oi > cur.oi,
                RollRule::Volume              => next.volume > cur.volume,
            }),
        };

        if let Some(new) = switch_to {
            if let (Some(old_exp), Some((prev_date, prev))) = (current, held.last())
                && new.expiry != old_exp
            {
                // Gap measured on the last day the old contract was held,
                // when both contracts traded.
                let new_then = by_date[prev_date].iter().find(|r| r.expiry == new.expiry);
                if let Some(new_then) = new_then {
                    rolls.push(RollEvent {
                        date,
                        from:       old_exp,
                        to:         new.expiry,
                        from_close: price(prev),
                        to_close:   price(new_then),
                    });
                }
            }
            current = Some(new.expiry);
            held.push((date, new));
        } else if let Some(cur) = cur_rec {
            held.push((date, cur));
        }
        // Otherwise only older contracts traded today: skip the day.
    }

    // Back-adjust: walk from the newest bar, accumulating the gap of each roll passed.
    let mut offset = 0.0;
    let mut factor = 1.0;
    let mut pending = rolls.iter().rev().peekable();
    let mut bars: Vec<ContinuousBar> = Vec::with_capacity(held.len());
    for (date, r) in held.iter().rev() {
        while let Some(roll) = pending.peek() {
            if roll.date > *date {
                offset += roll.to_close - roll.from_close;
                if roll.from_close > 0.0 {
                    factor *= roll.to_close / roll.from_close;
                }
                pending.next();
            } else {
                break;
            }
        }
        let adj = |p: f64| match options.adjustment {
            Adjustment::None       => p,
            Adjustment::Difference => p + offset,
            Adjustment::Ratio      => p * factor,
        };
        bars.push(ContinuousBar {
            date:      *date,
            open:      adj(r.open),
            high:      adj(r.high),
            low:       adj(r.low),
            close:     adj(price(r)),
            settle:    adj(r.settle),
            volume:    r.volume,
            oi:        r.oi,
            contract:  r.expiry,
            raw_close: price(r),
        });
    }
    bars.reverse();

    ContinuousFutures { symbol: symbol.to_uppercase(), bars, rolls }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn rec(date: NaiveDate, expiry: NaiveDate, close: f64, oi: f64) -> FuturesDailyRecord {
        FuturesDailyRecord {
            symbol: "NIFTY".into(), date, expiry,
            open: close, high: close, low: close, close, settle: close,
            volume: oi, oi, change_in_oi: 0.0, lot_size: 65.0, underlying: close,
        }
    }

    /// Near (exp Jan 8) at 100,101,102 and next (exp Feb 5) at 110,111,112,113,114.
    /// OI of the next contract overtakes on Jan 7.
    fn sample() -> Vec<FuturesDailyRecord> {
        let jan = d(2026, 1, 8);
        let feb = d(2026, 2, 5);
        vec![
            rec(d(2026, 1, 6), jan, 100.0, 50.0), rec(d(2026, 1, 6), feb, 110.0, 10.0),
            rec(d(2026, 1, 7), jan, 101.0, 30.0), rec(d(2026, 1, 7), feb, 111.0, 40.0),
            rec(d(2026, 1, 8), jan, 102.0, 5.0),  rec(d(2026, 1, 8), feb, 112.0, 60.0),
            rec(d(2026, 1, 9), feb, 113.0, 60.0),
            rec(d(2026, 1, 12), feb, 114.0, 60.0),
        ]
    }

    fn closes(c: &ContinuousFutures) -> Vec<f64> {
        c.bars.iter().map(|b| b.close).collect()
    }

    #[test]
    fn expiry_roll_raw() {
        let c = stitch_continuous("NIFTY", &sample(),
            ContinuousOptions { roll: RollRule::Expiry, adjustment: Adjustment::None });
        assert_eq!(closes(&c), vec![100.0, 101.0, 102.0, 113.0, 114.0]);
        assert_eq!(c.rolls.len(), 1);
        let r = &c.rolls[0];
        assert_eq!((r.date, r.from_close, r.to_close), (d(2026, 1, 9), 102.0, 112.0));
    }

    #[test]
    fn expiry_roll_difference_adjusted() {
        let c = stitch_continuous("NIFTY", &sample(),
            ContinuousOptions { roll: RollRule::Expiry, adjustment: Adjustment::Difference });
        assert_eq!(closes(&c), vec![110.0, 111.0, 112.0, 113.0, 114.0]);
    }

    #[test]
    fn ratio_adjustment_scales_old_bars() {
        let c = stitch_continuous("NIFTY", &sample(),
            ContinuousOptions { roll: RollRule::Expiry, adjustment: Adjustment::Ratio });
        let f = 112.0 / 102.0;
        let want = [100.0 * f, 101.0 * f, 102.0 * f, 113.0, 114.0];
        for (got, want) in closes(&c).iter().zip(want) {
            assert!((got - want).abs() < 1e-9, "{got} vs {want}");
        }
        // Unadjusted prices stay available.
        assert_eq!(c.bars[0].raw_close, 100.0);
    }

    #[test]
    fn open_interest_roll() {
        let c = stitch_continuous("NIFTY", &sample(),
            ContinuousOptions { roll: RollRule::OpenInterest, adjustment: Adjustment::None });
        assert_eq!(closes(&c), vec![100.0, 111.0, 112.0, 113.0, 114.0]);
        assert_eq!(c.rolls[0].date, d(2026, 1, 7));
        assert_eq!((c.rolls[0].from_close, c.rolls[0].to_close), (100.0, 110.0));
    }

    #[test]
    fn days_before_expiry_roll() {
        // On Jan 7 one trading day (Jan 8) is left → roll with n = 2.
        let c = stitch_continuous("NIFTY", &sample(),
            ContinuousOptions { roll: RollRule::DaysBeforeExpiry(2), adjustment: Adjustment::None });
        assert_eq!(c.rolls[0].date, d(2026, 1, 7));
        // n = 0 behaves like Expiry.
        let c0 = stitch_continuous("NIFTY", &sample(),
            ContinuousOptions { roll: RollRule::DaysBeforeExpiry(0), adjustment: Adjustment::None });
        assert_eq!(c0.rolls[0].date, d(2026, 1, 9));
    }

    #[test]
    fn missing_day_after_early_roll_does_not_go_back() {
        let jan = d(2026, 1, 8);
        let feb = d(2026, 2, 5);
        let recs = vec![
            rec(d(2026, 1, 5), jan, 100.0, 50.0), rec(d(2026, 1, 5), feb, 110.0, 10.0),
            rec(d(2026, 1, 6), jan, 101.0, 10.0), rec(d(2026, 1, 6), feb, 111.0, 50.0), // OI roll
            rec(d(2026, 1, 7), jan, 102.0, 10.0), // feb has no row: skip, don't go back to jan
            rec(d(2026, 1, 9), feb, 113.0, 50.0),
        ];
        let c = stitch_continuous("NIFTY", &recs,
            ContinuousOptions { roll: RollRule::OpenInterest, adjustment: Adjustment::None });
        let held: Vec<_> = c.bars.iter().map(|b| (b.date, b.contract)).collect();
        assert_eq!(held, vec![(d(2026, 1, 5), jan), (d(2026, 1, 6), feb), (d(2026, 1, 9), feb)]);
        assert_eq!(c.rolls.len(), 1);
    }

    #[test]
    fn decodes_focpv_rows() {
        let json = r#"{"data":[{"FH_SYMBOL":"NIFTY","FH_TIMESTAMP":"29-Sep-2026","FH_EXPIRY_DT":"29-Sep-2026",
            "FH_OPENING_PRICE":22755.5,"FH_TRADE_HIGH_PRICE":"22764.9","FH_TRADE_LOW_PRICE":22579,
            "FH_CLOSING_PRICE":22687.8,"FH_SETTLE_PRICE":22716.2,"FH_TOT_TRADED_QTY":5117450,
            "FH_OPEN_INT":6623305,"FH_CHANGE_IN_OI":-2228720,"FH_MARKET_LOT":65,"FH_UNDERLYING_VALUE":null}]}"#;
        let resp: FoCpvResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.data[0].high, 22764.9);
        assert_eq!(resp.data[0].underlying, 0.0);
    }

    #[test]
    fn instrument_type() {
        assert_eq!(futures_instrument_type("nifty"), "FUTIDX");
        assert_eq!(futures_instrument_type("RELIANCE"), "FUTSTK");
    }
}
