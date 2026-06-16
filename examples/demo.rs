use chrono::{Utc, NaiveDate, Duration};
use nse_rs::NseClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("Initializing NseClient...");
    let client = NseClient::new();
    
    println!("Initializing session (loading cache or fetching new cookies)...");
    client.init_session().await?;
    println!("Session initialized successfully!");
    
    // 1. Fetch live stock quote for SBIN
    println!("\n--- Testing Stock Quote (SBIN) ---");
    match client.get_stock_quote("SBIN").await {
        Ok(quote) => {
            if let Some(resp) = quote.equity_response {
                if let Some(first) = resp.first() {
                    let symbol = first.meta_data.as_ref().map(|m| m.symbol.as_str()).unwrap_or("Unknown");
                    let ltp = first.trade_info.as_ref().and_then(|t| t.last_price).unwrap_or(0.0);
                    let name = first.meta_data.as_ref().and_then(|m| m.company_name.as_deref()).unwrap_or("");
                    println!("Stock Symbol: {}", symbol);
                    println!("Company Name: {}", name);
                    println!("Last Traded Price: {}", ltp);
                }
            }
        }
        Err(e) => println!("Error fetching stock quote: {}", e),
    }
    
    // 2. Fetch live derivative contracts for NIFTY
    println!("\n--- Testing Derivatives Quote (NIFTY) ---");
    match client.get_derivatives_quote("NIFTY").await {
        Ok(deriv) => {
            if let Some(data) = deriv.data {
                println!("Retrieved {} derivative contracts for NIFTY.", data.len());
                if let Some(contract) = data.first() {
                    println!("First Contract: {}", contract.identifier);
                    println!("Instrument Type: {}", contract.instrument_type);
                    println!("Expiry Date: {}", contract.expiry_date);
                    println!("Last Price: {}", contract.last_price.unwrap_or(0.0));
                    println!("Open Interest: {}", contract.open_interest.unwrap_or(0.0));
                }
            }
        }
        Err(e) => println!("Error fetching derivatives quote: {}", e),
    }

    // 3. Fetch historical charting data (intraday 5m candles)
    println!("\n--- Testing Intraday Charting Candles (SBIN) ---");
    let end_time = Utc::now();
    let start_time = end_time - Duration::days(5);
    match client.get_historical_candles("SBIN", start_time, end_time, "5").await {
        Ok(candles) => {
            println!("Retrieved {} intraday 5-minute candles.", candles.len());
            if !candles.is_empty() {
                println!("First candle: {:?}", candles.first().unwrap());
                println!("Last candle: {:?}", candles.last().unwrap());
            }
        }
        Err(e) => println!("Error fetching charting candles: {}", e),
    }

    // 4. Fetch full Bhavcopy (EOD summary with delivery data)
    println!("\n--- Testing Full Bhavcopy (EOD Summary) ---");
    // Let's query a known trading date: June 15, 2026 (Monday)
    let test_date = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
    match client.fetch_full_bhavcopy(test_date).await {
        Ok(records) => {
            println!("Downloaded full Bhavcopy for 2026-06-15: {} records.", records.len());
            if let Some(rec) = records.iter().find(|r| r.symbol == "SBIN") {
                println!("SBIN Bhavcopy Record: {:?}", rec);
            }
        }
        Err(e) => println!("Error fetching full Bhavcopy: {}", e),
    }

    // 5. Fetch zipped Bhavcopy
    println!("\n--- Testing Zipped Bhavcopy (EOD Summary) ---");
    // Let's query a historical date before July 8, 2024: June 14, 2024 (Friday)
    let archive_date = NaiveDate::from_ymd_opt(2024, 6, 14).unwrap();
    match client.fetch_zipped_bhavcopy(archive_date).await {
        Ok(records) => {
            println!("Downloaded and parsed zipped Bhavcopy for 2024-06-14: {} records.", records.len());
            if let Some(rec) = records.iter().find(|r| r.symbol == "SBIN") {
                println!("SBIN Zipped Bhavcopy Record: {:?}", rec);
            }
        }
        Err(e) => println!("Error fetching zipped Bhavcopy: {}", e),
    }

    Ok(())
}
