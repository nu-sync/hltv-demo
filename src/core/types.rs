//! Values that cross module boundaries. Each shape is defined once: the library returns it and the
//! MCP server publishes its derived JSON Schema, so the two cannot drift. No behavior here.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Match {
    pub match_id: u64,
    /// Absolute hltv.org URL.
    pub url: String,
    /// ISO day (YYYY-MM-DD) when the results list said so; null when unknown.
    pub date: Option<String>,
    pub event: Option<String>,
    /// [team1, team2] as HLTV displays them.
    pub teams: (String, String),
    /// [team1, team2] maps won, or null when the list shows no score.
    pub scores: Option<(u32, u32)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DemoRef {
    pub match_id: u64,
    pub demo_id: u64,
    /// Absolute `/download/demo/<id>` URL.
    pub url: String,
    pub match_url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Archive {
    pub path: String,
    pub bytes: u64,
    pub demo_url: String,
    /// HLTV's own file name, e.g. `pgl-masters-bucharest-2026-vitality-vs-9z-bo3-eP_cF4LqZBl7PERj7oPEob.rar`
    pub suggested_name: String,
}

/// One line of the download manifest: the demo URL and the file it produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ManifestEntry {
    pub demo_url: String,
    pub file_name: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SearchFilter {
    /// HLTV team id, as in `hltv.org/team/<id>/...`
    pub team: Option<u64>,
    /// Map name, e.g. de_cache or cache (normalized to de_).
    pub map: Option<String>,
    /// Max matches to return: default 1, at most one results page (MAX_LIMIT).
    pub limit: Option<usize>,
    /// Results to skip, for paging (100 per page on hltv.org).
    pub offset: Option<u64>,
}
