<p align="center">
  <img src="assets/banner.png" alt="nse-rs banner" width="90%" />
</p>

<p align="center">
  <a href="https://github.com/ratan00/nse-rs"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License" /></a>
  <img src="https://img.shields.io/badge/Rust-1.75%2B-orange.svg" alt="Rust Version" />
  <img src="https://img.shields.io/badge/PRs-welcome-brightgreen.svg" alt="PRs Welcome" />
</p>

# nse-rs

`nse-rs` is a lightweight, asynchronous Rust library for retrieving live market data directly from the National Stock Exchange of India (NSE). 

**No API keys, no registrations, and no signup fees required.** The library automatically establishes and caches session cookies to communicate directly with the exchange's endpoints.

---

## ⚡ Quickstart

Add the dependency to your `Cargo.toml`:
```toml
[dependencies]
nse-rs = { git = "https://github.com/ratan00/nse-rs.git" }
tokio  = { version = "1", features = ["full"] }
chrono = "0.4"
```

Initialize the client and start querying:
```rust
use nse_rs::NseClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Create a client and initialize connection cookies
    let client = NseClient::new();
    client.init_session().await?;
    
    // Ready to fetch data!
    Ok(())
}
```

---

## 📋 Common Features, Code & Output Examples

### 1. Live Stock Quote (Equities)
Retrieve real-time price, session highs/lows, and daily volumes for any stock.

#### Code:
```rust
let quote = client.get_stock_quote("SBIN").await?;
println!("{:#?}", quote);
```

#### Output Struct:
```rust
NseQuote {
    symbol: "SBIN",
    company_name: "State Bank of India",
    ltp: 1045.2,
    open: 1034.0,
    high: 1051.7,
    low: 1032.1,
    close: 1045.2,
    prev_close: 1033.4,
    change: 11.8,
    change_pct: 1.14,
    volume: 12463437.0,
    traded_value: 127841.9,
    year_high: 1051.7,
    year_low: 835.1,
    last_update: "2026-06-26 15:30:00",
}
```

---

### 2. Live Index Spot Price
Fetch real-time index levels for benchmarking indices like NIFTY 50 or NIFTY BANK.

#### Code:
```rust
let index = client.get_index_quote("Nifty 50").await?;
println!("{:#?}", index);
```

#### Output Struct:
```rust
NseIndexQuote {
    name: "NIFTY 50",
    last: 24052.95,
    open: 24049.65,
    high: 24058.3,
    low: 24039.0,
    prev_close: 24076.3,
    change: -23.35,
    change_pct: -0.10,
}
```

---

### 3. Historical Charting Candles (Intraday or Daily)
Fetch historical price bars (OHLCV). Intraday candles automatically exclude off-market periods, and volume data is normalized from cumulative sums into per-bar values.

#### Code:
```rust
use chrono::{Utc, Duration};

let end = Utc::now();
let start = end - Duration::days(5);

// Fetch 5-minute interval candles for "SBIN"
let candles = client.get_historical_candles("SBIN", start, end, "5").await?;
println!("Retrieved {} candles. First candle:\n{:#?}", candles.len(), candles[0]);
```

#### Output Struct:
```rust
ChartCandle {
    time: 1782206392000, // Unix epoch milliseconds (IST-shifted)
    open: 1038.6,
    high: 1043.4,
    low: 1037.8,
    close: 1041.95,
    volume: 224744.0,    // Normalized per-bar volume
}
```
*Note: Supported intervals include `"1"`, `"3"`, `"5"`, `"15"`, `"30"`, `"60"` (minutes) or `"D"`, `"W"`, `"M"` (Daily, Weekly, Monthly).*

---

### 4. Live Option Chain
Fetch the full option chain matrix sorted by expiries and strikes.

#### Code:
```rust
let chain = client.get_option_chain("NIFTY").await?;
// Print the CE and PE prices for the first strike in the first expiry
if let Some((expiry, rows)) = chain.expiries.iter().next() {
    println!("Expiry Date: {}", expiry);
    println!("{:#?}", rows[0]);
}
```

#### Output Struct:
```rust
OptionChainRow {
    strike: 22300.0,
    ce: OptionSide {
        ltp: 0.0,
        oi: 0.0,
        change_in_oi: 0.0,
        volume: 0.0,
    },
    pe: OptionSide {
        ltp: 32.0,
        oi: 1540.0,
        change_in_oi: 20.0,
        volume: 120.0,
    },
}
```

---

## 🛠️ Advanced Features

### Live Polling Streams
Establish a polling channel to receive quotes at regular intervals, simulating a live data socket stream.

```rust
use tokio::sync::mpsc;

let (tx, mut rx) = mpsc::channel(64);

// Start polling INFY every 3 seconds in the background
tokio::spawn(async move {
    client.poll_quote("INFY", 3000, tx).await;
});

// Receive live stream packets
while let Some(quote) = rx.recv().await {
    println!("Live INFY Price: ₹{:.2}", quote.ltp);
}
```

---

## ⚠️ Important Deployment Notes

The NSE web servers utilize a firewall to protect their servers:
* **IP Geo-blocking**: Requests originating from servers outside of India are typically blocked (returning `403 Forbidden`).
* **Datacenter Blocks**: Datacenter IP addresses (e.g. AWS, DigitalOcean, Google Cloud, Azure) are blacklisted even if they reside in India.
* **Recommendation**: Run your code on a residential Indian internet connection or route requests through a residential Indian proxy server.

---

## 📜 License

This project is licensed under the MIT License.
