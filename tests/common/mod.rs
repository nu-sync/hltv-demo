//! A `Source` that answers from memory and writes archive stubs, with a log of every call: the
//! operations and the MCP server run against it with no Chrome and no network.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use hltv_demo::HltvError;
use hltv_demo::core::url::page_key;
use hltv_demo::hltv::extract::{RawMatch, RawMatchPage};
use hltv_demo::source::{Opener, Saved, Source};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Calls {
    pub results: Vec<String>,
    pub matches: Vec<String>,
    pub downloads: Vec<String>,
}

type DemoFor = Arc<dyn Fn(u64) -> Option<u64> + Send + Sync>;

#[derive(Clone)]
pub struct Fake {
    rows: Vec<RawMatch>,
    demo_for: DemoFor,
    failing: HashMap<String, String>,
    calls: Arc<Mutex<Calls>>,
    opened: Arc<AtomicUsize>,
}

impl Default for Fake {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            demo_for: Arc::new(|_| None),
            failing: HashMap::new(),
            calls: Arc::default(),
            opened: Arc::default(),
        }
    }
}

/// A results row that looks like hltv.org's, under a date heading.
pub fn row(match_id: u64) -> RawMatch {
    headed(match_id, Some("Results for October 24th 2026"))
}

pub fn headed(match_id: u64, heading: Option<&str>) -> RawMatch {
    RawMatch {
        href: format!("/matches/{match_id}/vitality-vs-9z"),
        teams: ("Vitality".into(), "9z".into()),
        scores: Some((2, 0)),
        event: Some("PGL Masters Bucharest 2026".into()),
        heading: heading.map(String::from),
    }
}

impl Fake {
    /// The rows every results URL answers with.
    pub fn rows(mut self, rows: Vec<RawMatch>) -> Self {
        self.rows = rows;
        self
    }

    /// The demo id each match page links to, by match id; None for a match without a demo.
    pub fn demo(mut self, demo_for: impl Fn(u64) -> Option<u64> + Send + Sync + 'static) -> Self {
        self.demo_for = Arc::new(demo_for);
        self
    }

    /// A demo URL whose download fails with this reason.
    pub fn failing(mut self, demo_url: &str, reason: &str) -> Self {
        self.failing.insert(demo_url.into(), reason.into());
        self
    }

    pub fn calls(&self) -> Calls {
        self.calls.lock().unwrap().clone()
    }

    /// How many units of work opened a browser.
    pub fn opened(&self) -> usize {
        self.opened.load(Ordering::SeqCst)
    }
}

impl Source for Fake {
    async fn results(&self, url: &str) -> Result<Vec<RawMatch>, HltvError> {
        self.calls.lock().unwrap().results.push(url.into());
        Ok(self.rows.clone())
    }

    async fn match_page(&self, url: &str) -> Result<RawMatchPage, HltvError> {
        self.calls.lock().unwrap().matches.push(url.into());
        let match_id = page_key(url)
            .trim_start_matches("/matches/")
            .parse()
            .unwrap_or(0);
        Ok(RawMatchPage {
            title: "match".into(),
            demo_href: (self.demo_for)(match_id).map(|demo| format!("/download/demo/{demo}")),
            date: None,
            event: None,
        })
    }

    async fn download(&self, demo_url: &str, out: &Path) -> Result<Saved, HltvError> {
        self.calls.lock().unwrap().downloads.push(demo_url.into());
        if let Some(reason) = self.failing.get(demo_url) {
            return Err(HltvError::DownloadFailed {
                url: demo_url.into(),
                reason: reason.clone(),
                bytes: None,
            });
        }
        let name = format!(
            "{}-event-a-vs-b-bo3-x.rar",
            demo_url.rsplit('/').next().unwrap()
        );
        std::fs::create_dir_all(out).unwrap();
        std::fs::write(out.join(&name), "rar").unwrap();
        Ok(Saved {
            path: out.join(&name),
            suggested_name: name,
        })
    }
}

impl Opener for Fake {
    type Source = Fake;

    async fn open(&self) -> Result<Fake, HltvError> {
        self.opened.fetch_add(1, Ordering::SeqCst);
        Ok(self.clone())
    }

    async fn close(&self, _source: Fake) {}
}
