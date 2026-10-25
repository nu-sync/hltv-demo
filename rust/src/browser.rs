//! Chrome over CDP. This is the only module that knows chromiumoxide: nothing else opens a browser
//! or reaches the network.
//!
//! Field facts encoded here rather than left to callers (see SPEC.md, Verified facts):
//!  - the browser must be headed with a persistent profile: HLTV challenges headless clients;
//!  - every operation gets a fresh page, so "is the DOM ready" is never answered by the previous page;
//!  - `Page.navigate`'s reply can arrive long after the page loads, and never arrives for a download,
//!    so navigation is fired and arrival is observed, never awaited;
//!  - completion is the browser's download event, never a file appearing on disk.
//!
//! What each observation of a page means lives in `arrival`, which holds no browser.

mod arrival;

use std::path::{Path, PathBuf};
use std::time::Duration;

use chromiumoxide::cdp::browser_protocol::browser::{
    CancelDownloadParams, DownloadProgressState, EventDownloadProgress, EventDownloadWillBegin,
    SetDownloadBehaviorBehavior, SetDownloadBehaviorParams,
};
use chromiumoxide::cdp::browser_protocol::page::NavigateParams;
use chromiumoxide::listeners::EventStream;
use chromiumoxide::{Browser, BrowserConfig, Page};
use futures::StreamExt;
use serde::de::DeserializeOwned;
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep, timeout};
use tracing::debug;

use self::arrival::{Arrival, Departure, Verdict};
pub use self::arrival::{CHALLENGE_GRACE, PAGE_TIMEOUT, Probe, is_challenge_title};
use crate::core::errors::HltvError;
use crate::core::lock::{self, Acquired, Lock};
use crate::hltv::extract::{self, RawMatch, RawMatchPage};
use crate::source::{Opener, Saved, Source};

const DEFAULT_CHROME: &str = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
/// A download whose progress stops for this long is reported as stalled.
const DOWNLOAD_STALL_TIMEOUT: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(250);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

/// The persistent profile every run shares, so a challenge answered once stays answered.
pub fn profile_dir() -> PathBuf {
    let dir = std::env::var_os("HLTV_DEMO_PROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_default()
                .join(".cache")
                .join("hltv-demo")
                .join("chrome")
        });
    std::path::absolute(&dir).unwrap_or(dir)
}

pub fn chrome_path() -> PathBuf {
    std::env::var_os("CHROME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_CHROME))
}

/// The lock that keeps two runs from driving the same profile.
pub fn lock_path() -> PathBuf {
    let mut path = profile_dir().into_os_string();
    path.push(".lock");
    PathBuf::from(path)
}

/// Not a supported mode: headless Chrome is challenged, and the switch exists to make that legible.
pub fn headless() -> bool {
    std::env::var("HLTV_DEMO_HEADLESS").is_ok_and(|v| v == "1")
}

fn failed(cause: impl std::fmt::Display) -> HltvError {
    HltvError::BrowserFailed {
        reason: cause.to_string(),
    }
}

/// Switches every launch shares, as chromiumoxide spells them: without the leading `--`, which it
/// adds. Its defaults are replaced rather than extended because they include `--enable-automation`.
/// The keychain switches are Playwright's too, and they matter: Chrome encrypts the profile's cookies
/// (Cloudflare's clearance among them) with the key they select, so both implementations must agree.
const BASE_SWITCHES: [&str; 3] = [
    "no-first-run",
    "no-default-browser-check",
    "use-mock-keychain",
];
const PASSWORD_STORE: (&str, &str) = ("password-store", "basic");

/// A launched Chrome and the task that drives its CDP connection.
struct Driver {
    browser: Browser,
    handler: JoinHandle<()>,
    profile: PathBuf,
}

impl Driver {
    async fn launch(config: BrowserConfig, profile: PathBuf) -> Result<Self, String> {
        let started = Instant::now();
        let (browser, mut events) = Browser::launch(config).await.map_err(|e| e.to_string())?;
        // An unknown message from a newer Chrome is not fatal; the loop ends when the connection does.
        let handler = tokio::spawn(async move { while events.next().await.is_some() {} });
        debug!(ms = started.elapsed().as_millis() as u64, "chrome launched");
        Ok(Self {
            browser,
            handler,
            profile,
        })
    }

    async fn close(mut self) {
        let started = Instant::now();
        let _ = timeout(CLOSE_TIMEOUT, self.browser.close()).await;
        let _ = timeout(CLOSE_TIMEOUT, self.browser.wait()).await;
        self.handler.abort();
        debug!(ms = started.elapsed().as_millis() as u64, "chrome closed");
    }

    /// A fresh page for one operation, closed however the operation ends.
    async fn with_page<T>(
        &self,
        work: impl AsyncFnOnce(&Page) -> Result<T, HltvError>,
    ) -> Result<T, HltvError> {
        let page = self.browser.new_page("about:blank").await.map_err(failed)?;
        let result = work(&page).await;
        let _ = timeout(CLOSE_TIMEOUT, page.close()).await;
        result
    }

    /// Open `url` in a fresh page, wait until it arrived and `selector` exists, then read it in-page.
    async fn read<T: DeserializeOwned>(
        &self,
        url: &str,
        selector: &str,
        reader: &str,
    ) -> Result<T, HltvError> {
        self.with_page(async |page| {
            navigate(page, url, selector, &self.profile).await?;
            evaluate(page, reader).await
        })
        .await
    }

    /// Load `html` into a fresh page and read it in-page, as `read` would a live page.
    async fn read_html<T: DeserializeOwned>(
        &self,
        html: &str,
        reader: &str,
    ) -> Result<T, HltvError> {
        self.with_page(async |page| {
            page.set_content(html).await.map_err(failed)?;
            evaluate(page, reader).await
        })
        .await
    }

    /// Navigate to a demo URL and save what the browser downloads. Chrome writes the file under its
    /// download guid; only once it reports completion is the file renamed to HLTV's own name, so a
    /// partial archive never carries an archive's name, and a run that ends early leaves nothing.
    async fn download_to(&self, url: &str, out: &Path) -> Result<Saved, HltvError> {
        let unwritable = |e: std::io::Error| HltvError::DownloadFailed {
            url: url.into(),
            reason: format!("could not write to {}: {e}", out.display()),
            bytes: None,
        };
        std::fs::create_dir_all(out).map_err(unwritable)?;
        let behavior = SetDownloadBehaviorParams::builder()
            .behavior(SetDownloadBehaviorBehavior::AllowAndName)
            .download_path(out.to_string_lossy())
            .events_enabled(true)
            .build()
            .map_err(failed)?;
        self.browser.execute(behavior).await.map_err(failed)?;
        // Listen before navigating: the navigation that becomes a download never reports back.
        let mut begins = self
            .browser
            .event_listener::<EventDownloadWillBegin>()
            .await
            .map_err(failed)?;
        let mut progress = self
            .browser
            .event_listener::<EventDownloadProgress>()
            .await
            .map_err(failed)?;
        self.with_page(async |page| {
            let started = Instant::now();
            fire(page, url);
            let began = wait_for_download(page, url, &self.profile, &mut begins).await?;
            debug!(
                ms = started.elapsed().as_millis() as u64,
                name = %began.suggested_filename,
                "download began"
            );
            let partial = Partial::new(out, &began.guid);
            let followed = async {
                let name = archive_name(url, &began.suggested_filename)?;
                Ok((name, follow(url, &began.guid, &mut progress).await?))
            }
            .await;
            let (name, bytes) = match followed {
                Ok(done) => done,
                Err(error) => {
                    let cancel = CancelDownloadParams::new(began.guid.clone());
                    let _ = timeout(CLOSE_TIMEOUT, self.browser.execute(cancel)).await;
                    return Err(error);
                }
            };
            debug!(
                ms = started.elapsed().as_millis() as u64,
                bytes, "download completed"
            );
            let path = out.join(name);
            partial.keep(&path).map_err(|e| HltvError::DownloadFailed {
                url: url.into(),
                reason: format!("could not save: {e}"),
                bytes: Some(bytes),
            })?;
            match std::fs::metadata(&path).map(|m| m.len()) {
                Ok(0) => {
                    let _ = std::fs::remove_file(&path);
                    Err(HltvError::DownloadFailed {
                        url: url.into(),
                        reason: "empty file".into(),
                        bytes: Some(0),
                    })
                }
                Ok(_) => Ok(Saved {
                    path,
                    suggested_name: name.to_string(),
                }),
                Err(e) => Err(HltvError::DownloadFailed {
                    url: url.into(),
                    reason: format!("could not save: {e}"),
                    bytes: Some(bytes),
                }),
            }
        })
        .await
    }
}

/// Start a navigation without waiting for its reply.
fn fire(page: &Page, url: &str) {
    let page = page.clone();
    let params = NavigateParams::new(url);
    tokio::spawn(async move {
        let _ = timeout(PAGE_TIMEOUT, page.execute(params)).await;
    });
}

async fn evaluate<T: DeserializeOwned>(page: &Page, expression: &str) -> Result<T, HltvError> {
    let value = page
        .evaluate(expression.to_string())
        .await
        .map_err(failed)?;
    value.into_value::<T>().map_err(failed)
}

/// Where the page is, and whether `selector` exists yet. None while the page is between documents.
async fn probe(page: &Page, selector: &str) -> Option<Probe> {
    let selector = serde_json::to_string(selector).ok()?;
    let expression = format!(
        "({{ href: location.href, title: document.title, \
         ready: document.querySelector({selector}) !== null, \
         loaded: document.readyState === 'complete' }})"
    );
    let value = timeout(Duration::from_secs(5), page.evaluate(expression))
        .await
        .ok()?
        .ok()?;
    value.into_value().ok()
}

async fn navigate(page: &Page, url: &str, selector: &str, profile: &Path) -> Result<(), HltvError> {
    let mut arrival = Arrival::new(url, profile);
    let started = Instant::now();
    fire(page, url);
    loop {
        let seen = probe(page, selector).await;
        tracing::trace!(url, ?seen, "probe");
        match arrival.observe(seen.as_ref(), started.elapsed()) {
            Verdict::Wait => sleep(POLL).await,
            Verdict::Read => {
                debug!(url, ms = started.elapsed().as_millis() as u64, "arrived");
                return Ok(());
            }
            Verdict::Fail(error) => {
                debug!(url, ms = started.elapsed().as_millis() as u64, %error, "did not arrive");
                return Err(error);
            }
        }
    }
}

/// The browser's announcement of the download, or the reason none is coming.
async fn wait_for_download(
    page: &Page,
    url: &str,
    profile: &Path,
    begins: &mut EventStream<EventDownloadWillBegin>,
) -> Result<std::sync::Arc<EventDownloadWillBegin>, HltvError> {
    let mut departure = Departure::new(url, profile);
    let started = Instant::now();
    loop {
        tokio::select! {
            began = begins.next() => {
                return began.ok_or_else(|| failed("the browser stopped reporting downloads"));
            }
            _ = sleep(Duration::from_secs(1)) => {
                let seen = probe(page, "html").await;
                if let Some(error) = departure.observe(seen.as_ref(), started.elapsed()) {
                    return Err(error);
                }
            }
        }
    }
}

/// Follow one download's progress to completion; the bytes received.
async fn follow(
    url: &str,
    guid: &str,
    progress: &mut EventStream<EventDownloadProgress>,
) -> Result<u64, HltvError> {
    let mut received = None;
    let fail = |reason: &str, bytes: Option<u64>| HltvError::DownloadFailed {
        url: url.into(),
        reason: reason.into(),
        bytes,
    };
    loop {
        let event = match timeout(DOWNLOAD_STALL_TIMEOUT, progress.next()).await {
            Ok(Some(event)) => event,
            Ok(None) => return Err(fail("the browser stopped reporting progress", received)),
            Err(_) => return Err(fail("stalled", received)),
        };
        if event.guid != guid {
            continue;
        }
        received = Some(event.received_bytes as u64);
        match event.state {
            DownloadProgressState::InProgress => {}
            DownloadProgressState::Canceled => return Err(fail("canceled", received)),
            DownloadProgressState::Completed => return Ok(received.unwrap_or(0)),
        }
    }
}

/// HLTV's own file name, used as it is. One that could leave the folder is refused, not repaired.
fn archive_name<'a>(url: &str, suggested: &'a str) -> Result<&'a str, HltvError> {
    let plain = Path::new(suggested)
        .file_name()
        .is_some_and(|name| name == suggested);
    if plain && suggested != ".." {
        Ok(suggested)
    } else {
        Err(HltvError::DownloadFailed {
            url: url.into(),
            reason: format!("the browser suggested an unsafe file name: {suggested:?}"),
            bytes: None,
        })
    }
}

/// The guid-named file a download is written to. Removed when dropped, unless it was kept: renamed
/// to the archive's name after the browser reported completion.
struct Partial {
    paths: [PathBuf; 2],
    kept: bool,
}

impl Partial {
    fn new(out: &Path, guid: &str) -> Self {
        Self {
            paths: [out.join(guid), out.join(format!("{guid}.crdownload"))],
            kept: false,
        }
    }

    fn keep(mut self, path: &Path) -> std::io::Result<()> {
        std::fs::rename(&self.paths[0], path)?;
        self.kept = true;
        Ok(())
    }
}

impl Drop for Partial {
    fn drop(&mut self) {
        if !self.kept {
            for path in &self.paths {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

/// A headed Chrome on the shared profile, holding the profile lock until it closes.
pub struct Session {
    driver: Driver,
    _lock: Lock,
}

impl Session {
    /// A profile that cannot be locked, or a Chrome that will not launch, is reported as
    /// BrowserMissing (open decision D5). A profile another Chrome already has open, such as the window
    /// opened to answer a challenge, is ProfileBusy.
    pub async fn open(headless: bool) -> Result<Self, HltvError> {
        let executable = chrome_path();
        let missing = || HltvError::BrowserMissing {
            tried: executable.clone(),
        };
        if !executable.exists() {
            return Err(missing());
        }
        let profile = profile_dir();
        let lock = match lock::acquire(&lock_path()).map_err(|_| missing())? {
            Acquired::Held(lock) => lock,
            Acquired::Busy { pid } => return Err(HltvError::ProfileBusy { profile, pid }),
        };
        if let Some(pid) = lock::chrome_holder(&profile) {
            return Err(HltvError::ProfileBusy {
                profile,
                pid: Some(pid),
            });
        }
        let mut config = BrowserConfig::builder()
            .chrome_executable(&executable)
            .user_data_dir(&profile)
            .disable_default_args()
            .args(BASE_SWITCHES)
            .arg(PASSWORD_STORE)
            .arg(("disable-blink-features", "AutomationControlled"))
            .viewport(None)
            .launch_timeout(PAGE_TIMEOUT)
            .request_timeout(PAGE_TIMEOUT);
        if !headless {
            config = config.with_head();
        }
        let config = config.build().map_err(|_| missing())?;
        let driver = Driver::launch(config, profile).await.map_err(|reason| {
            debug!(%reason, "chrome did not launch");
            missing()
        })?;
        Ok(Self {
            driver,
            _lock: lock,
        })
    }

    pub async fn close(self) {
        self.driver.close().await
    }

    /// Open `url` in a fresh page, wait until it arrived and `selector` exists (or the page settled
    /// without it), then evaluate `reader`, a self-contained JavaScript expression, in the page.
    pub async fn read<T: DeserializeOwned>(
        &self,
        url: &str,
        selector: &str,
        reader: &str,
    ) -> Result<T, HltvError> {
        self.driver.read(url, selector, reader).await
    }
}

impl Source for Session {
    async fn results(&self, url: &str) -> Result<Vec<RawMatch>, HltvError> {
        self.read(url, extract::RESULTS_READY, &extract::read_matches())
            .await
    }

    async fn match_page(&self, url: &str) -> Result<RawMatchPage, HltvError> {
        self.read(url, extract::MATCH_READY, &extract::read_match_page())
            .await
    }

    async fn download(&self, demo_url: &str, out: &Path) -> Result<Saved, HltvError> {
        self.driver.download_to(demo_url, out).await
    }
}

/// Opens a real, headed Chrome per unit of work.
#[derive(Debug, Clone, Copy, Default)]
pub struct Chrome;

impl Opener for Chrome {
    type Source = Session;

    async fn open(&self) -> Result<Session, HltvError> {
        Session::open(headless()).await
    }

    async fn close(&self, session: Session) {
        session.close().await
    }
}

/// A headless Chrome on a throwaway profile, with the network switched off. It reads saved HTML with
/// the production readers (the fixture tests, `cargo xtask capture-fixtures --trim`), and proves the
/// failure paths of a live read without touching hltv.org: every URL it opens is unreachable.
pub struct Offline {
    driver: Driver,
    _profile: tempfile::TempDir,
}

impl Offline {
    pub async fn launch() -> Result<Self, HltvError> {
        let executable = chrome_path();
        let missing = || HltvError::BrowserMissing {
            tried: executable.clone(),
        };
        if !executable.exists() {
            return Err(missing());
        }
        let profile = tempfile::Builder::new()
            .prefix("hltv-demo-offline-")
            .tempdir()
            .map_err(failed)?;
        let config = BrowserConfig::builder()
            .chrome_executable(&executable)
            .user_data_dir(profile.path())
            .disable_default_args()
            .args(BASE_SWITCHES)
            .arg(PASSWORD_STORE)
            // Every host name fails to resolve, so nothing this browser opens reaches the network.
            .arg(("host-resolver-rules", "MAP * ~NOTFOUND"))
            .launch_timeout(PAGE_TIMEOUT)
            .request_timeout(PAGE_TIMEOUT)
            .build()
            .map_err(|_| missing())?;
        let driver = Driver::launch(config, profile.path().to_path_buf())
            .await
            .map_err(|_| missing())?;
        Ok(Self {
            driver,
            _profile: profile,
        })
    }

    pub async fn close(self) {
        self.driver.close().await
    }

    /// Load `html` into a fresh page and evaluate `reader` in it.
    pub async fn read_html<T: DeserializeOwned>(
        &self,
        html: &str,
        reader: &str,
    ) -> Result<T, HltvError> {
        self.driver.read_html(html, reader).await
    }

    /// The live read path, with no network under it.
    pub async fn read<T: DeserializeOwned>(
        &self,
        url: &str,
        selector: &str,
        reader: &str,
    ) -> Result<T, HltvError> {
        self.driver.read(url, selector, reader).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_names_are_used_as_given_or_refused() {
        let url = "https://www.hltv.org/download/demo/1";
        let name = "pgl-masters-bucharest-2026-vitality-vs-9z-bo3-eP_cF4LqZBl7PERj7oPEob.rar";
        assert_eq!(archive_name(url, name).unwrap(), name);
        for unsafe_name in ["../x.rar", "a/b.rar", "..", "", "/abs.rar"] {
            assert!(archive_name(url, unsafe_name).is_err(), "{unsafe_name:?}");
        }
    }

    #[test]
    fn a_partial_download_is_removed_unless_kept() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("guid-1"), b"partial").unwrap();
        drop(Partial::new(dir.path(), "guid-1"));
        assert!(!dir.path().join("guid-1").exists());

        std::fs::write(dir.path().join("guid-2"), b"Rar!").unwrap();
        Partial::new(dir.path(), "guid-2")
            .keep(&dir.path().join("a-vs-b.rar"))
            .unwrap();
        assert!(dir.path().join("a-vs-b.rar").exists());
        assert!(!dir.path().join("guid-2").exists());
    }
}
