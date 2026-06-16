# nse-rs

`nse-rs` is a high-performance, async-first Rust library for retrieving live quotes, option chains, intraday charting candles, and historical Bhavcopy archives from the National Stock Exchange of India (NSE). 

It combines the best aspects of Python's `jugaad-data` and `nsemine` libraries, rewritten in pure Rust for type safety, speed, and low resource overhead.

---

## Features

- **🛡️ Secure Session & Cookie Caching**: Caches cookies (`nsit`, `nseappid`, etc.) locally at `~/.cache/nse-rs/session.json` with an automated 1-hour TTL expiration check. This dramatically reduces landing page requests and protects you from getting rate-limited or blocked.
- **⚡ Zero OpenSSL Compilation Dependency**: Uses `rustls` for TLS out of the box. Highly portable and compiles seamlessly on all Linux and cross-compilation environments.
- **📈 Intraday Charting Candles**: Retrieves live/historical intraday OHLC candles at custom intervals (1m, 3m, 5m, etc.) from `charting.nseindia.com`.
- **🕒 Market Hours Filtering**: Intraday charting results are automatically filtered to exclude pre-market and post-market ticks (keeping only 09:15:00 to 15:30:00 IST session data).
- **📋 Live Quotes & Derivatives**: Accesses active NextApi endpoints (`getSymbolData` and `getSymbolDerivativesData`) to fetch quotes, volumes, bid/ask spreads, and active F&O options chains.
- **📂 Bulk Bhavcopies**: 
  - Downloads zipped historical Bhavcopies (pre-July 2024 dates) and extracts them **in-memory** without requiring disk storage.
  - Downloads modern UDiff full Bhavcopies containing detailed delivery reports.

---

## Architecture

The crate is structured into dedicated, decoupled modules:

```
src/
├── lib.rs          # Crate exports and public modules
├── client.rs       # Unified NseClient struct (main entry point)
├── session.rs      # Session tracking, page landing request, cookie caching
├── live.rs         # Live NextApi quotes (stocks, futures, options)
├── historical.rs   # Intraday charting client and token mapping
├── archives.rs     # Bhavcopy downloader, zip decompressor, CSV parser
└── models.rs       # Serde Deserialize structures for all JSON/CSV records
```

---

## Installation

Add the following to your `Cargo.toml`:

```toml
[dependencies]
nse-rs = { git = "https://github.com/ratan00/nse-rs.git" }
tokio = { version = "1.38", features = ["full"] }
chrono = "0.4"
```

---

## Usage Example

Initialize `NseClient` and fetch quotes, charting, or EOD archives:

```rust
use chrono::{Utc, NaiveDate, Duration};
use nse_rs::NseClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // 1. Initialize Client
    let client = NseClient::new();
    client.init_session().await?;

    // 2. Fetch live stock quotes
    let quote = client.get_stock_quote("SBIN").await?;
    if let Some(resp) = quote.equity_response {
        if let Some(first) = resp.first() {
            println!("SBIN LTP: {}", first.trade_info.as_ref().and_then(|t| t.last_price).unwrap_or(0.0));
        }
    }

    // 3. Fetch live derivative contracts (NIFTY option chain)
    let deriv = client.get_derivatives_quote("NIFTY").await?;
    if let Some(contracts) = deriv.data {
        println!("Retrieved {} contracts", contracts.len());
    }

    // 4. Fetch 5-minute intraday charting candles
    let end_time = Utc::now();
    let start_time = end_time - Duration::days(5);
    let candles = client.get_historical_candles("SBIN", start_time, end_time, "5").await?;
    println!("Retrieved {} intraday candles.", candles.len());

    // 5. Fetch full daily EOD summary with delivery details
    let date = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
    let records = client.fetch_full_bhavcopy(date).await?;
    println!("Full Bhavcopy records: {}", records.len());

    Ok(())
}
```

You can run a complete demo script in the repository:
```bash
cargo run --example demo
```

---

## License

MIT License.
