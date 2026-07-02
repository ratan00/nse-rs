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
            println!("Stock Symbol: {}", quote.symbol);
            println!("Company Name: {}", quote.company_name);
            println!("Last Traded Price: {}", quote.ltp);
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

    // 2b. Fetch only Futures contracts
    println!("\n--- Testing Futures Quote (NIFTY) ---");
    match client.get_futures("NIFTY").await {
        Ok(futures) => {
            println!("Retrieved {} futures contracts for NIFTY.", futures.len());
            if let Some(contract) = futures.first() {
                println!("First Futures Contract: {}", contract.identifier);
                println!("Expiry Date: {}", contract.expiry_date);
                println!("Last Price: {}", contract.last_price.unwrap_or(0.0));
            }
        }
        Err(e) => println!("Error fetching futures: {}", e),
    }

    // 2c. Fetch Option Chain contracts
    println!("\n--- Testing Option Chain Quote (NIFTY) ---");
    match client.get_option_chain("NIFTY").await {
        Ok(options) => {
            println!("Retrieved option chain for NIFTY. Expiry dates count: {}", options.expiries.len());
            if let Some((expiry, rows)) = options.expiries.iter().next() {
                println!("First Expiry: {}", expiry);
                if let Some(row) = rows.first() {
                    println!("First Strike: {}", row.strike);
                    println!("CE LTP: {}", row.ce.ltp);
                    println!("PE LTP: {}", row.pe.ltp);
                }
            }
        }
        Err(e) => println!("Error fetching option chain: {}", e),
    }

    // 3. Fetch historical charting data (Daily EOD candles)
    println!("\n--- Testing Daily Charting Candles (NIFTY) ---");
    let end_time = Utc::now();
    let start_time = end_time - Duration::days(45);
    match client.get_historical_candles("NIFTY", start_time, end_time, "D").await {
        Ok(candles) => {
            println!("Retrieved {} daily candles.", candles.len());
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

    // 6. Fetch market status
    println!("\n--- Testing Market Status ---");
    match client.get_market_status().await {
        Ok(status) => {
            println!("Retrieved Market Status!");
            for state in status.market_state.iter().take(2) {
                println!("{}: {}", state.market, state.market_status);
            }
        }
        Err(e) => println!("Error fetching market status: {}", e),
    }

    // 7. Fetch F&O Bhavcopy
    println!("\n--- Testing F&O Bhavcopy ---");
    let fo_date = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
    match client.fetch_fo_bhavcopy(fo_date).await {
        Ok(records) => {
            println!("Downloaded F&O Bhavcopy for 2026-06-15: {} records.", records.len());
            if let Some(rec) = records.first() {
                println!("First F&O Record: {:?}", rec);
            }
        }
        Err(e) => println!("Error fetching F&O Bhavcopy: {}", e),
    }

    Ok(())
}
