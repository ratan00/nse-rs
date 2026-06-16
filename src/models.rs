use serde::{Deserialize, Serialize};

/// Struct representing the cache persisted to disk for cookie session caching
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCache {
    pub cookies: std::collections::HashMap<String, String>,
    pub updated_on: chrono::DateTime<chrono::Utc>,
}

// ==========================================
// NextApi Models (getSymbolData)
// ==========================================

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NextApiQuoteResponse {
    #[serde(rename = "equityResponse")]
    pub equity_response: Option<Vec<EquityResponse>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EquityResponse {
    #[serde(rename = "metaData")]
    pub meta_data: Option<MetaData>,
    #[serde(rename = "priceInfo")]
    pub price_info: Option<PriceInfo>,
    #[serde(rename = "tradeInfo")]
    pub trade_info: Option<TradeInfo>,
    #[serde(rename = "secInfo")]
    pub sec_info: Option<SecInfo>,
    #[serde(rename = "lastUpdateTime")]
    pub last_update_time: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MetaData {
    pub symbol: String,
    #[serde(rename = "companyName")]
    pub company_name: Option<String>,
    pub series: Option<String>,
    pub open: Option<f64>,
    #[serde(rename = "dayHigh")]
    pub day_high: Option<f64>,
    #[serde(rename = "dayLow")]
    pub day_low: Option<f64>,
    #[serde(rename = "previousClose")]
    pub previous_close: Option<f64>,
    pub change: Option<f64>,
    #[serde(rename = "closePrice")]
    pub close_price: Option<f64>,
    #[serde(rename = "pChange")]
    pub p_change: Option<f64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PriceInfo {
    #[serde(rename = "yearHigh")]
    pub year_high: Option<f64>,
    #[serde(rename = "yearLow")]
    pub year_low: Option<f64>,
    #[serde(rename = "lowerBand")]
    pub lower_band: Option<f64>,
    #[serde(rename = "upperBand")]
    pub upper_band: Option<f64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TradeInfo {
    #[serde(rename = "totalTradedVolume")]
    pub total_traded_volume: Option<f64>,
    #[serde(rename = "totalTradedValue")]
    pub total_traded_value: Option<f64>,
    #[serde(rename = "lastPrice")]
    pub last_price: Option<f64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SecInfo {
    #[serde(rename = "isinCode")]
    pub isin_code: Option<String>,
    pub industry: Option<String>,
    pub sector: Option<String>,
}

// ==========================================
// NextApi Derivative Models (getSymbolDerivativesData)
// ==========================================

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NextApiDerivativesResponse {
    pub data: Option<Vec<DerivativeContract>>,
    pub timestamp: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DerivativeContract {
    pub identifier: String,
    #[serde(rename = "instrumentType")]
    pub instrument_type: String,
    pub underlying: String,
    #[serde(rename = "expiryDate")]
    pub expiry_date: String,
    #[serde(rename = "optionType")]
    pub option_type: String,
    #[serde(rename = "strikePrice")]
    pub strike_price: serde_json::Value,
    #[serde(rename = "lastPrice")]
    pub last_price: Option<f64>,
    #[serde(rename = "openInterest")]
    pub open_interest: Option<f64>,
    #[serde(rename = "changeinOpenInterest")]
    pub change_in_open_interest: Option<f64>,
    #[serde(rename = "pchangeinOpenInterest")]
    pub p_change_in_open_interest: Option<f64>,
    #[serde(rename = "totalTradedVolume")]
    pub total_traded_volume: Option<f64>,
    pub volume: Option<f64>,
}

// ==========================================
// Charting / Intraday Models (symbolHistoricalData)
// ==========================================

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChartResponse {
    pub data: Option<Vec<ChartCandle>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChartCandle {
    pub time: i64, // millisecond timestamp
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

// ==========================================
// Market Status Models
// ==========================================

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MarketStatusResponse {
    #[serde(rename = "marketState")]
    pub market_state: Vec<MarketState>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MarketState {
    pub market: String,
    #[serde(rename = "marketStatus")]
    pub market_status: String,
    #[serde(rename = "tradeDate")]
    pub trade_date: String,
    #[serde(rename = "marketStatusMessage")]
    pub market_status_message: Option<String>,
}

// ==========================================
// CSV/Bhavcopy Output Record
// ==========================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricalRecord {
    pub date: String,
    pub symbol: String,
    pub series: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub previous_close: f64,
    pub ltp: f64,
    pub volume: u64,
    pub value: f64,
}
