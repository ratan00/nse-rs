//! NSE's hosts, in one place. NSE moved `www` and `charting` from
//! nseindia.com to nseindia.in in November 2026; a move like that is a change
//! here only. The client refuses redirects to another host (see
//! [`crate::client`]), so the next one fails with an error naming the new host
//! instead of timing out.

/// A URL on the main site (pages and `/api/...`): `www!("/api/allIndices")`.
macro_rules! www {
    ($path:literal) => {
        concat!("https://www.nseindia.in", $path)
    };
}

/// A URL on the charting site (`/v1/...`).
macro_rules! charting {
    ($path:literal) => {
        concat!("https://charting.nseindia.in", $path)
    };
}

/// A URL on the archives site (bhavcopies, contract lists). Still on .com.
macro_rules! archives {
    ($path:literal) => {
        concat!("https://nsearchives.nseindia.com", $path)
    };
}

pub(crate) use {archives, charting, www};

#[cfg(test)]
mod tests {
    #[test]
    fn urls_are_built_on_the_current_hosts() {
        assert_eq!(www!("/api/marketStatus"), "https://www.nseindia.in/api/marketStatus");
        assert_eq!(charting!("/v1/x"), "https://charting.nseindia.in/v1/x");
        assert_eq!(archives!(""), "https://nsearchives.nseindia.com");
    }
}
