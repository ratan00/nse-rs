use std::collections::HashMap;
use anyhow::{Context, Result};
use reqwest::Client;
use reqwest::header::COOKIE;
use crate::models::{
    NextApiQuoteResponse, NextApiDerivativesResponse, IndexApiResponse, NseIndexQuote,
};
use crate::session::format_cookie_header;

const NEXT_API_URL: &str    = "https://www.nseindia.com/api/NextApi/apiClient/GetQuoteApi";
const MARKET_STATUS_URL: &str = "https://www.nseindia.com/api/marketStatus";
const INDEX_API_URL: &str   = "https://www.nseindia.com/api/indices";

pub async fn get_market_status(
    client: &Client,
    cookies: &HashMap<String, String>,
) -> Result<crate::models::MarketStatusResponse> {
    let cookie_val = format_cookie_header(cookies);
    client
        .get(MARKET_STATUS_URL)
        .header(COOKIE, cookie_val)
        .send()
        .await
        .context("market status request")?
        .json()
        .await
        .context("market status decode")
}

/// Fetch live equity quote for a symbol (e.g. "SBIN", "RELIANCE").
pub async fn get_stock_quote(
    client: &Client,
    cookies: &HashMap<String, String>,
    symbol: &str,
) -> Result<NextApiQuoteResponse> {
    let cookie_val = format_cookie_header(cookies);
    client
        .get(NEXT_API_URL)
        .header(COOKIE, cookie_val)
        .query(&[
            ("functionName", "getSymbolData"),
            ("marketType", "N"),
            ("series", "EQ"),
            ("symbol", symbol),
        ])
        .send()
        .await
        .context("quote request")?
        .json()
        .await
        .context("quote decode")
}

/// Fetch live derivatives (futures & options) for a symbol (e.g. "NIFTY", "SBIN").
pub async fn get_derivatives_quote(
    client: &Client,
    cookies: &HashMap<String, String>,
    symbol: &str,
) -> Result<NextApiDerivativesResponse> {
    let cookie_val = format_cookie_header(cookies);
    client
        .get(NEXT_API_URL)
        .header(COOKIE, cookie_val)
        .query(&[
            ("functionName", "getSymbolDerivativesData"),
            ("symbol", symbol),
        ])
        .send()
        .await
        .context("derivatives request")?
        .json()
        .await
        .context("derivatives decode")
}

/// Fetch LTP and OHLC for an NSE index.
/// `index_name` is the display name as used by NSE, e.g. `"NIFTY 50"`, `"NIFTY BANK"`.
pub async fn get_index_quote(
    client: &Client,
    cookies: &HashMap<String, String>,
    index_name: &str,
) -> Result<NseIndexQuote> {
    let cookie_val = format_cookie_header(cookies);
    let resp: IndexApiResponse = client
        .get(INDEX_API_URL)
        .header(COOKIE, cookie_val)
        .query(&[("index", index_name)])
        .send()
        .await
        .context("index request")?
        .json()
        .await
        .context("index decode")?;

    let entry = resp
        .data
        .unwrap_or_default()
        .into_iter()
        .next()
        .with_context(|| format!("no data for index '{index_name}'"))?;

    Ok(NseIndexQuote {
        name:       entry.index_symbol,
        last:       entry.last,
        open:       entry.open,
        high:       entry.high,
        low:        entry.low,
        prev_close: entry.prev_close,
        change:     entry.change,
        change_pct: entry.change_pct,
    })
}
