use std::io::Read;
use anyhow::{Context, Result, bail};
use reqwest::Client;
use chrono::NaiveDate;
use crate::models::{HistoricalRecord, FoBhavRecord};

const BASE_ARCHIVES_URL: &str = "https://nsearchives.nseindia.com";

/// Fetch full bhavcopy CSV (includes delivery data).
pub async fn fetch_full_bhavcopy(
    client: &Client,
    date: NaiveDate,
) -> Result<Vec<HistoricalRecord>> {
    let url = format!(
        "{}/products/content/sec_bhavdata_full_{}.csv",
        BASE_ARCHIVES_URL,
        date.format("%d%m%Y"),
    );
    let text = get_text(client, &url).await?;
    parse_full_bhavcopy(&text)
}

/// Fetch a list of all actively trading equity symbols for a given date.
pub async fn fetch_symbol_list(client: &Client, date: NaiveDate) -> Result<Vec<String>> {
    let records = match fetch_full_bhavcopy(client, date).await {
        Ok(r)  => r,
        Err(_) => fetch_zipped_bhavcopy(client, date).await?,
    };
    let mut symbols: Vec<String> = records
        .into_iter()
        .filter(|r| matches!(r.series.as_str(), "EQ" | "BE" | "SM" | "MF"))
        .map(|r| r.symbol)
        .collect();
    symbols.sort();
    symbols.dedup();
    Ok(symbols)
}

/// Fetch standard zipped bhavcopy (older format, no delivery data).
pub async fn fetch_zipped_bhavcopy(
    client: &Client,
    date: NaiveDate,
) -> Result<Vec<HistoricalRecord>> {
    let yyyy     = date.format("%Y").to_string();
    let mmm      = date.format("%b").to_string().to_uppercase();
    let date_up  = date.format("%d%b%Y").to_string().to_uppercase();
    let url = format!(
        "{}/content/historical/EQUITIES/{}/{}/cm{}bhav.csv.zip",
        BASE_ARCHIVES_URL, yyyy, mmm, date_up,
    );
    let text = get_zip_text(client, &url).await?;
    parse_zipped_bhavcopy(&text, date)
}

/// Fetch and parse F&O bhavcopy into typed `FoBhavRecord`s.
/// Handles both the pre-July-2024 legacy and the post-July-2024 UDiFF ZIP formats.
pub async fn fetch_fo_bhavcopy(client: &Client, date: NaiveDate) -> Result<Vec<FoBhavRecord>> {
    let transition = NaiveDate::from_ymd_opt(2024, 7, 8).expect("valid date");
    let url = if date >= transition {
        format!(
            "{}/content/fo/BhavCopy_NSE_FO_0_0_0_{}_F_0000.csv.zip",
            BASE_ARCHIVES_URL,
            date.format("%Y%m%d"),
        )
    } else {
        let yyyy    = date.format("%Y").to_string();
        let mmm     = date.format("%b").to_string().to_uppercase();
        let date_up = date.format("%d%b%Y").to_string().to_uppercase();
        format!(
            "{}/content/historical/DERIVATIVES/{}/{}/fo{}bhav.csv.zip",
            BASE_ARCHIVES_URL, yyyy, mmm, date_up,
        )
    };

    let text = get_zip_text(client, &url).await?;
    let transition = NaiveDate::from_ymd_opt(2024, 7, 8).expect("valid date");
    if date >= transition {
        parse_fo_udiff(&text)
    } else {
        parse_fo_legacy(&text)
    }
}

// ── HTTP helpers ──────────────────────────────────────────────────────────────

async fn get_text(client: &Client, url: &str) -> Result<String> {
    let resp = client
        .get(url)
        .header(reqwest::header::ACCEPT, "*/*")
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    if !resp.status().is_success() {
        bail!("HTTP {} for {url}", resp.status());
    }
    resp.text().await.context("read response text")
}

async fn get_zip_text(client: &Client, url: &str) -> Result<String> {
    let resp = client
        .get(url)
        .header(reqwest::header::ACCEPT, "*/*")
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    if !resp.status().is_success() {
        bail!("HTTP {} for {url}", resp.status());
    }
    let bytes  = resp.bytes().await.context("read bytes")?;
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor).context("open zip")?;
    let mut file = archive.by_index(0).context("zip first entry")?;
    let mut text = String::new();
    file.read_to_string(&mut text).context("read zip entry")?;
    Ok(text)
}

// ── Parsers ───────────────────────────────────────────────────────────────────

fn parse_full_bhavcopy(csv: &str) -> Result<Vec<HistoricalRecord>> {
    let mut rdr = csv::ReaderBuilder::new().trim(csv::Trim::All).from_reader(csv.as_bytes());
    let mut out = Vec::new();
    for result in rdr.records() {
        let r = result.context("csv record")?;
        if r.len() < 15 { continue; }
        out.push(HistoricalRecord {
            symbol:         get(&r, 0),
            series:         get(&r, 1),
            date:           get(&r, 2),
            previous_close: pf64(&r, 3),
            open:           pf64(&r, 4),
            high:           pf64(&r, 5),
            low:            pf64(&r, 6),
            ltp:            pf64(&r, 7),
            close:          pf64(&r, 8),
            volume:         pf64(&r, 10) as u64,
            value:          pf64(&r, 11),
        });
    }
    Ok(out)
}

fn parse_zipped_bhavcopy(csv: &str, date: NaiveDate) -> Result<Vec<HistoricalRecord>> {
    let mut rdr = csv::ReaderBuilder::new().trim(csv::Trim::All).from_reader(csv.as_bytes());
    let date_str = date.format("%d-%b-%Y").to_string();
    let mut out = Vec::new();
    for result in rdr.records() {
        let r = result.context("csv record")?;
        if r.len() < 13 { continue; }
        out.push(HistoricalRecord {
            symbol:         get(&r, 0),
            series:         get(&r, 1),
            date:           date_str.clone(),
            open:           pf64(&r, 2),
            high:           pf64(&r, 3),
            low:            pf64(&r, 4),
            close:          pf64(&r, 5),
            ltp:            pf64(&r, 6),
            previous_close: pf64(&r, 7),
            volume:         pf64(&r, 8) as u64,
            value:          pf64(&r, 9),
        });
    }
    Ok(out)
}

/// Post-July-2024 UDiFF format columns:
/// FinInstrmNm, XpryDt, OptnTp, StrkPric, OpnPric, HghPric, LwPric, ClsPric,
/// SttlmPric, TtlTradgVol, OpnIntrst, ChngInOpnIntrst, ...
fn parse_fo_udiff(csv: &str) -> Result<Vec<FoBhavRecord>> {
    let mut rdr = csv::ReaderBuilder::new().trim(csv::Trim::All).from_reader(csv.as_bytes());
    let mut out = Vec::new();
    for result in rdr.records() {
        let r = result.context("csv record")?;
        if r.len() < 12 { continue; }
        let instrument_type = infer_instrument_type(get(&r, 0).as_str(), get(&r, 2).as_str());
        out.push(FoBhavRecord {
            symbol:          get(&r, 0),
            expiry:          get(&r, 1),
            instrument_type,
            option_type:     get(&r, 2),
            strike:          pf64(&r, 3),
            open:            pf64(&r, 4),
            high:            pf64(&r, 5),
            low:             pf64(&r, 6),
            close:           pf64(&r, 7),
            settle_price:    pf64(&r, 8),
            contracts:       pf64(&r, 9) as u64,
            oi:              pf64(&r, 10) as u64,
            change_in_oi:    pf64(&r, 11) as i64,
        });
    }
    Ok(out)
}

/// Legacy format columns:
/// INSTRUMENT,SYMBOL,EXPIRY_DT,STRIKE_PR,OPTION_TYP,OPEN,HIGH,LOW,CLOSE,SETTLE_PR,
/// CONTRACTS,VAL_IN_LAKH,OPEN_INT,CHG_IN_OI,TIMESTAMP
fn parse_fo_legacy(csv: &str) -> Result<Vec<FoBhavRecord>> {
    let mut rdr = csv::ReaderBuilder::new().trim(csv::Trim::All).from_reader(csv.as_bytes());
    let mut out = Vec::new();
    for result in rdr.records() {
        let r = result.context("csv record")?;
        if r.len() < 14 { continue; }
        out.push(FoBhavRecord {
            instrument_type: get(&r, 0),
            symbol:          get(&r, 1),
            expiry:          get(&r, 2),
            strike:          pf64(&r, 3),
            option_type:     get(&r, 4),
            open:            pf64(&r, 5),
            high:            pf64(&r, 6),
            low:             pf64(&r, 7),
            close:           pf64(&r, 8),
            settle_price:    pf64(&r, 9),
            contracts:       pf64(&r, 10) as u64,
            oi:              pf64(&r, 12) as u64,
            change_in_oi:    pf64(&r, 13) as i64,
        });
    }
    Ok(out)
}

fn infer_instrument_type(symbol: &str, option_type: &str) -> String {
    let is_index = matches!(symbol, "NIFTY" | "BANKNIFTY" | "FINNIFTY" | "MIDCPNIFTY" | "SENSEX");
    match option_type {
        "CE" | "PE" => if is_index { "OPTIDX".into() } else { "OPTSTK".into() },
        _            => if is_index { "FUTIDX".into() } else { "FUTSTK".into() },
    }
}

// ── CSV field helpers ─────────────────────────────────────────────────────────

fn get(r: &csv::StringRecord, i: usize) -> String {
    r.get(i).unwrap_or("").trim().to_string()
}

fn pf64(r: &csv::StringRecord, i: usize) -> f64 {
    r.get(i).unwrap_or("0").trim().replace(',', "").parse().unwrap_or(0.0)
}
