use chrono::{Local, Datelike, Duration};
use nse_rs::NseClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = NseClient::new();
    
    // Note: Archives/bhavcopy endpoints do not require session cookie initialization.
    // Try dates going backwards to find the latest available bhavcopy
    let current_date = Local::now().date_naive();
    let mut symbols = Vec::new();
    
    for i in 0..10 {
        let date = current_date - Duration::days(i);
        // Skip weekends
        if date.weekday() == chrono::Weekday::Sat || date.weekday() == chrono::Weekday::Sun {
            continue;
        }
        
        println!("Trying to fetch symbol list for {}", date);
        match client.fetch_symbol_list(date).await {
            Ok(s) => {
                symbols = s;
                println!("Successfully fetched symbols for {}", date);
                break;
            }
            Err(e) => {
                println!("Failed for {}: {}", date, e);
            }
        }
    }
    
    if symbols.is_empty() {
        println!("Could not fetch symbol list for recent dates.");
        return Ok(());
    }
    
    println!("Found {} symbols.", symbols.len());
    println!("First 20 symbols:");
    for (i, symbol) in symbols.iter().take(20).enumerate() {
        println!("{}. {}", i + 1, symbol);
    }
    
    Ok(())
}
