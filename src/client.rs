use std::collections::HashMap;
use std::sync::RwLock;
use chrono::{DateTime, Utc, NaiveDate};
use reqwest::Client;
use reqwest::header::{HeaderMap, HeaderValue};
use crate::models::{NextApiQuoteResponse, NextApiDerivativesResponse, ChartCandle, HistoricalRecord};
use crate::session::{load_session_cache, save_session_cache, fetch_new_cookies};
use crate::live;
use crate::historical;
use crate::archives;

pub struct NseClient {
    client: Client,
    cookies: RwLock<HashMap<String, String>>,
}

impl NseClient {
    /// Create a new NseClient instance with configured request headers
    pub fn new() -> Self {
        let mut headers = HeaderMap::new();
        headers.insert("User-Agent", HeaderValue::from_static("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36"));
        headers.insert("Accept", HeaderValue::from_static("application/json, text/javascript, */*; q=0.01"));
        headers.insert("Accept-Language", HeaderValue::from_static("en-US,en;q=0.9"));
        headers.insert("Connection", HeaderValue::from_static("keep-alive"));
        headers.insert("Referer", HeaderValue::from_static("https://www.nseindia.com/"));
        headers.insert("X-Requested-With", HeaderValue::from_static("XMLHttpRequest"));

        let client = Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            cookies: RwLock::new(HashMap::new()),
        }
    }

    /// Load the session from cache or request a new one
    pub async fn init_session(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // 1. Try to load cached session
        if let Some(cache) = load_session_cache() {
            let mut cookies_lock = self.cookies.write().map_err(|e| format!("Lock error: {}", e))?;
            *cookies_lock = cache.cookies;
            return Ok(());
        }

        // 2. Fetch new cookies if cache is missing/expired
        let fresh_cookies = fetch_new_cookies(&self.client).await?;
        
        // 3. Save to disk cache
        save_session_cache(&fresh_cookies);
        
        // 4. Store in memory
        let mut cookies_lock = self.cookies.write().map_err(|e| format!("Lock error: {}", e))?;
        *cookies_lock = fresh_cookies;
        
        Ok(())
    }

    /// Refresh the session forcefully, ignoring cached values
    pub async fn force_refresh_session(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let fresh_cookies = fetch_new_cookies(&self.client).await?;
        save_session_cache(&fresh_cookies);
        
        let mut cookies_lock = self.cookies.write().map_err(|e| format!("Lock error: {}", e))?;
        *cookies_lock = fresh_cookies;
        Ok(())
    }

    /// Get live stock quote
    pub async fn get_stock_quote(&self, symbol: &str) -> Result<NextApiQuoteResponse, Box<dyn std::error::Error + Send + Sync>> {
        let cookies = self.cookies.read().map_err(|e| format!("Lock error: {}", e))?.clone();
        
        // Attempt request; if it fails with empty response/forbidden, refresh cookie once
        match live::get_stock_quote(&self.client, &cookies, symbol).await {
            Ok(res) => Ok(res),
            Err(e) => {
                // If it looks like a blocked session, retry after cookie refresh
                if e.is_status() || e.is_decode() {
                    self.force_refresh_session().await?;
                    let fresh_cookies = self.cookies.read().map_err(|e| format!("Lock error: {}", e))?.clone();
                    let retry_res = live::get_stock_quote(&self.client, &fresh_cookies, symbol).await?;
                    Ok(retry_res)
                } else {
                    Err(Box::new(e))
                }
            }
        }
    }

    /// Get derivative options and futures quote
    pub async fn get_derivatives_quote(&self, symbol: &str) -> Result<NextApiDerivativesResponse, Box<dyn std::error::Error + Send + Sync>> {
        let cookies = self.cookies.read().map_err(|e| format!("Lock error: {}", e))?.clone();
        
        match live::get_derivatives_quote(&self.client, &cookies, symbol).await {
            Ok(res) => Ok(res),
            Err(e) => {
                if e.is_status() || e.is_decode() {
                    self.force_refresh_session().await?;
                    let fresh_cookies = self.cookies.read().map_err(|e| format!("Lock error: {}", e))?.clone();
                    let retry_res = live::get_derivatives_quote(&self.client, &fresh_cookies, symbol).await?;
                    Ok(retry_res)
                } else {
                    Err(Box::new(e))
                }
            }
        }
    }

    /// Get historical charting candles (intraday minutes or daily/weekly/monthly)
    pub async fn get_historical_candles(
        &self,
        symbol: &str,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
        interval: &str,
    ) -> Result<Vec<ChartCandle>, Box<dyn std::error::Error + Send + Sync>> {
        let cookies = self.cookies.read().map_err(|e| format!("Lock error: {}", e))?.clone();
        
        match historical::get_historical_candles(&self.client, &cookies, symbol, start_time, end_time, interval).await {
            Ok(res) => Ok(res),
            Err(_) => {
                // Refresh session on any session-related charting failure
                self.force_refresh_session().await?;
                let fresh_cookies = self.cookies.read().map_err(|e| format!("Lock error: {}", e))?.clone();
                let retry_res = historical::get_historical_candles(&self.client, &fresh_cookies, symbol, start_time, end_time, interval).await?;
                Ok(retry_res)
            }
        }
    }

    /// Download and parse full bhavcopy CSV containing delivery details
    pub async fn fetch_full_bhavcopy(&self, date: NaiveDate) -> Result<Vec<HistoricalRecord>, Box<dyn std::error::Error + Send + Sync>> {
        archives::fetch_full_bhavcopy(&self.client, date).await
    }

    /// Download and parse standard zipped bhavcopy (useful for older historical files)
    pub async fn fetch_zipped_bhavcopy(&self, date: NaiveDate) -> Result<Vec<HistoricalRecord>, Box<dyn std::error::Error + Send + Sync>> {
        archives::fetch_zipped_bhavcopy(&self.client, date).await
    }
}

impl Default for NseClient {
    fn default() -> Self {
        Self::new()
    }
}
