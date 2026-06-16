use std::collections::HashMap;
use reqwest::Client;
use reqwest::header::COOKIE;
use crate::models::{NextApiQuoteResponse, NextApiDerivativesResponse};
use crate::session::format_cookie_header;

const NEXT_API_URL: &str = "https://www.nseindia.com/api/NextApi/apiClient/GetQuoteApi";

/// Fetch live equity quote data for a symbol (e.g. "SBIN", "RELIANCE")
pub async fn get_stock_quote(
    client: &Client,
    cookies: &HashMap<String, String>,
    symbol: &str,
) -> Result<NextApiQuoteResponse, reqwest::Error> {
    let cookie_val = format_cookie_header(cookies);
    
    let resp = client.get(NEXT_API_URL)
        .header(COOKIE, cookie_val)
        .query(&[
            ("functionName", "getSymbolData"),
            ("marketType", "N"),
            ("series", "EQ"),
            ("symbol", symbol),
        ])
        .send()
        .await?;
        
    let response_data = resp.json::<NextApiQuoteResponse>().await?;
    Ok(response_data)
}

/// Fetch live derivatives (futures & options) contracts for a symbol (e.g. "NIFTY", "SBIN")
pub async fn get_derivatives_quote(
    client: &Client,
    cookies: &HashMap<String, String>,
    symbol: &str,
) -> Result<NextApiDerivativesResponse, reqwest::Error> {
    let cookie_val = format_cookie_header(cookies);
    
    let resp = client.get(NEXT_API_URL)
        .header(COOKIE, cookie_val)
        .query(&[
            ("functionName", "getSymbolDerivativesData"),
            ("symbol", symbol),
        ])
        .send()
        .await?;
        
    let response_data = resp.json::<NextApiDerivativesResponse>().await?;
    Ok(response_data)
}
