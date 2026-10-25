//! `<out>/.hltv-demo.tsv` maps a demo URL to the file it produced, so a repeat fetch does not pull
//! hundreds of megabytes again. A line whose file is gone is ignored, which makes deleting the archive
//! the way to re-fetch it. The format is the TypeScript version's, so both share one folder.

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use crate::core::errors::HltvError;
use crate::core::types::ManifestEntry;

pub const MANIFEST: &str = ".hltv-demo.tsv";

pub fn manifest_path(out: &Path) -> PathBuf {
    out.join(MANIFEST)
}

fn unreadable(out: &Path, reason: impl ToString) -> HltvError {
    HltvError::ManifestUnreadable {
        path: manifest_path(out),
        reason: reason.to_string(),
    }
}

pub fn parse(text: &str) -> Result<Vec<ManifestEntry>, String> {
    text.lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .map(|line| match line.split_once('\t') {
            Some((url, name)) if !url.is_empty() => Ok(ManifestEntry {
                demo_url: url.to_string(),
                file_name: name.to_string(),
            }),
            _ => Err(format!("malformed line: {line}")),
        })
        .collect()
}

pub fn format(entry: &ManifestEntry) -> String {
    format!("{}\t{}\n", entry.demo_url, entry.file_name)
}

pub fn read(out: &Path) -> Result<Vec<ManifestEntry>, HltvError> {
    match fs::read_to_string(manifest_path(out)) {
        Ok(text) => parse(&text).map_err(|reason| unreadable(out, reason)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(unreadable(out, error)),
    }
}

/// Record a download, ignoring an exact repeat.
pub fn record(out: &Path, entry: &ManifestEntry) -> Result<(), HltvError> {
    if read(out)?.contains(entry) {
        return Ok(());
    }
    fs::create_dir_all(out).map_err(|e| unreadable(out, e))?;
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(manifest_path(out))
        .and_then(|mut file| file.write_all(format(entry).as_bytes()))
        .map_err(|e| unreadable(out, e))
}

/// The file a previous run downloaded for this demo URL, if it is still on disk.
pub fn lookup(out: &Path, demo_url: &str) -> Result<Option<PathBuf>, HltvError> {
    let hit = read(out)?
        .into_iter()
        .find(|entry| entry.demo_url == demo_url);
    Ok(hit
        .map(|entry| out.join(entry.file_name))
        .filter(|path| path.is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(url: &str, name: &str) -> ManifestEntry {
        ManifestEntry {
            demo_url: url.into(),
            file_name: name.into(),
        }
    }

    #[test]
    fn format_and_parse_round_trip() {
        let one = entry("https://www.hltv.org/download/demo/1", "a-b-vs-c-bo3-x.rar");
        assert_eq!(parse(&format(&one)).unwrap(), vec![one.clone()]);
        assert_eq!(
            parse(&format!("# header\n\n{}", format(&one))).unwrap(),
            vec![one]
        );
    }

    #[test]
    fn a_missing_manifest_is_empty_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read(dir.path()).unwrap(), vec![]);
    }

    #[test]
    fn recording_is_idempotent_and_lookup_resolves_only_while_the_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        let demo_url = "https://www.hltv.org/download/demo/2";
        let file = dir.path().join("event-a-vs-b-bo3-y.rar");

        assert_eq!(lookup(dir.path(), demo_url).unwrap(), None);
        fs::write(&file, "rar").unwrap();
        record(dir.path(), &entry(demo_url, "event-a-vs-b-bo3-y.rar")).unwrap();
        record(dir.path(), &entry(demo_url, "event-a-vs-b-bo3-y.rar")).unwrap();

        assert_eq!(read(dir.path()).unwrap().len(), 1);
        assert_eq!(lookup(dir.path(), demo_url).unwrap(), Some(file.clone()));

        fs::remove_file(&file).unwrap();
        assert_eq!(lookup(dir.path(), demo_url).unwrap(), None);
    }

    #[test]
    fn a_corrupt_line_is_a_typed_failure_not_a_silent_skip() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(manifest_path(dir.path()), "no-tab-here\n").unwrap();
        assert!(matches!(
            read(dir.path()),
            Err(HltvError::ManifestUnreadable { .. })
        ));
    }

    #[test]
    fn the_file_is_the_typescript_versions_format() {
        let dir = tempfile::tempdir().unwrap();
        record(
            dir.path(),
            &entry(
                "https://www.hltv.org/download/demo/5",
                "event-a-vs-b-bo3-x.rar",
            ),
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(manifest_path(dir.path())).unwrap(),
            "https://www.hltv.org/download/demo/5\tevent-a-vs-b-bo3-x.rar\n"
        );
    }
}
