#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = reqwest::Client::new();
    let url = "https://nsearchives.nseindia.com/content/fo/BhavCopy_NSE_FO_0_0_0_20260616_F_0000.csv.zip";
    
    let resp = client.get(url).send().await?;
    println!("Status for modern fo bhavcopy: {}", resp.status());
    
    // Also try the old format for an older date (e.g. 2023)
    let url_old = "https://nsearchives.nseindia.com/content/historical/DERIVATIVES/2023/JUN/fo16JUN2023bhav.csv.zip";
    let resp_old = client.get(url_old).send().await?;
    println!("Status for old fo bhavcopy: {}", resp_old.status());
    
    Ok(())
}
