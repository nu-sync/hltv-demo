//! The browser boundary, and the only thing the operations need. Everything above it is
//! deterministic and testable without a browser: `search`, `resolve`, and `get` take a `Source`,
//! never a `Session`.

use std::future::Future;
use std::path::{Path, PathBuf};

use crate::core::errors::HltvError;
use crate::hltv::extract::{RawMatch, RawMatchPage};

/// What a finished download left on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    pub path: PathBuf,
    pub suggested_name: String,
}

pub trait Source: Send + Sync {
    /// The match rows of a results page.
    fn results(&self, url: &str) -> impl Future<Output = Result<Vec<RawMatch>, HltvError>> + Send;
    /// The match page: demo link, event, date.
    fn match_page(&self, url: &str)
    -> impl Future<Output = Result<RawMatchPage, HltvError>> + Send;
    /// Download a demo URL into `out`; resolves only when the browser reports completion.
    fn download(
        &self,
        demo_url: &str,
        out: &Path,
    ) -> impl Future<Output = Result<Saved, HltvError>> + Send;
}

/// Opens one `Source` per unit of work. A CLI command is one unit; an MCP tool call is one unit.
/// That keeps Chrome from idling for hours behind an MCP server that is only listing tools, and the
/// profile lock keeps two units from sharing a browser anyway.
pub trait Opener: Send + Sync + 'static {
    type Source: Source;
    fn open(&self) -> impl Future<Output = Result<Self::Source, HltvError>> + Send;
    /// Release the browser and the profile lock. Dropping the source does the same, less gracefully.
    fn close(&self, source: Self::Source) -> impl Future<Output = ()> + Send;
}

/// Run one unit of work with a freshly opened source, and close it however the work ends.
pub async fn with_source<O, T, F>(opener: &O, work: F) -> Result<T, HltvError>
where
    O: Opener,
    F: AsyncFnOnce(&O::Source) -> Result<T, HltvError>,
{
    let source = opener.open().await?;
    let result = work(&source).await;
    opener.close(source).await;
    result
}
