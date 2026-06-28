use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;

// ── Session cache ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCache {
    pub cookies: std::collections::HashMap<String, String>,
    pub updated_on: chrono::DateTime<chrono::Utc>,
}

// ── Normalized live quote ─────────────────────────────────────────────────────

/// Flat, ready-to-use quote extracted from the NSE equity response.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NseQuote {
    pub symbol:         String,
    pub company_name:   String,
    pub series:         String,
    pub ltp:            f64,
    pub open:           f64,
    pub high:           f64,
    pub low:            f64,
    pub prev_close:     f64,
    pub close:          f64,
    pub change:         f64,
    pub change_pct:     f64,
    pub volume:         f64,
    pub traded_value:   f64,
    pub year_high:      f64,
    pub year_low:       f64,
    pub last_update:    String,
}

// ── Normalized index quote ────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NseIndexQuote {
    pub name:       String,
    pub last:       f64,
    pub open:       f64,
    pub high:       f64,
    pub low:        f64,
    pub prev_close: f64,
    pub change:     f64,
    pub change_pct: f64,
}

// ── Raw NextApi types ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NextApiQuoteResponse {
    #[serde(rename = "equityResponse")]
    pub equity_response: Option<Vec<EquityResponse>>,
}

impl NextApiQuoteResponse {
    /// Extract a flat `NseQuote` from the first equity response entry.
    pub fn into_quote(self) -> Option<NseQuote> {
        let eq = self.equity_response?.into_iter().next()?;
        let meta  = eq.meta_data?;
        let price = eq.price_info.unwrap_or_default();
        let trade = eq.trade_info.unwrap_or_default();
        Some(NseQuote {
            symbol:       meta.symbol,
            company_name: meta.company_name.unwrap_or_default(),
            series:       meta.series.unwrap_or_default(),
            ltp:          trade.last_price.unwrap_or(0.0),
            open:         meta.open.unwrap_or(0.0),
            high:         meta.day_high.unwrap_or(0.0),
            low:          meta.day_low.unwrap_or(0.0),
            prev_close:   meta.previous_close.unwrap_or(0.0),
            close:        meta.close_price.unwrap_or(0.0),
            change:       meta.change.unwrap_or(0.0),
            change_pct:   meta.p_change.unwrap_or(0.0),
            volume:       trade.total_traded_volume.unwrap_or(0.0),
            traded_value: trade.total_traded_value.unwrap_or(0.0),
            year_high:    price.year_high.unwrap_or(0.0),
            year_low:     price.year_low.unwrap_or(0.0),
            last_update:  eq.last_update_time.unwrap_or_default(),
        })
    }
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
    #[serde(rename = "companyName")]  pub company_name:  Option<String>,
    pub series:                                          Option<String>,
    pub open:                                            Option<f64>,
    #[serde(rename = "dayHigh")]      pub day_high:      Option<f64>,
    #[serde(rename = "dayLow")]       pub day_low:       Option<f64>,
    #[serde(rename = "previousClose")]pub previous_close:Option<f64>,
    pub change:                                          Option<f64>,
    #[serde(rename = "closePrice")]   pub close_price:   Option<f64>,
    #[serde(rename = "pChange")]      pub p_change:      Option<f64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct PriceInfo {
    #[serde(rename = "yearHigh")]  pub year_high:   Option<f64>,
    #[serde(rename = "yearLow")]   pub year_low:    Option<f64>,
    #[serde(rename = "lowerBand")] pub lower_band:  Option<f64>,
    #[serde(rename = "upperBand")] pub upper_band:  Option<f64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct TradeInfo {
    #[serde(rename = "totalTradedVolume")] pub total_traded_volume: Option<f64>,
    #[serde(rename = "totalTradedValue")]  pub total_traded_value:  Option<f64>,
    #[serde(rename = "lastPrice")]         pub last_price:          Option<f64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SecInfo {
    #[serde(rename = "isinCode")] pub isin_code: Option<String>,
    pub industry:                                 Option<String>,
    pub sector:                                   Option<String>,
}

// ── Raw index API ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct IndexApiResponse {
    pub data: Option<Vec<RawIndexEntry>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawIndexEntry {
    #[serde(rename = "indexSymbol")] pub index_symbol: String,
    #[serde(rename = "last",        default)] pub last:       f64,
    #[serde(rename = "open",        default)] pub open:       f64,
    #[serde(rename = "high",        default)] pub high:       f64,
    #[serde(rename = "low",         default)] pub low:        f64,
    #[serde(rename = "previousClose",default)]pub prev_close: f64,
    #[serde(rename = "variation",    default)] pub change:     f64,
    #[serde(rename = "percentChange",default)]pub change_pct: f64,
}

// ── Derivatives / option chain ────────────────────────────────────────────────

fn deserialize_strike<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    let v: serde_json::Value = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0),
        serde_json::Value::String(s) => s.trim().parse().unwrap_or(0.0),
        _ => 0.0,
    })
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NextApiDerivativesResponse {
    pub data:      Option<Vec<DerivativeContract>>,
    pub timestamp: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DerivativeContract {
    pub identifier:                                         String,
    #[serde(rename = "instrumentType")]  pub instrument_type:        String,
    pub underlying:                                         String,
    #[serde(rename = "expiryDate")]      pub expiry_date:            String,
    #[serde(rename = "optionType")]      pub option_type:            String,
    #[serde(rename = "strikePrice", deserialize_with = "deserialize_strike")]
    pub strike_price:                                       f64,
    #[serde(rename = "lastPrice")]              pub last_price:              Option<f64>,
    #[serde(rename = "openPrice")]              pub open_price:              Option<f64>,
    #[serde(rename = "highPrice")]              pub high_price:              Option<f64>,
    #[serde(rename = "lowPrice")]               pub low_price:               Option<f64>,
    #[serde(rename = "closePrice")]             pub close_price:             Option<f64>,
    #[serde(rename = "prevClose")]              pub prev_close:              Option<f64>,
    #[serde(rename = "openInterest")]           pub open_interest:           Option<f64>,
    #[serde(rename = "changeinOpenInterest")]   pub change_in_open_interest: Option<f64>,
    #[serde(rename = "pchangeinOpenInterest")]  pub p_change_in_open_interest: Option<f64>,
    #[serde(rename = "totalTradedVolume")]      pub total_traded_volume:     Option<f64>,
    pub volume:                                             Option<f64>,
}

// ── Structured option chain ───────────────────────────────────────────────────

/// One side (CE or PE) at a given strike.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OptionSide {
    pub ltp:           f64,
    pub oi:            f64,
    pub change_in_oi:  f64,
    pub volume:        f64,
}

/// One row in the option chain (one strike, both sides).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionChainRow {
    pub strike: f64,
    pub ce:     OptionSide,
    pub pe:     OptionSide,
}

/// Option chain for a given symbol, grouped by expiry date string.
/// `rows` within each expiry are sorted by strike ascending.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionChain {
    pub symbol:  String,
    /// Maps expiry date string (as returned by NSE, e.g. "27-Jun-2024") → sorted rows.
    pub expiries: BTreeMap<String, Vec<OptionChainRow>>,
}

impl OptionChain {
    pub fn from_contracts(symbol: &str, contracts: Vec<DerivativeContract>) -> Self {
        let mut map: BTreeMap<String, BTreeMap<i64, OptionChainRow>> = BTreeMap::new();

        for c in contracts {
            if !c.instrument_type.starts_with("OPT") { continue; }
            let strike_key = (c.strike_price * 100.0) as i64;
            let row = map
                .entry(c.expiry_date.clone())
                .or_default()
                .entry(strike_key)
                .or_insert_with(|| OptionChainRow {
                    strike: c.strike_price,
                    ce:     OptionSide::default(),
                    pe:     OptionSide::default(),
                });

            let side = if c.option_type.eq_ignore_ascii_case("CE") {
                &mut row.ce
            } else {
                &mut row.pe
            };
            side.ltp          = c.last_price.unwrap_or(0.0);
            side.oi           = c.open_interest.unwrap_or(0.0);
            side.change_in_oi = c.change_in_open_interest.unwrap_or(0.0);
            side.volume       = c.total_traded_volume.or(c.volume).unwrap_or(0.0);
        }

        let expiries = map
            .into_iter()
            .map(|(exp, strikes)| {
                let rows = strikes.into_values().collect();
                (exp, rows)
            })
            .collect();

        OptionChain { symbol: symbol.to_string(), expiries }
    }
}

// ── Charting ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChartResponse {
    pub data: Option<Vec<ChartCandle>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChartCandle {
    pub time:   i64,  // milliseconds UTC
    pub open:   f64,
    pub high:   f64,
    pub low:    f64,
    pub close:  f64,
    pub volume: f64,
}

// ── Market status ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MarketStatusResponse {
    #[serde(rename = "marketState")]
    pub market_state: Vec<MarketState>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MarketState {
    pub market:                                             String,
    #[serde(rename = "marketStatus")]        pub market_status:         String,
    #[serde(rename = "tradeDate")]           pub trade_date:            String,
    #[serde(rename = "marketStatusMessage")] pub market_status_message: Option<String>,
}

// ── EOD bhavcopy ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricalRecord {
    pub date:           String,
    pub symbol:         String,
    pub series:         String,
    pub open:           f64,
    pub high:           f64,
    pub low:            f64,
    pub close:          f64,
    pub previous_close: f64,
    pub ltp:            f64,
    pub volume:         u64,
    pub value:          f64,
}

// ── F&O bhavcopy ─────────────────────────────────────────────────────────────

/// One row from the NSE F&O bhavcopy (EOD derivatives data).
/// Covers both the pre-July-2024 legacy format and the post-July-2024 UDiFF format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FoBhavRecord {
    pub symbol:          String,
    pub expiry:          String,
    pub instrument_type: String, // "FUTSTK", "FUTIDX", "OPTSTK", "OPTIDX"
    pub option_type:     String, // "CE", "PE", or "-" for futures
    pub strike:          f64,
    pub open:            f64,
    pub high:            f64,
    pub low:             f64,
    pub close:           f64,
    pub settle_price:    f64,
    pub contracts:       u64,
    pub oi:              u64,
    pub change_in_oi:    i64,
}
