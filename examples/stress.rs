//! Find how fast your IP can query NSE before it starts refusing requests.
//!
//! Sends `get_stock_quote` calls at increasing rates and stops at the first
//! rate that produces errors.  Run it from the machine you will deploy on —
//! NSE (Akamai) treats home, office and datacenter IPs very differently.
//!
//!     cargo run --example stress -- [max_rate_per_sec] [requests_per_step]
//!
//! Defaults: up to 8 req/s, 20 requests per step.  A block can last minutes
//! to hours, so don't run this from an IP you need for live trading.

use std::sync::Arc;
use std::time::{Duration, Instant};
use nse_rs::{NseClient, NseConfig};

const SYMBOLS: &[&str] = &["RELIANCE", "SBIN", "INFY", "TCS", "HDFCBANK"];
const RATES: &[f64] = &[1.0, 2.0, 3.0, 5.0, 8.0, 12.0, 20.0];

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let max_rate: f64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(8.0);
    let per_step: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(20);

    for &rate in RATES.iter().filter(|&&r| r <= max_rate) {
        // Fresh client per step so the step's rate is the only throttle.
        let client = Arc::new(NseClient::with_config(NseConfig {
            requests_per_sec: rate,
            max_concurrent:   16,
            ..NseConfig::default()
        }));
        client.init_session().await?;

        let started = Instant::now();
        let tasks: Vec<_> = (0..per_step)
            .map(|i| {
                let client = Arc::clone(&client);
                tokio::spawn(async move { client.get_stock_quote(SYMBOLS[i % SYMBOLS.len()]).await })
            })
            .collect();
        let mut results = Vec::with_capacity(per_step);
        for task in tasks {
            results.push(task.await?);
        }
        let elapsed = started.elapsed();

        let failed: Vec<_> = results.iter().filter_map(|r| r.as_ref().err()).collect();
        println!(
            "{rate:>5.1} req/s: {}/{} ok in {:.1}s",
            per_step - failed.len(),
            per_step,
            elapsed.as_secs_f64(),
        );
        if let Some(e) = failed.first() {
            println!("        first error: {e:#}");
            println!("Limit reached — stay below {rate} req/s from this IP.");
            return Ok(());
        }
        // Let any rolling window on NSE's side drain before the next step.
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
    println!("No errors up to {max_rate} req/s.");
    Ok(())
}
