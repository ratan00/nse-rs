use std::io::Read;
use reqwest::Client;
use chrono::NaiveDate;
use crate::models::HistoricalRecord;

const BASE_ARCHIVES_URL: &str = "https://nsearchives.nseindia.com";

/// Fetch full bhavcopy CSV containing delivery data
/// URL: https://nsearchives.nseindia.com/products/content/sec_bhavdata_full_DDMMYYYY.csv
pub async fn fetch_full_bhavcopy(
    client: &Client,
    date: NaiveDate,
) -> Result<Vec<HistoricalRecord>, Box<dyn std::error::Error + Send + Sync>> {
    let date_str = date.format("%d%m%Y").to_string();
    let url = format!("{}/products/content/sec_bhavdata_full_{}.csv", BASE_ARCHIVES_URL, date_str);
    
    let resp = client.get(&url)
        .header(reqwest::header::ACCEPT, "*/*")
        .send().await?;
    if !resp.status().is_success() {
        return Err(format!("Failed to download full bhavcopy, status: {}", resp.status()).into());
    }
    
    let csv_text = resp.text().await?;
    let records = parse_full_bhavcopy(&csv_text)?;
    Ok(records)
}

/// Fetch a master list of all actively trading symbols for a given date
pub async fn fetch_symbol_list(
    client: &Client,
    date: NaiveDate,
) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let records = match fetch_full_bhavcopy(client, date).await {
        Ok(r) => r,
        Err(_) => fetch_zipped_bhavcopy(client, date).await?,
    };
    
    let mut symbols: Vec<String> = records
        .into_iter()
        .filter(|r| r.series == "EQ" || r.series == "BE" || r.series == "SM" || r.series == "MF")
        .map(|r| r.symbol)
        .collect();
        
    symbols.sort();
    symbols.dedup();
    
    Ok(symbols)
}

/// Fetch standard zipped bhavcopy (no delivery data, but covers older historical dates)
/// URL: https://nsearchives.nseindia.com/content/historical/EQUITIES/YYYY/MMM/cmDDMMM[YYYY]bhav.csv.zip
pub async fn fetch_zipped_bhavcopy(
    client: &Client,
    date: NaiveDate,
) -> Result<Vec<HistoricalRecord>, Box<dyn std::error::Error + Send + Sync>> {
    let yyyy = date.format("%Y").to_string();
    let mmm = date.format("%b").to_string().to_uppercase();
    let date_upper = date.format("%d%b%Y").to_string().to_uppercase();
    
    let url = format!(
        "{}/content/historical/EQUITIES/{}/{}/cm{}bhav.csv.zip",
        BASE_ARCHIVES_URL, yyyy, mmm, date_upper
    );
    
    let resp = client.get(&url)
        .header(reqwest::header::ACCEPT, "*/*")
        .send().await?;
    if !resp.status().is_success() {
        return Err(format!("Failed to download zipped bhavcopy, status: {}", resp.status()).into());
    }
    
    let bytes = resp.bytes().await?;
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor)?;
    
    // Find the first CSV file in the zip
    let mut file = archive.by_index(0)?;
    let mut csv_text = String::new();
    file.read_to_string(&mut csv_text)?;
    
    let records = parse_zipped_bhavcopy(&csv_text, date)?;
    Ok(records)
}

/// Parse full bhavcopy CSV text
fn parse_full_bhavcopy(csv_text: &str) -> Result<Vec<HistoricalRecord>, Box<dyn std::error::Error + Send + Sync>> {
    let mut reader = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .from_reader(csv_text.as_bytes());
        
    let mut records = Vec::new();
    for result in reader.records() {
        let record = result?;
        if record.len() < 15 {
            continue;
        }
        
        let symbol = record.get(0).unwrap_or("").to_string();
        let series = record.get(1).unwrap_or("").to_string();
        let date_str = record.get(2).unwrap_or("").to_string();
        
        let prev_close = record.get(3).unwrap_or("0").parse::<f64>().unwrap_or(0.0);
        let open = record.get(4).unwrap_or("0").parse::<f64>().unwrap_or(0.0);
        let high = record.get(5).unwrap_or("0").parse::<f64>().unwrap_or(0.0);
        let low = record.get(6).unwrap_or("0").parse::<f64>().unwrap_or(0.0);
        let ltp = record.get(7).unwrap_or("0").parse::<f64>().unwrap_or(0.0);
        let close = record.get(8).unwrap_or("0").parse::<f64>().unwrap_or(0.0);
        
        let volume = record.get(10).unwrap_or("0").parse::<f64>().unwrap_or(0.0) as u64;
        let value = record.get(11).unwrap_or("0").parse::<f64>().unwrap_or(0.0);
        
        records.push(HistoricalRecord {
            date: date_str,
            symbol,
            series,
            open,
            high,
            low,
            close,
            previous_close: prev_close,
            ltp,
            volume,
            value,
        });
    }
    Ok(records)
}

/// Parse zipped bhavcopy CSV text (different headers)
fn parse_zipped_bhavcopy(csv_text: &str, date: NaiveDate) -> Result<Vec<HistoricalRecord>, Box<dyn std::error::Error + Send + Sync>> {
    let mut reader = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .from_reader(csv_text.as_bytes());
        
    let date_str = date.format("%d-%b-%Y").to_string();
    let mut records = Vec::new();
    
    for result in reader.records() {
        let record = result?;
        if record.len() < 13 {
            continue;
        }
        
        let symbol = record.get(0).unwrap_or("").to_string();
        let series = record.get(1).unwrap_or("").to_string();
        
        let open = record.get(2).unwrap_or("0").parse::<f64>().unwrap_or(0.0);
        let high = record.get(3).unwrap_or("0").parse::<f64>().unwrap_or(0.0);
        let low = record.get(4).unwrap_or("0").parse::<f64>().unwrap_or(0.0);
        let close = record.get(5).unwrap_or("0").parse::<f64>().unwrap_or(0.0);
        let last = record.get(6).unwrap_or("0").parse::<f64>().unwrap_or(0.0);
        let prev_close = record.get(7).unwrap_or("0").parse::<f64>().unwrap_or(0.0);
        
        let volume = record.get(8).unwrap_or("0").parse::<f64>().unwrap_or(0.0) as u64;
        let value = record.get(9).unwrap_or("0").parse::<f64>().unwrap_or(0.0);
        
        records.push(HistoricalRecord {
            date: date_str.clone(),
            symbol,
            series,
            open,
            high,
            low,
            close,
            previous_close: prev_close,
            ltp: last,
            volume,
            value,
        });
    }
    Ok(records)
}
