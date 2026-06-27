use nse_rs::NseClient;
use chrono::{Utc, Duration, TimeZone};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = NseClient::new();
    client.init_session().await?;

    let end_time = Utc::now();
    let start_time = end_time - Duration::days(3);

    // Call get_historical_candles on client.
    // Wait, let's write a custom query to get raw charting response to bypass client's filter and see raw candles!
    // Since get_script_token and internal reqwest are private, we can temporarily disable the retain filter in src/historical.rs,
    // or just let client.get_historical_candles run after we disable the filter.
    // Let's print raw candles.

    Ok(())
}
