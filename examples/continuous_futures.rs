//! Continuous front-month NIFTY futures, stitched across expiries.
//!
//!     cargo run --example continuous_futures -- [SYMBOL] [FROM yyyy-mm-dd] [TO yyyy-mm-dd]

use chrono::{Duration, NaiveDate, Utc};
use nse_rs::{Adjustment, ContinuousOptions, NseClient, RollRule};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let symbol = args.next().unwrap_or_else(|| "NIFTY".into());
    let today = Utc::now().date_naive();
    let date = |s: Option<String>| s.and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok());
    let from = date(args.next()).unwrap_or(today - Duration::days(365));
    let to = date(args.next()).unwrap_or(today);

    let client = NseClient::new();
    client.init_session().await?;

    let series = client
        .get_continuous_futures(&symbol, from, to, ContinuousOptions {
            roll:       RollRule::OpenInterest,
            adjustment: Adjustment::Ratio,
        })
        .await?;

    println!("{symbol}: {} daily bars, {} rolls", series.bars.len(), series.rolls.len());
    for r in &series.rolls {
        println!(
            "  roll {}  {} → {}  gap {:+.2} ({:+.2}%)",
            r.date, r.from, r.to,
            r.to_close - r.from_close,
            (r.to_close / r.from_close - 1.0) * 100.0,
        );
    }
    for b in series.bars.iter().rev().take(5).rev() {
        println!("  {}  close {:.2} (raw {:.2})  vol {}  oi {}  contract {}",
            b.date, b.close, b.raw_close, b.volume, b.oi, b.contract);
    }
    Ok(())
}
