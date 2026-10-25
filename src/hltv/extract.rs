//! Readers that run *inside the page*. Each is a self-contained JavaScript expression evaluated by
//! the browser, so it closes over nothing. The selectors live only here and in `extract.js`, so an
//! HLTV markup change breaks one file instead of production.

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RawMatch {
    pub href: String,
    pub teams: (String, String),
    pub scores: Option<(u32, u32)>,
    pub event: Option<String>,
    pub heading: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawMatchPage {
    pub title: String,
    pub demo_href: Option<String>,
    pub date: Option<String>,
    pub event: Option<String>,
}

const READERS: &str = include_str!("extract.js");

/// Every match row on a results page, in page order (hltv.org lists newest first).
pub fn read_matches() -> String {
    format!("({READERS}).readMatches()")
}

/// The match page's demo link, if it has one.
pub fn read_match_page() -> String {
    format!("({READERS}).readMatchPage()")
}

/// The selector each page waits for before its reader runs.
pub const RESULTS_READY: &str = ".result-con";
pub const MATCH_READY: &str = ".timeAndEvent, .streams";
