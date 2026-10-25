//! Selector drift alarm. The production readers run against real HTML saved from hltv.org, inside a
//! headless Chrome with the network switched off. Refresh the pages with
//! `cargo xtask capture-fixtures`; when a selector stops matching, the failure is here rather than in
//! a user's download. Needs Chrome; without it each test says so and passes.

use std::path::Path;

use hltv_demo::HltvError;
use hltv_demo::browser::{Offline, chrome_path};
use hltv_demo::core::dates::from_heading;
use hltv_demo::hltv::extract::{self, RawMatch, RawMatchPage};
use hltv_demo::hltv::search::to_match;

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

async fn offline() -> Option<Offline> {
    if !chrome_path().exists() {
        eprintln!(
            "skipped: no Chrome at {} (set CHROME=)",
            chrome_path().display()
        );
        return None;
    }
    Some(Offline::launch().await.unwrap())
}

/// Run a reader the way production does: inside a page, over real HTML.
async fn read<T: serde::de::DeserializeOwned>(fixture_name: &str, reader: String) -> Option<T> {
    let browser = offline().await?;
    let value = browser.read_html(&fixture(fixture_name), &reader).await;
    browser.close().await;
    Some(value.unwrap())
}

fn is_iso_day(day: &str) -> bool {
    let bytes = day.as_bytes();
    bytes.len() == 10
        && bytes.iter().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                *b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
}

#[tokio::test]
async fn every_results_row_becomes_a_match_with_teams_event_and_date() {
    let Some(rows) = read::<Vec<RawMatch>>("results-cache.html", extract::read_matches()).await
    else {
        return;
    };
    assert!(!rows.is_empty());
    let first = &rows[0];
    let id = first
        .href
        .strip_prefix("/matches/")
        .and_then(|rest| rest.split('/').next())
        .unwrap();
    assert!(
        id.chars().all(|c| c.is_ascii_digit()) && !id.is_empty(),
        "{}",
        first.href
    );
    assert!(
        !first.teams.0.is_empty() && !first.teams.1.is_empty(),
        "{:?}",
        first.teams
    );
    assert!(
        first.event.as_deref().is_some_and(|e| !e.is_empty()),
        "{:?}",
        first.event
    );

    let found = to_match(first.clone()).unwrap();
    assert!(
        found.url.starts_with("https://www.hltv.org/matches/"),
        "{}",
        found.url
    );
}

#[tokio::test]
async fn rows_under_a_date_heading_get_an_iso_date_matching_the_heading() {
    let Some(rows) = read::<Vec<RawMatch>>("results-cache.html", extract::read_matches()).await
    else {
        return;
    };
    // The first block on a results page is "Featured results" and has no date heading, so only the
    // grouped rows can carry one. That is why the date is nullable rather than guessed.
    assert_eq!(rows[0].heading, None);
    let dated: Vec<_> = rows.iter().filter(|row| row.heading.is_some()).collect();
    assert!(!dated.is_empty());
    for row in dated {
        let found = to_match(row.clone()).unwrap();
        let date = found.date.as_deref().unwrap();
        assert!(is_iso_day(date), "{date}");
        assert_eq!(from_heading(row.heading.as_deref()).as_deref(), Some(date));
    }
}

#[tokio::test]
async fn scores_are_pairs_of_numbers_when_the_list_shows_them() {
    let Some(rows) = read::<Vec<RawMatch>>("results-cache.html", extract::read_matches()).await
    else {
        return;
    };
    // The type makes "a pair of whole numbers" the only shape a score can arrive in.
    assert!(rows.iter().any(|row| row.scores.is_some()));
}

#[tokio::test]
async fn the_match_page_exposes_the_demo_link_the_event_and_a_date() {
    let Some(page) = read::<RawMatchPage>("match.html", extract::read_match_page()).await else {
        return;
    };
    let href = page.demo_href.as_deref().unwrap();
    let id = href.strip_prefix("/download/demo/").unwrap();
    assert!(
        !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()),
        "{href}"
    );
    assert!(
        page.event.as_deref().is_some_and(|e| !e.is_empty()),
        "{:?}",
        page.event
    );
    assert!(is_iso_day(page.date.as_deref().unwrap()), "{:?}", page.date);
}

#[tokio::test]
async fn the_results_reader_finds_no_rows_on_a_match_page() {
    let Some(rows) = read::<Vec<RawMatch>>("match.html", extract::read_matches()).await else {
        return;
    };
    assert_eq!(rows, vec![]);
}

#[tokio::test]
async fn a_live_read_without_network_is_unreachable_not_a_challenge() {
    let Some(browser) = offline().await else {
        return;
    };
    let url = "https://www.hltv.org/results?team=9565";
    let read = browser
        .read::<Vec<RawMatch>>(url, extract::RESULTS_READY, &extract::read_matches())
        .await;
    browser.close().await;
    assert_eq!(
        read.unwrap_err(),
        HltvError::Unreachable { url: url.into() }
    );
}
