pub mod models;
pub mod session;
pub mod live;
pub mod historical;
pub mod archives;
pub mod client;
pub mod holidays;
pub mod ratelimit;
pub mod futures;

pub use client::{NseClient, NseConfig, MIN_POLL_INTERVAL_MS};
pub use holidays::{NseHoliday, TradingHolidays};
pub use futures::{
    Adjustment, ContinuousBar, ContinuousFutures, ContinuousOptions, FuturesDailyRecord, RollEvent, RollRule,
};
pub use models::{
    // Session
    SessionCache,
    // Live quotes
    NseQuote, NseIndexQuote,
    // Raw API types (for callers that need the full response)
    NextApiQuoteResponse, NextApiDerivativesResponse, DerivativeContract,
    // Option chain
    OptionChain, OptionChainRow, OptionSide,
    // Historical
    ChartCandle, HistoricalRecord,
    // F&O EOD
    FoBhavRecord,
    // Market status
    MarketStatusResponse,
};
