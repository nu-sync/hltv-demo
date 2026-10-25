//! Every failure the tool can produce, with the remediation printed by the CLI. Each carries the
//! values a user needs to act, so no message has to be reconstructed at the call site.

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HltvError {
    /// Not an hltv.org URL. Nothing was opened.
    #[error("{url} is not an hltv.org URL")]
    NotHltvUrl { url: String },
    /// hltv.org, but not a page this tool reads.
    #[error("{url} is not a match, results, or demo URL")]
    UnsupportedPage { url: String },
    /// A map filter that cannot be a map name; refused before a browser opens.
    #[error("{map} is not a map name")]
    NotAMap { map: String },
    /// Cloudflare answered with its challenge instead of the page. Only a person can clear it.
    #[error("{url} is behind the Cloudflare challenge")]
    Challenge { url: String, profile: PathBuf },
    /// The match page loaded but offers no demo.
    #[error("{url} has no demo download link")]
    NoDemoFound { url: String },
    /// A results page listed nothing for the filter.
    #[error("{url} lists no matches")]
    NoMatchesFound { url: String },
    /// A download was canceled, incomplete, or stalled.
    #[error("{url} did not download{}: {reason}", bytes.map(|b| format!(" after {} MB", (b as f64 / 1e6).round())).unwrap_or_default())]
    DownloadFailed {
        url: String,
        reason: String,
        bytes: Option<u64>,
    },
    /// No Chrome to drive.
    #[error("no Chrome found at {}", tried.display())]
    BrowserMissing { tried: PathBuf },
    /// A browser operation failed: a tab would not open, the page threw, or it opened another page.
    #[error("browser operation failed: {reason}")]
    BrowserFailed { reason: String },
    /// hltv.org did not answer: no network, no DNS, or a refused connection. Nothing was read.
    #[error("{url} could not be reached")]
    Unreachable { url: String },
    /// Another process is driving the shared Chrome profile.
    #[error("the Chrome profile {} is in use{}", profile.display(), pid.map(|p| format!(" by pid {p}")).unwrap_or_default())]
    ProfileBusy { profile: PathBuf, pid: Option<u32> },
    /// The download manifest could not be read.
    #[error("cannot read the download manifest {}: {reason}", path.display())]
    ManifestUnreadable { path: PathBuf, reason: String },
}

impl HltvError {
    pub fn remediation(&self) -> String {
        match self {
            Self::NotHltvUrl { .. } => {
                "pass https://www.hltv.org/... or a bare path such as /download/demo/113159".into()
            }
            Self::UnsupportedPage { .. } => {
                "accepted: /matches/<id>/..., /results?..., /download/demo/<id>".into()
            }
            Self::NotAMap { .. } => "pass a map such as cache or de_cache (letters, digits, _)".into(),
            Self::Challenge { profile, .. } => [
                "answer it once in the fetch profile, then run the same command again:".to_string(),
                format!(
                    "  open -a \"Google Chrome\" --args --user-data-dir={} https://www.hltv.org",
                    profile.display()
                ),
                "headless Chrome cannot pass it; do not add a bypass".to_string(),
            ]
            .join("\n"),
            Self::NoDemoFound { .. } => {
                "the match may not publish a demo (online or early-stage), or HLTV changed its markup".into()
            }
            Self::NoMatchesFound { .. } => "check the team id and map name, or drop the filter".into(),
            Self::DownloadFailed { .. } => {
                "retry; nothing partial is recorded, so a retry is always safe".into()
            }
            Self::BrowserMissing { .. } => "install Google Chrome, or set CHROME=/path/to/chrome".into(),
            Self::Unreachable { .. } => "check the network connection, then retry".into(),
            Self::BrowserFailed { .. } => {
                "close the Chrome window the tool opened and retry; a killed run can leave the profile held".into()
            }
            Self::ProfileBusy { .. } => {
                "wait for the other run to finish, or close the Chrome window using that profile; \
                 HLTV_DEMO_PROFILE=/another/path isolates a run"
                    .into()
            }
            Self::ManifestUnreadable { .. } => {
                "delete the manifest to start a fresh one; archives already on disk are not re-downloaded by name".into()
            }
        }
    }

    /// The message, then its remediation indented beneath it, for humans.
    pub fn explain(&self) -> String {
        format!("{self}\n  {}", self.remediation().replace('\n', "\n  "))
    }

    /// Exit 2 for what the user must fix in their environment (no Chrome, a held profile, no
    /// network); 1 for everything else.
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::BrowserMissing { .. } | Self::ProfileBusy { .. } | Self::Unreachable { .. } => 2,
            _ => 1,
        }
    }
}
