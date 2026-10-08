use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::sync::RwLock;
use std::time::{Duration, Instant};
use anyhow::{Context, Result};
use chrono::NaiveDate;
use reqwest::Client;
use reqwest::header::{HeaderMap, HeaderValue};
use tokio::sync::mpsc;
use tokio::time::{interval, sleep, MissedTickBehavior};
use crate::models::{
    ChartCandle, DerivativeContract, FoBhavRecord, HistoricalRecord,
    NextApiDerivativesResponse, NextApiQuoteResponse, NseIndexQuote, NseQuote,
    OptionChain,
};
use crate::ratelimit::{backoff, RateLimiter};
use crate::session::{load_session_cache, save_session_cache, fetch_new_cookies};
use crate::{live, historical, archives};

/// How long a `get_derivatives_quote` response is reused.
const DERIV_TTL: Duration = Duration::from_secs(2);
/// Cached derivatives responses older than this are dropped (option chains are large).
const DERIV_EVICT_AFTER: Duration = Duration::from_secs(60);

/// Retries for 429 / 5xx / network errors (session failures get one refresh instead).
const MAX_RETRIES: u32 = 2;
const RETRY_BASE: Duration = Duration::from_millis(500);
const RETRY_MAX: Duration = Duration::from_secs(10);
/// Pause applied to the whole client when NSE still answers 403 with fresh cookies —
/// that is Akamai throttling the IP, and continuing only extends the block.
const BLOCKED_COOLDOWN: Duration = Duration::from_secs(15);

/// Smallest polling interval accepted by `poll_*`.
pub const MIN_POLL_INTERVAL_MS: u64 = 500;
/// Upper bound for the error backoff inside polling loops.
const POLL_MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Process-wide cache for `get_derivatives_quote` responses keyed by uppercase
/// underlying symbol.  Three independent callers (live feed, chain pane, gamma
/// poller) all hit this endpoint within the same ~3 s window; the cache
/// deduplicates those calls so only one HTTP request is made per TTL window.
/// Each symbol has its own async lock so concurrent misses wait for one fetch
/// instead of all going to NSE.
type DerivSlot = Arc<tokio::sync::Mutex<Option<(Instant, NextApiDerivativesResponse)>>>;
type DerivCache = Mutex<HashMap<String, DerivSlot>>;

fn deriv_cache() -> &'static DerivCache {
    static CACHE: OnceLock<DerivCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Drop derivatives responses nobody has refreshed recently.
fn evict_stale_derivs() {
    let mut map = deriv_cache().lock().unwrap_or_else(|e| e.into_inner());
    map.retain(|_, slot| match slot.try_lock() {
        Ok(mut entry) => {
            if entry.as_ref().is_some_and(|(ts, _)| ts.elapsed() >= DERIV_EVICT_AFTER) {
                *entry = None;
            }
            entry.is_some() || Arc::strong_count(slot) > 1
        }
        Err(_) => true, // a fetch is in progress
    });
}

/// Cached charting token for a symbol: `(charting_symbol, scripcode, instrument_type)`.
type TokenEntry = (String, String, String);

/// Tuning knobs for [`NseClient`].
///
/// NSE publishes no rate limit; it sits behind Akamai, which answers sustained
/// bursts with 403s and temporary IP blocks.  The defaults stay well inside
/// what is commonly reported as safe (~3 requests/s per IP).
#[derive(Debug, Clone)]
pub struct NseConfig {
    /// Maximum request starts per second across the whole client. `<= 0` disables pacing.
    pub requests_per_sec: f64,
    /// Maximum requests in flight at once.
    pub max_concurrent: usize,
    /// Per-request timeout.
    pub timeout: Duration,
}

impl Default for NseConfig {
    fn default() -> Self {
        Self {
            requests_per_sec: 3.0,
            max_concurrent:   4,
            timeout:          Duration::from_secs(45),
        }
    }
}

/// Cookies plus a generation counter that increments on every refresh, so
/// concurrent callers that failed with the same cookies trigger one refresh.
#[derive(Default)]
struct Session {
    generation: u64,
    cookies:    HashMap<String, String>,
}

/// How a failed request should be handled.
#[derive(Debug, PartialEq, Eq)]
enum Failure {
    /// 401/403 or an HTML page instead of JSON — cookies expired (or IP throttled).
    Session,
    /// 429.
    Throttled,
    /// 5xx, timeout, connection error.
    Transient,
    /// Anything else (404, bad symbol, …) — retrying will not help.
    Fatal,
}

fn classify(err: &anyhow::Error) -> Failure {
    for cause in err.chain() {
        if let Some(e) = cause.downcast_ref::<reqwest::Error>() {
            if let Some(status) = e.status() {
                return match status.as_u16() {
                    401 | 403 => Failure::Session,
                    429       => Failure::Throttled,
                    500..=599 => Failure::Transient,
                    _         => Failure::Fatal,
                };
            }
            if e.is_decode() {
                return Failure::Session;
            }
            return Failure::Transient;
        }
    }
    Failure::Fatal
}

pub struct NseClient {
    client:        Client,
    session:       RwLock<Session>,
    /// Serialises session refreshes so a burst of 403s causes one refresh.
    refresh_lock:  tokio::sync::Mutex<()>,
    limiter:       RateLimiter,
    /// In-memory cache: NSE symbol (uppercase) → charting token triple.
    token_cache:   RwLock<HashMap<String, TokenEntry>>,
}

impl NseClient {
    pub fn new() -> Self {
        Self::with_config(NseConfig::default())
    }

    pub fn with_config(config: NseConfig) -> Self {
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
            .connect_timeout(Duration::from_secs(10))
            .timeout(config.timeout)
            .build()
            .unwrap_or_default();

        Self {
            client,
            session:      RwLock::new(Session::default()),
            refresh_lock: tokio::sync::Mutex::new(()),
            limiter:      RateLimiter::new(config.requests_per_sec, config.max_concurrent),
            token_cache:  RwLock::new(HashMap::new()),
        }
    }

    // ── Session ───────────────────────────────────────────────────────────────

    /// Load the session from disk cache or request a fresh one.
    pub async fn init_session(&self) -> Result<()> {
        if let Some(cache) = load_session_cache() {
            self.store_cookies(cache.cookies);
            return Ok(());
        }
        self.force_refresh_session().await
    }

    /// Discard cached cookies and fetch a new session.
    pub async fn force_refresh_session(&self) -> Result<()> {
        let _guard = self.refresh_lock.lock().await;
        self.refresh_locked().await
    }

    /// Refresh the session unless another task already did so since `seen_generation`.
    async fn refresh_session_after(&self, seen_generation: u64) -> Result<()> {
        let _guard = self.refresh_lock.lock().await;
        if self.snapshot().0 != seen_generation {
            return Ok(());
        }
        self.refresh_locked().await
    }

    /// Caller must hold `refresh_lock`.
    async fn refresh_locked(&self) -> Result<()> {
        // The warm-up loads two pages; book both slots with the limiter.
        let fresh = {
            let _permit = self.limiter.acquire_weighted(2).await;
            fetch_new_cookies(&self.client)
                .await
                .context("fetch session cookies")?
        };
        save_session_cache(&fresh);
        self.store_cookies(fresh);
        Ok(())
    }

    fn store_cookies(&self, cookies: HashMap<String, String>) {
        let mut session = self.session.write().unwrap_or_else(|e| e.into_inner());
        session.generation += 1;
        session.cookies = cookies;
    }

    fn snapshot(&self) -> (u64, HashMap<String, String>) {
        let session = self.session.read().unwrap_or_else(|e| e.into_inner());
        (session.generation, session.cookies.clone())
    }

    /// Send one request through the rate limiter and recover from failures:
    /// - expired session → one shared refresh, then retry;
    /// - still rejected after refreshing → client-wide cool-down, give up;
    /// - 429 / 5xx / network → exponential backoff, up to `MAX_RETRIES`;
    /// - anything else → return the error immediately.
    ///
    /// `f` must perform exactly one HTTP request.
    async fn request<F, Fut, T>(&self, f: F) -> Result<T>
    where
        F: Fn(Client, HashMap<String, String>) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let mut refreshed = false;
        let mut attempt = 0;
        loop {
            let (generation, cookies) = self.snapshot();
            let result = {
                let _permit = self.limiter.acquire().await;
                f(self.client.clone(), cookies).await
            };
            let err = match result {
                Ok(v) => return Ok(v),
                Err(e) => e,
            };
            match classify(&err) {
                Failure::Session if !refreshed => {
                    refreshed = true;
                    self.refresh_session_after(generation).await?;
                }
                Failure::Session => {
                    self.limiter.cool_down(BLOCKED_COOLDOWN).await;
                    return Err(err.context(
                        "NSE rejected the request even with a fresh session (likely rate-limited)",
                    ));
                }
                Failure::Throttled if attempt < MAX_RETRIES => {
                    self.limiter.cool_down(backoff(RETRY_BASE, attempt, RETRY_MAX)).await;
                    attempt += 1;
                }
                Failure::Transient if attempt < MAX_RETRIES => {
                    sleep(backoff(RETRY_BASE, attempt, RETRY_MAX)).await;
                    attempt += 1;
                }
                _ => return Err(err),
            }
        }
    }

    // ── Live equity ───────────────────────────────────────────────────────────

    /// Raw NextApi response for an equity symbol.
    pub async fn get_stock_quote_raw(&self, symbol: &str) -> Result<NextApiQuoteResponse> {
        let sym = symbol.to_string();
        self.request(|client, c| {
            let sym = sym.clone();
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
        self.request(|client, c| {
            let name = name.clone();
            async move { live::get_index_quote(&client, &c, &name).await }
        })
        .await
    }

    /// Every index NSE publishes, from a single request.
    pub async fn get_all_indices(&self) -> Result<Vec<NseIndexQuote>> {
        self.request(|client, c| async move { live::get_all_indices(&client, &c).await })
            .await
    }

    /// Quotes for several indices from a single request, in the order asked.
    /// Fails if any name is unknown.
    pub async fn get_index_quotes(&self, index_names: &[&str]) -> Result<Vec<NseIndexQuote>> {
        let all = self.get_all_indices().await?;
        index_names
            .iter()
            .map(|name| {
                all.iter()
                    .find(|q| q.name.eq_ignore_ascii_case(name))
                    .cloned()
                    .with_context(|| format!("no data for index '{name}'"))
            })
            .collect()
    }

    // ── Derivatives ───────────────────────────────────────────────────────────

    pub async fn get_derivatives_quote(&self, symbol: &str) -> Result<NextApiDerivativesResponse> {
        let key = symbol.to_uppercase();
        let slot = deriv_cache()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(key)
            .or_default()
            .clone();
        // Concurrent callers for the same symbol queue here and reuse the
        // response the first one fetched.
        let mut entry = slot.lock().await;
        if let Some((ts, cached)) = entry.as_ref()
            && ts.elapsed() < DERIV_TTL
        {
            return Ok(cached.clone());
        }
        // Cache on success regardless of data freshness
        // (NSE returns stale prev-close data outside market hours; we still cache it).
        let sym = symbol.to_string();
        let resp = self.request(|client, c| {
            let sym = sym.clone();
            async move { live::get_derivatives_quote(&client, &c, &sym).await }
        }).await?;
        *entry = Some((Instant::now(), resp.clone()));
        drop(entry);
        evict_stale_derivs();
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

    /// Historical OHLCV candles.  The symbol's charting token is looked up once
    /// and cached, so repeat calls cost a single request.
    pub async fn get_historical_candles(
        &self,
        symbol: &str,
        start_time: chrono::DateTime<chrono::Utc>,
        end_time: chrono::DateTime<chrono::Utc>,
        interval: &str,
    ) -> Result<Vec<ChartCandle>> {
        let key = symbol.to_uppercase();
        let cached = self.token_cache.read().unwrap_or_else(|e| e.into_inner()).get(&key).cloned();
        let token = match cached {
            Some(token) => token,
            None => {
                let sym = symbol.to_string();
                let token = self.request(|client, c| {
                    let sym = sym.clone();
                    async move { historical::get_script_token(&client, &c, &sym).await }
                })
                .await?;
                self.token_cache
                    .write()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(key.clone(), token.clone());
                token
            }
        };
        let int = interval.to_string();
        let result = self.request(|client, c| {
            let token = token.clone();
            let int = int.clone();
            async move {
                historical::get_historical_candles_for_token(&client, &c, &token, start_time, end_time, &int).await
            }
        })
        .await;
        if result.is_err() {
            // The token may be stale (symbol renamed / relisted); look it up again next time.
            self.token_cache.write().unwrap_or_else(|e| e.into_inner()).remove(&key);
        }
        result
    }

    // ── Polling live feed ─────────────────────────────────────────────────────

    /// Poll `symbol` every `interval_ms` milliseconds (minimum
    /// [`MIN_POLL_INTERVAL_MS`]) and send each `NseQuote` to `tx`.
    /// Stops when `tx` is closed or the task is cancelled.
    /// Errors are logged and back off exponentially (up to 60 s) instead of
    /// retrying at full speed.
    pub async fn poll_quote(
        &self,
        symbol: &str,
        interval_ms: u64,
        tx: mpsc::Sender<NseQuote>,
    ) {
        let label = format!("poll_quote {symbol}");
        self.poll_loop(&label, interval_ms, tx, || self.get_stock_quote(symbol)).await
    }

    /// Poll an index every `interval_ms` milliseconds.  To follow several
    /// indices use [`Self::poll_indices`], which costs one request per tick.
    pub async fn poll_index(
        &self,
        index_name: &str,
        interval_ms: u64,
        tx: mpsc::Sender<NseIndexQuote>,
    ) {
        let label = format!("poll_index {index_name}");
        self.poll_loop(&label, interval_ms, tx, || self.get_index_quote(index_name)).await
    }

    /// Poll several indices with one request per tick; each message holds the
    /// quotes in the order of `index_names`.
    pub async fn poll_indices(
        &self,
        index_names: &[&str],
        interval_ms: u64,
        tx: mpsc::Sender<Vec<NseIndexQuote>>,
    ) {
        let label = format!("poll_indices {index_names:?}");
        self.poll_loop(&label, interval_ms, tx, || self.get_index_quotes(index_names)).await
    }

    async fn poll_loop<T, F, Fut>(&self, label: &str, interval_ms: u64, tx: mpsc::Sender<T>, mut fetch: F)
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let period = Duration::from_millis(interval_ms.max(MIN_POLL_INTERVAL_MS));
        let mut ticker = interval(period);
        // A slow response must not be followed by a burst of catch-up ticks.
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut failures = 0;
        loop {
            tokio::select! {
                _ = ticker.tick() => {}
                _ = tx.closed() => break,
            }
            match fetch().await {
                Ok(v) => {
                    failures = 0;
                    if tx.send(v).await.is_err() { break; }
                }
                Err(e) => {
                    eprintln!("{label} error: {e:#}");
                    let pause = backoff(period, failures, POLL_MAX_BACKOFF);
                    failures += 1;
                    tokio::select! {
                        _ = sleep(pause) => {}
                        _ = tx.closed() => break,
                    }
                    ticker.reset();
                }
            }
        }
    }

    // ── Market status ─────────────────────────────────────────────────────────

    pub async fn get_market_status(&self) -> Result<crate::models::MarketStatusResponse> {
        self.request(|client, c| async move { live::get_market_status(&client, &c).await })
            .await
    }

    // ── Holidays ──────────────────────────────────────────────────────────────

    /// NSE equity ("CM") and F&O ("FO") trading holidays for the current IST year.
    pub async fn get_trading_holidays(&self) -> Result<crate::holidays::TradingHolidays> {
        self.get_trading_holidays_for_year(Some(crate::holidays::current_year_ist())).await
    }

    /// Same as [`Self::get_trading_holidays`] but for a given year (`None` = every year NSE returns).
    pub async fn get_trading_holidays_for_year(
        &self,
        year: Option<i32>,
    ) -> Result<crate::holidays::TradingHolidays> {
        self.request(|client, c| {
            async move { crate::holidays::fetch_trading_holidays(&client, &c, year).await }
        })
        .await
    }

    // ── Archives ─────────────────────────────────────────────────────────────
    // Archive files need no session; they still go through the rate limiter.

    pub async fn fetch_full_bhavcopy(&self, date: NaiveDate) -> Result<Vec<HistoricalRecord>> {
        let _permit = self.limiter.acquire().await;
        archives::fetch_full_bhavcopy(&self.client, date).await
    }

    pub async fn fetch_zipped_bhavcopy(&self, date: NaiveDate) -> Result<Vec<HistoricalRecord>> {
        let _permit = self.limiter.acquire().await;
        archives::fetch_zipped_bhavcopy(&self.client, date).await
    }

    pub async fn fetch_fo_bhavcopy(&self, date: NaiveDate) -> Result<Vec<FoBhavRecord>> {
        let _permit = self.limiter.acquire().await;
        archives::fetch_fo_bhavcopy(&self.client, date).await
    }

    /// All actively trading equity symbols for `date` (falls back to the zipped
    /// bhavcopy when the full one is unavailable).
    pub async fn fetch_symbol_list(&self, date: NaiveDate) -> Result<Vec<String>> {
        let records = match self.fetch_full_bhavcopy(date).await {
            Ok(r)  => r,
            Err(_) => self.fetch_zipped_bhavcopy(date).await?,
        };
        Ok(archives::symbols_from_records(records))
    }
}

impl Default for NseClient {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_http_errors_are_fatal() {
        assert_eq!(classify(&anyhow::anyhow!("no data for index 'FOO'")), Failure::Fatal);
    }
}
