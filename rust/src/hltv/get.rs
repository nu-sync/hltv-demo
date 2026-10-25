use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use crate::core::errors::HltvError;
use crate::core::manifest;
use crate::core::types::{Archive, ManifestEntry, SearchFilter};
use crate::core::url::{Target, classify, results_filter};
use crate::hltv::{resolve::resolve, search::search};
use crate::source::Source;

/// Where archives go when a run names no folder.
pub fn default_out() -> PathBuf {
    std::env::var_os("HLTV_DEMO_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_default()
                .join(".cache")
                .join("hltv-demo")
                .join("archives")
        })
}

/// One canonical folder, so every path a run reports is the same string: absolute, with `.`, `..`,
/// and doubled or trailing slashes resolved lexically. macOS `TMPDIR` ends in `/`, so an
/// unnormalized join would differ by `//`. Symlinks are left alone.
pub fn canonical_out(out: Option<&Path>) -> PathBuf {
    let out = out.map(Path::to_path_buf).unwrap_or_else(default_out);
    let absolute = if out.is_absolute() {
        out
    } else {
        std::env::current_dir().unwrap_or_default().join(out)
    };
    let mut canonical = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                canonical.pop();
            }
            other => canonical.push(other),
        }
    }
    canonical
}

/// The pause between the matches of a results URL, so a list is not a burst.
pub const DEFAULT_DELAY: Duration = Duration::from_millis(2000);

#[derive(Default)]
pub struct Options<'a> {
    /// Destination folder; default `default_out()`.
    pub out: Option<PathBuf>,
    /// How many matches a results URL contributes: default 1, at most MAX_LIMIT.
    pub limit: Option<usize>,
    /// Pause between matches; default DEFAULT_DELAY.
    pub delay: Option<Duration>,
    /// Human progress, and what was skipped.
    pub on_note: Option<&'a (dyn Fn(&str) + Send + Sync)>,
}

/// Turn any supported URL into archives on disk.
///
/// A results URL is the only multi-match case. A match with no demo, or one that fails to download,
/// is a note rather than a failed run: the rest continue. Nothing partial is ever returned or recorded.
pub async fn get(
    source: &impl Source,
    url: &str,
    options: &Options<'_>,
) -> Result<Vec<Archive>, HltvError> {
    let target = classify(url)?;
    let out = canonical_out(options.out.as_deref());
    let note = |text: String| {
        if let Some(on_note) = options.on_note {
            on_note(&text)
        }
    };

    match target {
        Target::Demo { url, .. } => Ok(vec![one(source, &out, &url, &note).await?]),
        Target::Match { url, .. } => {
            let demo = resolve(source, &url).await?;
            Ok(vec![one(source, &out, &demo.url, &note).await?])
        }
        Target::Results { url } => {
            // Keep the URL's own filter: `results?team=9565&map=de_inferno` must not become `/results`.
            let filter = results_filter(&url)?;
            let filter = SearchFilter {
                team: filter.team,
                map: filter.map,
                offset: filter.offset,
                limit: options.limit,
            };
            let delay = options.delay.unwrap_or(DEFAULT_DELAY);
            let mut archives = Vec::new();
            for (index, found) in search(source, &filter).await?.into_iter().enumerate() {
                if index > 0 {
                    tokio::time::sleep(delay).await;
                }
                let demo = match resolve(source, &found.url).await {
                    Err(HltvError::NoDemoFound { .. }) => {
                        note(format!("nodemo {}", found.url));
                        continue;
                    }
                    other => other?,
                };
                match one(source, &out, &demo.url, &note).await {
                    Err(error @ HltvError::DownloadFailed { .. }) => {
                        note(format!("error {}: {error}", demo.url))
                    }
                    other => archives.push(other?),
                }
            }
            Ok(archives)
        }
    }
}

/// One demo URL to one archive on disk, skipping a file the manifest already points at.
async fn one(
    source: &impl Source,
    out: &Path,
    demo_url: &str,
    note: &impl Fn(String),
) -> Result<Archive, HltvError> {
    let size = |path: &Path| {
        std::fs::metadata(path)
            .map(|m| m.len())
            .map_err(|e| HltvError::DownloadFailed {
                url: demo_url.to_string(),
                reason: format!("archive vanished: {e}"),
                bytes: None,
            })
    };
    if let Some(known) = manifest::lookup(out, demo_url)? {
        note(format!("skip  {demo_url} (already downloaded)"));
        return Ok(Archive {
            bytes: size(&known)?,
            suggested_name: known
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            path: known.to_string_lossy().into_owned(),
            demo_url: demo_url.to_string(),
        });
    }
    note(format!("get   {demo_url}"));
    let saved = source.download(demo_url, out).await?;
    manifest::record(
        out,
        &ManifestEntry {
            demo_url: demo_url.to_string(),
            file_name: saved.suggested_name.clone(),
        },
    )?;
    let bytes = size(&saved.path)?;
    note(format!(
        "done  {} ({:.0} MB)",
        saved.path.display(),
        bytes as f64 / 1e6
    ));
    Ok(Archive {
        path: saved.path.to_string_lossy().into_owned(),
        bytes,
        demo_url: demo_url.to_string(),
        suggested_name: saved.suggested_name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_is_absolute_and_lexically_normalized() {
        assert_eq!(
            canonical_out(Some(Path::new("/tmp/a/../b/./c/"))),
            PathBuf::from("/tmp/b/c")
        );
        assert_eq!(
            canonical_out(Some(Path::new("/tmp//x"))),
            PathBuf::from("/tmp/x")
        );
        assert_eq!(canonical_out(Some(Path::new("/.."))), PathBuf::from("/"));
        assert_eq!(
            canonical_out(Some(Path::new("rel/../demos"))),
            std::env::current_dir().unwrap().join("demos")
        );
    }
}
