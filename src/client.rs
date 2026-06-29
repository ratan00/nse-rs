use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::sync::RwLock;
use std::time::{Duration, Instant};
use anyhow::{Context, Result};
use chrono::NaiveDate;
use reqwest::Client;
use reqwest::header::{HeaderMap, HeaderValue};
use tokio::sync::mpsc;
use tokio::time::{interval};
use crate::models::{
    ChartCandle, DerivativeContract, FoBhavRecord, HistoricalRecord,
    NextApiDerivativesResponse, NextApiQuoteResponse, NseIndexQuote, NseQuote,
    OptionChain,
};
use crate::session::{load_session_cache, save_session_cache, fetch_new_cookies};
use crate::{live, historical, archives};

/// Process-wide cache for `get_derivatives_quote` responses keyed by uppercase
/// underlying symbol.  Three independent callers (live feed, chain pane, gamma
/// poller) all hit this endpoint within the same ~3 s window; the cache
/// deduplicates those calls so only one HTTP request is made per TTL window.
type DerivCache = Mutex<HashMap<String, (Instant, NextApiDerivativesResponse)>>;

fn deriv_cache() -> &'static DerivCache {
    static CACHE: OnceLock<DerivCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Cached charting token for a symbol: `(charting_symbol, scripcode, instrument_type)`.
type TokenEntry = (String, String, String);

pub struct NseClient {
    client:      Client,
    cookies:     RwLock<HashMap<String, String>>,
    /// In-memory cache: NSE symbol (uppercase) → charting token triple.
    token_cache: RwLock<HashMap<String, TokenEntry>>,
}

impl NseClient {
    pub fn new() -> Self {
        let mut headers = HeaderMap::new();
        headers.insert("User-Agent",       HeaderValue::from_static("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36"));
        headers.insert("Accept",           HeaderValue::from_static("application/json, text/javascript, */*; q=0.01"));
        headers.insert("Accept-Language",  HeaderValue::from_static("en-US,en;q=0.9"));
        headers.insert("Connection",       HeaderValue::from_static("keep-alive"));
        headers.insert("Referer",          HeaderValue::from_static("https://www.nseindia.com/"));
        headers.insert("X-Requested-With", HeaderValue::from_static("XMLHttpRequest"));

        let client = Client::builder()
            .default_headers(headers)
            .cookie_store(true)
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_default();

        Self {
            client,
            cookies:     RwLock::new(HashMap::new()),
            token_cache: RwLock::new(HashMap::new()),
        }
    }

    // ── Session ───────────────────────────────────────────────────────────────

    /// Load the session from disk cache or request a fresh one.
    pub async fn init_session(&self) -> Result<()> {
        if let Some(cache) = load_session_cache() {
            *self.cookies.write().unwrap() = cache.cookies;
            return Ok(());
        }
        self.force_refresh_session().await
    }

    /// Discard cached cookies and fetch a new session.
    pub async fn force_refresh_session(&self) -> Result<()> {
        let fresh = fetch_new_cookies(&self.client)
            .await
            .context("fetch session cookies")?;
        save_session_cache(&fresh);
        *self.cookies.write().unwrap() = fresh;
        Ok(())
    }

    fn cookies(&self) -> HashMap<String, String> {
        self.cookies.read().unwrap().clone()
    }

    /// Run `f(cookies)` and on session-related failure refresh once and retry.
    async fn with_session_retry<F, Fut, T>(&self, f: F) -> Result<T>
    where
        F: Fn(HashMap<String, String>) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        match f(self.cookies()).await {
            Ok(v) => Ok(v),
            Err(_) => {
                self.force_refresh_session().await?;
                f(self.cookies()).await
            }
        }
    }

    // ── Live equity ───────────────────────────────────────────────────────────

    /// Raw NextApi response for an equity symbol.
    pub async fn get_stock_quote_raw(&self, symbol: &str) -> Result<NextApiQuoteResponse> {
        let sym = symbol.to_string();
        self.with_session_retry(|c| {
            let sym = sym.clone();
            let client = self.client.clone();
            async move { live::get_stock_quote(&client, &c, &sym).await }
        })
        .await
    }

    /// Flat `NseQuote` for an equity symbol.
    pub async fn get_stock_quote(&self, symbol: &str) -> Result<NseQuote> {
        self.get_stock_quote_raw(symbol)
            .await?
            .into_quote()
            .with_context(|| format!("no quote data for '{symbol}'"))
    }

    // ── Live index ────────────────────────────────────────────────────────────

    /// Quote for an NSE index (e.g. `"NIFTY 50"`, `"NIFTY BANK"`).
    pub async fn get_index_quote(&self, index_name: &str) -> Result<NseIndexQuote> {
        let name = index_name.to_string();
        self.with_session_retry(|c| {
            let name = name.clone();
            let client = self.client.clone();
            async move { live::get_index_quote(&client, &c, &name).await }
        })
        .await
    }

    // ── Derivatives ───────────────────────────────────────────────────────────

    pub async fn get_derivatives_quote(&self, symbol: &str) -> Result<NextApiDerivativesResponse> {
        let key = symbol.to_uppercase();
        const TTL: Duration = Duration::from_secs(2);
        // Fast path: check the process-wide cache (3 callers hit this within ~3s).
        {
            let cache = deriv_cache().lock().unwrap();
            if let Some((ts, cached)) = cache.get(&key) {
                if ts.elapsed() < TTL {
                    return Ok(cached.clone());
                }
            }
        }
        // Slow path: fetch from NSE, cache on success regardless of data freshness
        // (NSE returns stale prev-close data outside market hours; we still cache it).
        let sym = symbol.to_string();
        let resp = self.with_session_retry(|c| {
            let sym = sym.clone();
            let client = self.client.clone();
            async move { live::get_derivatives_quote(&client, &c, &sym).await }
        }).await?;
        {
            let mut cache = deriv_cache().lock().unwrap();
            cache.insert(key, (Instant::now(), resp.clone()));
        }
        Ok(resp)
    }

    pub async fn get_futures(&self, symbol: &str) -> Result<Vec<DerivativeContract>> {
        Ok(self
            .get_derivatives_quote(symbol)
            .await?
            .data
            .unwrap_or_default()
            .into_iter()
            .filter(|c| c.instrument_type.starts_with("FUT"))
            .collect())
    }

    pub async fn get_option_contracts(&self, symbol: &str) -> Result<Vec<DerivativeContract>> {
        Ok(self
            .get_derivatives_quote(symbol)
            .await?
            .data
            .unwrap_or_default()
            .into_iter()
            .filter(|c| c.instrument_type.starts_with("OPT"))
            .collect())
    }

    /// Full option chain grouped by expiry date and strike.
    pub async fn get_option_chain(&self, symbol: &str) -> Result<OptionChain> {
        let contracts = self
            .get_derivatives_quote(symbol)
            .await?
            .data
            .unwrap_or_default();
        Ok(OptionChain::from_contracts(symbol, contracts))
    }

    // ── Historical candles ────────────────────────────────────────────────────

    /// Historical OHLCV candles.  Token lookups are cached in memory.
    pub async fn get_historical_candles(
        &self,
        symbol: &str,
        start_time: chrono::DateTime<chrono::Utc>,
        end_time: chrono::DateTime<chrono::Utc>,
        interval: &str,
    ) -> Result<Vec<ChartCandle>> {
        let sym = symbol.to_string();
        let int = interval.to_string();
        // Check token cache first to skip the extra search request.
        let cached = self.token_cache.read().unwrap().get(&sym.to_uppercase()).cloned();
        if cached.is_none() {
            // Warm the token cache.
            let cookies = self.cookies();
            if let Ok(entry) = historical::get_script_token(&self.client, &cookies, &sym).await {
                self.token_cache.write().unwrap().insert(sym.to_uppercase(), entry);
            }
        }
        self.with_session_retry(|c| {
            let sym = sym.clone();
            let int = int.clone();
            let client = self.client.clone();
            async move {
                historical::get_historical_candles(&client, &c, &sym, start_time, end_time, &int).await
            }
        })
        .await
    }

    // ── Polling live feed ─────────────────────────────────────────────────────

    /// Poll `symbol` at `interval_ms` milliseconds and send each `NseQuote` to `tx`.
    /// Stops when `tx` is closed or the task is cancelled.
    /// Automatically refreshes the session on auth failures.
    pub async fn poll_quote(
        &self,
        symbol: &str,
        interval_ms: u64,
        tx: mpsc::Sender<NseQuote>,
    ) {
        let mut ticker = interval(Duration::from_millis(interval_ms));
        loop {
            ticker.tick().await;
            if tx.is_closed() { break; }
            match self.get_stock_quote(symbol).await {
                Ok(q)  => { if tx.send(q).await.is_err() { break; } }
                Err(e) => { eprintln!("poll_quote error for {symbol}: {e:#}"); }
            }
        }
    }

    /// Poll an index at `interval_ms` milliseconds.
    pub async fn poll_index(
        &self,
        index_name: &str,
        interval_ms: u64,
        tx: mpsc::Sender<NseIndexQuote>,
    ) {
        let mut ticker = interval(Duration::from_millis(interval_ms));
        loop {
            ticker.tick().await;
            if tx.is_closed() { break; }
            match self.get_index_quote(index_name).await {
                Ok(q)  => { if tx.send(q).await.is_err() { break; } }
                Err(e) => { eprintln!("poll_index error for {index_name}: {e:#}"); }
            }
        }
    }

    // ── Market status ─────────────────────────────────────────────────────────

    pub async fn get_market_status(&self) -> Result<crate::models::MarketStatusResponse> {
        self.with_session_retry(|c| {
            let client = self.client.clone();
            async move {
                live::get_market_status(&client, &c)
                    .await
            }
        })
        .await
    }

    // ── Archives ─────────────────────────────────────────────────────────────

    pub async fn fetch_full_bhavcopy(&self, date: NaiveDate) -> Result<Vec<HistoricalRecord>> {
        archives::fetch_full_bhavcopy(&self.client, date).await
    }

    pub async fn fetch_zipped_bhavcopy(&self, date: NaiveDate) -> Result<Vec<HistoricalRecord>> {
        archives::fetch_zipped_bhavcopy(&self.client, date).await
    }

    pub async fn fetch_fo_bhavcopy(&self, date: NaiveDate) -> Result<Vec<FoBhavRecord>> {
        archives::fetch_fo_bhavcopy(&self.client, date).await
    }

    pub async fn fetch_symbol_list(&self, date: NaiveDate) -> Result<Vec<String>> {
        archives::fetch_symbol_list(&self.client, date).await
    }
}

impl Default for NseClient {
    fn default() -> Self { Self::new() }
}
