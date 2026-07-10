use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use chrono::Utc;
use reqwest::header::{HeaderValue, SET_COOKIE};
use reqwest::Client;
use crate::models::SessionCache;

const SESSION_WARMUP_URL: &str = "https://www.nseindia.com/get-quote/equity/RELIANCE/Reliance-Industries-Limited";

/// Get cache file path in standard local cache directory
pub fn get_cache_path() -> Option<PathBuf> {
    let mut base = if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        if !xdg.is_empty() {
            Some(PathBuf::from(xdg))
        } else {
            None
        }
    } else {
        None
    };
    if base.is_none() {
        base = dirs::cache_dir();
    }
    base.map(|mut p| {
        p.push("nse-rs");
        p.push("session.json");
        p
    })
}

/// Load session cache from disk
pub fn load_session_cache() -> Option<SessionCache> {
    let path = get_cache_path()?;
    if !path.exists() {
        return None;
    }
    
    let content = fs::read_to_string(path).ok()?;
    let cache: SessionCache = serde_json::from_str(&content).ok()?;
    
    // Check if session has expired (TTL of 1 hour)
    let elapsed = Utc::now() - cache.updated_on;
    if elapsed < chrono::Duration::hours(1) {
        Some(cache)
    } else {
        None
    }
}

/// Save session cache to disk
pub fn save_session_cache(cookies: &HashMap<String, String>) -> Option<()> {
    let path = get_cache_path()?;
    
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok()?;
    }
    
    let cache = SessionCache {
        cookies: cookies.clone(),
        updated_on: Utc::now(),
    };
    
    let serialized = serde_json::to_string_pretty(&cache).ok()?;
    fs::write(path, serialized).ok()?;
    Some(())
}

/// Extract cookies from SET_COOKIE headers
pub fn extract_cookies(response: &reqwest::Response) -> HashMap<String, String> {
    let mut cookies = HashMap::new();
    for header_val in response.headers().get_all(SET_COOKIE) {
        if let Ok(cookie_str) = header_val.to_str() {
            if let Some(first_part) = cookie_str.split(';').next() {
                let mut parts = first_part.splitn(2, '=');
                if let (Some(name), Some(val)) = (parts.next(), parts.next()) {
                    cookies.insert(name.trim().to_string(), val.trim().to_string());
                }
            }
        }
    }
    cookies
}

/// Formats a HashMap of cookies into a single Cookie header value
pub fn format_cookie_header(cookies: &HashMap<String, String>) -> HeaderValue {
    let cookie_str = cookies.iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("; ");
    HeaderValue::from_str(&cookie_str).unwrap_or_else(|_| HeaderValue::from_static(""))
}

/// Helper to request cookies from the landing page.
/// Visits the NSE India homepage first to establish the cookie jar context.
pub async fn fetch_new_cookies(client: &Client) -> Result<HashMap<String, String>, reqwest::Error> {
    let mut cookies = HashMap::new();
    if let Ok(resp1) = client.get("https://www.nseindia.com/").send().await {
        cookies.extend(extract_cookies(&resp1));
    }
    let resp2 = client.get(SESSION_WARMUP_URL).send().await?;
    cookies.extend(extract_cookies(&resp2));
    Ok(cookies)
}
