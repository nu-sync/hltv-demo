//! What one look at a page means. The browser loop probes the page and asks these state machines
//! whether to keep waiting, read, or fail; they hold no browser, so every timing rule is tested
//! without Chrome.

use std::time::Duration;

use crate::core::errors::HltvError;
use crate::core::url::page_key;

/// How long a read may take from navigation to reader.
pub const PAGE_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a challenge may stand before it is reported. Cloudflare's managed challenge often clears
/// itself in a headed browser, and a person watching the window can answer it in this time.
pub const CHALLENGE_GRACE: Duration = Duration::from_secs(30);
/// How long a fully loaded page may go without the selector before the reader runs anyway. HLTV
/// renders on the server, so an empty results page is complete when it loads.
pub const SETTLE: Duration = Duration::from_secs(5);
/// How long a demo URL may take to turn into a download.
pub const DOWNLOAD_START_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// One observation of a page.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct Probe {
    pub href: String,
    pub title: String,
    /// The selector the read waits for is present.
    pub ready: bool,
    /// `document.readyState` is "complete".
    pub loaded: bool,
}

const BLANK: &str = "about:blank";
/// The document Chrome commits when a navigation fails at the network: DNS, refused, offline.
const NETWORK_ERROR: &str = "chrome-error://";

pub fn is_challenge_title(title: &str) -> bool {
    let title = title.to_ascii_lowercase();
    [
        "just a moment",
        "security verification",
        "attention required",
    ]
    .iter()
    .any(|t| title.contains(t))
}

fn challenge(url: &str, profile: &std::path::Path) -> HltvError {
    HltvError::Challenge {
        url: url.to_string(),
        profile: profile.to_path_buf(),
    }
}

fn unreachable(url: &str) -> HltvError {
    HltvError::Unreachable {
        url: url.to_string(),
    }
}

fn titled(title: &str) -> &str {
    if title.is_empty() { "unknown" } else { title }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    Wait,
    Read,
    Fail(HltvError),
}

/// A read: navigate, then require arrival (same page identity) and the selector.
pub struct Arrival {
    url: String,
    wanted: String,
    profile: std::path::PathBuf,
    arrived: bool,
    challenged_since: Option<Duration>,
    loaded_since: Option<Duration>,
    elsewhere_since: Option<Duration>,
    last: Option<Probe>,
}

impl Arrival {
    pub fn new(url: &str, profile: &std::path::Path) -> Self {
        Self {
            url: url.to_string(),
            wanted: page_key(url),
            profile: profile.to_path_buf(),
            arrived: false,
            challenged_since: None,
            loaded_since: None,
            elsewhere_since: None,
            last: None,
        }
    }

    /// `seen` is None when the page could not be asked (between documents); `at` is the time since
    /// the navigation started.
    pub fn observe(&mut self, seen: Option<&Probe>, at: Duration) -> Verdict {
        if let Some(page) = seen {
            if page.href.starts_with(NETWORK_ERROR) {
                return Verdict::Fail(unreachable(&self.url));
            }
            if is_challenge_title(&page.title) {
                // The challenge stands in for the page, at its URL or at Cloudflare's rewrite of it.
                self.loaded_since = None;
                self.elsewhere_since = None;
                let since = *self.challenged_since.get_or_insert(at);
                if at - since >= CHALLENGE_GRACE {
                    return Verdict::Fail(challenge(&self.url, &self.profile));
                }
            } else {
                self.challenged_since = None;
                self.arrived |= page_key(&page.href) == self.wanted;
                if self.arrived {
                    self.elsewhere_since = None;
                    if page.ready {
                        return Verdict::Read;
                    }
                    if !page.loaded {
                        self.loaded_since = None;
                    } else if at - *self.loaded_since.get_or_insert(at) >= SETTLE {
                        return Verdict::Read;
                    }
                } else if page.href != BLANK && page.loaded {
                    // Loaded somewhere else: a redirect, not a slow page. It gets the same settle time.
                    if at - *self.elsewhere_since.get_or_insert(at) >= SETTLE {
                        return Verdict::Fail(self.elsewhere(page));
                    }
                } else {
                    self.elsewhere_since = None;
                }
            }
            self.last = Some(page.clone());
        }
        if at < PAGE_TIMEOUT {
            Verdict::Wait
        } else if self.challenged_since.is_some() {
            Verdict::Fail(challenge(&self.url, &self.profile))
        } else if self.arrived {
            Verdict::Read
        } else {
            match &self.last {
                Some(page) if page.href != BLANK => Verdict::Fail(self.elsewhere(page)),
                _ => Verdict::Fail(unreachable(&self.url)),
            }
        }
    }

    fn elsewhere(&self, page: &Probe) -> HltvError {
        HltvError::BrowserFailed {
            reason: format!(
                "{} opened {} ({}) instead",
                self.url,
                page.href,
                titled(&page.title)
            ),
        }
    }
}

/// A download: the demo URL must turn into a download before the page settles on a document.
pub struct Departure {
    url: String,
    profile: std::path::PathBuf,
    document_since: Option<Duration>,
    last_title: String,
}

impl Departure {
    pub fn new(url: &str, profile: &std::path::Path) -> Self {
        Self {
            url: url.to_string(),
            profile: profile.to_path_buf(),
            document_since: None,
            last_title: String::new(),
        }
    }

    /// The failure to report, once waiting is over. A navigation that becomes a download never
    /// commits, so the page stays blank; a document means a challenge or an HLTV page instead.
    pub fn observe(&mut self, seen: Option<&Probe>, at: Duration) -> Option<HltvError> {
        if let Some(page) = seen {
            if page.href.starts_with(NETWORK_ERROR) {
                return Some(unreachable(&self.url));
            }
            self.last_title = page.title.clone();
            if page.href == BLANK {
                self.document_since = None;
            } else {
                let since = *self.document_since.get_or_insert(at);
                if at - since >= CHALLENGE_GRACE {
                    return Some(self.failure());
                }
            }
        }
        (at >= DOWNLOAD_START_TIMEOUT).then(|| self.failure())
    }

    fn failure(&self) -> HltvError {
        if is_challenge_title(&self.last_title) {
            challenge(&self.url, &self.profile)
        } else {
            HltvError::DownloadFailed {
                url: self.url.clone(),
                reason: format!("no download started (page: {})", titled(&self.last_title)),
                bytes: None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const URL: &str = "https://www.hltv.org/matches/2399723/vitality-vs-9z";

    fn probe(href: &str, title: &str, ready: bool, loaded: bool) -> Probe {
        Probe {
            href: href.into(),
            title: title.into(),
            ready,
            loaded,
        }
    }

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    fn arrival() -> Arrival {
        Arrival::new(URL, Path::new("/profile"))
    }

    #[test]
    fn a_read_waits_for_arrival_then_the_selector() {
        let mut a = arrival();
        assert_eq!(
            a.observe(Some(&probe("about:blank", "", false, true)), secs(0)),
            Verdict::Wait
        );
        assert_eq!(a.observe(None, secs(1)), Verdict::Wait);
        let page = probe(
            "https://www.hltv.org/matches/2399723/other-slug",
            "Vitality vs. 9z",
            false,
            false,
        );
        assert_eq!(a.observe(Some(&page), secs(2)), Verdict::Wait);
        let ready = Probe {
            ready: true,
            ..page
        };
        assert_eq!(a.observe(Some(&ready), secs(3)), Verdict::Read);
    }

    #[test]
    fn a_loaded_page_without_the_selector_is_read_after_settling() {
        let mut a = arrival();
        let page = probe(URL, "Vitality vs. 9z", false, true);
        assert_eq!(a.observe(Some(&page), secs(1)), Verdict::Wait);
        assert_eq!(a.observe(Some(&page), secs(5)), Verdict::Wait);
        assert_eq!(a.observe(Some(&page), secs(6)), Verdict::Read);
    }

    #[test]
    fn a_challenge_gets_its_grace_and_may_clear() {
        let mut a = arrival();
        let wall = probe(URL, "Just a moment...", false, true);
        assert_eq!(a.observe(Some(&wall), secs(1)), Verdict::Wait);
        assert_eq!(a.observe(Some(&wall), secs(20)), Verdict::Wait);
        let cleared = probe(URL, "Vitality vs. 9z", true, true);
        assert_eq!(a.observe(Some(&cleared), secs(21)), Verdict::Read);

        let mut b = arrival();
        assert_eq!(b.observe(Some(&wall), secs(1)), Verdict::Wait);
        assert!(matches!(
            b.observe(Some(&wall), secs(31)),
            Verdict::Fail(HltvError::Challenge { .. })
        ));
    }

    #[test]
    fn a_challenge_still_standing_at_the_deadline_is_a_challenge() {
        let mut a = arrival();
        let wall = probe(URL, "Just a moment...", false, true);
        assert_eq!(a.observe(Some(&wall), secs(45)), Verdict::Wait);
        assert!(matches!(
            a.observe(Some(&wall), secs(60)),
            Verdict::Fail(HltvError::Challenge { .. })
        ));
    }

    #[test]
    fn a_challenge_at_cloudflares_rewrite_of_the_url_is_still_the_challenge() {
        let mut a = Arrival::new(
            "https://www.hltv.org/results?team=9565",
            Path::new("/profile"),
        );
        let rewritten = probe(
            "https://www.hltv.org/results?team=9565&__cf_chl_rt_tk=x-1",
            "Just a moment...",
            false,
            true,
        );
        assert_eq!(a.observe(Some(&rewritten), secs(1)), Verdict::Wait);
        assert_eq!(
            a.observe(Some(&rewritten), secs(10)),
            Verdict::Wait,
            "not a redirect elsewhere"
        );
        assert!(matches!(
            a.observe(Some(&rewritten), secs(31)),
            Verdict::Fail(HltvError::Challenge { .. })
        ));
    }

    #[test]
    fn a_page_cleared_by_cloudflare_arrives_with_or_without_its_token() {
        let mut a = Arrival::new(
            "https://www.hltv.org/results?team=9565",
            Path::new("/profile"),
        );
        let cleared = probe(
            "https://www.hltv.org/results?team=9565&__cf_chl_tk=x",
            "Counter-Strike Results",
            true,
            true,
        );
        assert_eq!(a.observe(Some(&cleared), secs(4)), Verdict::Read);
    }

    #[test]
    fn the_network_error_page_is_unreachable_at_once() {
        let mut a = arrival();
        let error = probe("chrome-error://chromewebdata/", "www.hltv.org", false, true);
        assert_eq!(
            a.observe(Some(&error), secs(1)),
            Verdict::Fail(HltvError::Unreachable { url: URL.into() })
        );
    }

    #[test]
    fn a_page_that_never_commits_is_unreachable() {
        let mut a = arrival();
        let blank = probe("about:blank", "", false, true);
        assert_eq!(a.observe(Some(&blank), secs(30)), Verdict::Wait);
        assert_eq!(
            a.observe(Some(&blank), secs(60)),
            Verdict::Fail(HltvError::Unreachable { url: URL.into() })
        );
    }

    #[test]
    fn a_redirect_elsewhere_is_reported_as_where_it_went() {
        let mut a = arrival();
        let home = probe(
            "https://www.hltv.org/",
            "CS2 News & Coverage | HLTV.org",
            false,
            true,
        );
        assert_eq!(a.observe(Some(&home), secs(1)), Verdict::Wait);
        let Verdict::Fail(HltvError::BrowserFailed { reason }) = a.observe(Some(&home), secs(6))
        else {
            panic!("expected a BrowserFailed")
        };
        assert!(reason.contains("opened https://www.hltv.org/"), "{reason}");
    }

    #[test]
    fn an_arrived_page_is_read_at_the_deadline_so_the_reader_reports_what_is_missing() {
        let mut a = arrival();
        let slow = probe(URL, "Vitality vs. 9z", false, false);
        assert_eq!(a.observe(Some(&slow), secs(59)), Verdict::Wait);
        assert_eq!(a.observe(Some(&slow), secs(60)), Verdict::Read);
    }

    #[test]
    fn a_download_waits_while_the_page_stays_blank() {
        let mut d = Departure::new("https://www.hltv.org/download/demo/1", Path::new("/p"));
        let blank = probe("about:blank", "", false, true);
        assert_eq!(d.observe(Some(&blank), secs(1)), None);
        assert_eq!(d.observe(Some(&blank), secs(300)), None);
        assert!(matches!(
            d.observe(Some(&blank), DOWNLOAD_START_TIMEOUT),
            Some(HltvError::DownloadFailed { .. })
        ));
    }

    #[test]
    fn a_download_that_lands_on_a_document_fails_after_the_grace() {
        let mut d = Departure::new("https://www.hltv.org/download/demo/1", Path::new("/p"));
        let wall = probe(
            "https://www.hltv.org/download/demo/1",
            "Just a moment...",
            false,
            true,
        );
        assert_eq!(d.observe(Some(&wall), secs(1)), None);
        assert!(matches!(
            d.observe(Some(&wall), secs(31)),
            Some(HltvError::Challenge { .. })
        ));

        let mut e = Departure::new("https://www.hltv.org/download/demo/1", Path::new("/p"));
        let gone = probe(
            "https://www.hltv.org/download/demo/1",
            "404 | HLTV.org",
            false,
            true,
        );
        assert_eq!(e.observe(Some(&gone), secs(1)), None);
        let Some(HltvError::DownloadFailed { reason, .. }) = e.observe(Some(&gone), secs(31))
        else {
            panic!("expected a DownloadFailed")
        };
        assert_eq!(reason, "no download started (page: 404 | HLTV.org)");
    }

    #[test]
    fn a_download_without_network_is_unreachable_at_once() {
        let mut d = Departure::new("https://www.hltv.org/download/demo/1", Path::new("/p"));
        let error = probe("chrome-error://chromewebdata/", "", false, true);
        assert!(matches!(
            d.observe(Some(&error), secs(1)),
            Some(HltvError::Unreachable { .. })
        ));
    }
}
