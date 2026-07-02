use nse_rs::NseClient;
use chrono::{Utc, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("Initializing NseClient...");
    let client = NseClient::new();
    client.init_session().await?;
    println!("Session initialized.");

    println!("\n--- Test: get_historical_candles(\"Nifty Midcap Select\") ---");
    let end_time = Utc::now();
    let start_time = end_time - Duration::days(5);
    match client.get_historical_candles("Nifty Midcap Select", start_time, end_time, "5").await {
        Ok(candles) => {
            println!("Success! Retrieved {} candles.", candles.len());
            if !candles.is_empty() {
                println!("First: {:?}", candles.first().unwrap());
                println!("Last: {:?}", candles.last().unwrap());
            }
        }
        Err(e) => println!("Failed: {:?}", e),
    }

    Ok(())
}
