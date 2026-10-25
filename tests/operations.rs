//! The operations shared by the CLI and the MCP server, against a fake Source. No browser, no
//! network: these are the rules that decide what a URL means, what gets skipped, and what a failure is.

mod common;

use std::sync::Mutex;
use std::time::Duration;

use common::{Fake, headed, row};
use hltv_demo::core::manifest;
use hltv_demo::hltv::get::{Options, get};
use hltv_demo::{DemoRef, HltvError, SearchFilter, resolve, search};

fn options(out: &std::path::Path, limit: usize) -> Options<'static> {
    Options {
        out: Some(out.to_path_buf()),
        limit: Some(limit),
        delay: Some(Duration::ZERO),
        on_note: None,
    }
}

#[tokio::test]
async fn search_asks_hltv_for_the_team_and_map_it_was_given() {
    let fake = Fake::default().rows(vec![row(1), row(2)]);
    let filter = SearchFilter {
        team: Some(9565),
        map: Some("cache".into()),
        limit: Some(2),
        offset: None,
    };
    let matches = search(&fake, &filter).await.unwrap();
    assert_eq!(
        fake.calls().results,
        vec!["https://www.hltv.org/results?team=9565&map=de_cache"]
    );
    assert_eq!(
        matches.iter().map(|m| m.match_id).collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[tokio::test]
async fn search_respects_the_limit_and_drops_the_featured_duplicate() {
    let fake = Fake::default().rows(vec![row(1), headed(1, None), row(2), row(3)]);
    let matches = search(
        &fake,
        &SearchFilter {
            limit: Some(2),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        matches.iter().map(|m| m.match_id).collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(matches[0].date.as_deref(), Some("2026-10-24"));
}

#[tokio::test]
async fn an_empty_results_page_is_no_matches_found_with_the_url() {
    let filter = SearchFilter {
        team: Some(1),
        limit: Some(1),
        ..Default::default()
    };
    let error = search(&Fake::default(), &filter).await.unwrap_err();
    assert!(
        matches!(error, HltvError::NoMatchesFound { .. }),
        "{error:?}"
    );
    assert!(error.to_string().contains("/results?team=1"), "{error}");
}

#[tokio::test]
async fn resolve_reads_the_demo_id_from_the_match_page() {
    let fake = Fake::default().demo(|_| Some(113159));
    let demo = resolve(&fake, "/matches/2399723/vitality-vs-9z")
        .await
        .unwrap();
    assert_eq!(
        demo,
        DemoRef {
            match_id: 2399723,
            demo_id: 113159,
            url: "https://www.hltv.org/download/demo/113159".into(),
            match_url: "https://www.hltv.org/matches/2399723/vitality-vs-9z".into(),
        }
    );
    assert_eq!(
        fake.calls().matches,
        vec!["https://www.hltv.org/matches/2399723/vitality-vs-9z"]
    );
}

#[tokio::test]
async fn a_match_with_no_demo_is_no_demo_found() {
    let error = resolve(&Fake::default(), "/matches/1/x").await.unwrap_err();
    assert!(matches!(error, HltvError::NoDemoFound { .. }), "{error:?}");
}

#[tokio::test]
async fn resolving_a_non_match_url_never_opens_a_page() {
    let fake = Fake::default();
    let error = resolve(&fake, "/download/demo/1").await.unwrap_err();
    assert!(matches!(error, HltvError::NoDemoFound { .. }), "{error:?}");
    assert!(fake.calls().matches.is_empty());
}

#[tokio::test]
async fn a_demo_url_downloads_once_and_is_recorded_in_the_manifest() {
    let out = tempfile::tempdir().unwrap();
    let fake = Fake::default();
    let archives = get(&fake, "/download/demo/113159", &options(out.path(), 1))
        .await
        .unwrap();
    assert_eq!(archives.len(), 1);
    assert_eq!(
        fake.calls().downloads,
        vec!["https://www.hltv.org/download/demo/113159"]
    );
    assert_eq!(
        manifest::read(out.path()).unwrap()[0].file_name,
        "113159-event-a-vs-b-bo3-x.rar"
    );

    let again = get(&fake, "/download/demo/113159", &options(out.path(), 1))
        .await
        .unwrap();
    assert_eq!(again[0].path, archives[0].path);
    assert_eq!(again[0].bytes, archives[0].bytes);
    assert_eq!(fake.calls().downloads.len(), 1, "the second run skipped it");
}

#[tokio::test]
async fn a_match_url_resolves_first() {
    let out = tempfile::tempdir().unwrap();
    let fake = Fake::default().demo(|_| Some(42));
    let archives = get(
        &fake,
        "https://www.hltv.org/matches/7/x",
        &options(out.path(), 1),
    )
    .await
    .unwrap();
    assert_eq!(fake.calls().matches.len(), 1);
    assert_eq!(
        archives[0].demo_url,
        "https://www.hltv.org/download/demo/42"
    );
}

#[tokio::test]
async fn a_results_url_keeps_its_filter_and_continues_past_a_match_with_no_demo() {
    let out = tempfile::tempdir().unwrap();
    let fake = Fake::default()
        .rows(vec![row(1), row(2)])
        .demo(|id| (id != 1).then_some(id));
    let notes = Mutex::new(Vec::new());
    let on_note = |text: &str| notes.lock().unwrap().push(text.to_string());
    let url = "https://www.hltv.org/results?team=9565&map=de_inferno";
    let archives = get(
        &fake,
        url,
        &Options {
            on_note: Some(&on_note),
            ..options(out.path(), 2)
        },
    )
    .await
    .unwrap();
    assert_eq!(fake.calls().results, vec![url]);
    assert_eq!(
        fake.calls().downloads,
        vec!["https://www.hltv.org/download/demo/2"]
    );
    assert_eq!(archives.len(), 1);
    assert!(
        notes
            .into_inner()
            .unwrap()
            .contains(&"nodemo https://www.hltv.org/matches/1/vitality-vs-9z".into())
    );
}

#[tokio::test]
async fn a_results_url_continues_past_a_download_that_fails_and_says_so() {
    let out = tempfile::tempdir().unwrap();
    let fake = Fake::default()
        .rows(vec![row(1), row(2)])
        .demo(Some)
        .failing("https://www.hltv.org/download/demo/1", "stalled");
    let notes = Mutex::new(Vec::new());
    let on_note = |text: &str| notes.lock().unwrap().push(text.to_string());
    let archives = get(
        &fake,
        "/results",
        &Options {
            on_note: Some(&on_note),
            ..options(out.path(), 2)
        },
    )
    .await
    .unwrap();
    assert_eq!(
        archives
            .iter()
            .map(|a| a.demo_url.as_str())
            .collect::<Vec<_>>(),
        vec!["https://www.hltv.org/download/demo/2"]
    );
    assert!(notes.into_inner().unwrap().contains(
        &"error https://www.hltv.org/download/demo/1: https://www.hltv.org/download/demo/1 did not download: stalled"
            .to_string()
    ));
    assert_eq!(
        manifest::read(out.path()).unwrap().len(),
        1,
        "nothing partial is recorded"
    );
}

#[tokio::test]
async fn a_corrupt_manifest_is_a_typed_manifest_unreadable() {
    let out = tempfile::tempdir().unwrap();
    std::fs::write(out.path().join(".hltv-demo.tsv"), "no-tab-here\n").unwrap();
    let fake = Fake::default();
    let error = get(&fake, "/download/demo/1", &options(out.path(), 1))
        .await
        .unwrap_err();
    assert!(
        matches!(error, HltvError::ManifestUnreadable { .. }),
        "{error:?}"
    );
    assert!(fake.calls().downloads.is_empty());
}

#[tokio::test]
async fn a_foreign_host_is_refused_before_anything_opens() {
    let out = tempfile::tempdir().unwrap();
    let fake = Fake::default();
    let error = get(
        &fake,
        "https://example.com/matches/1/x",
        &options(out.path(), 1),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, HltvError::NotHltvUrl { .. }), "{error:?}");
    assert_eq!(fake.calls(), Default::default());
}

#[tokio::test]
async fn an_archive_deleted_after_its_download_is_fetched_again() {
    let out = tempfile::tempdir().unwrap();
    let fake = Fake::default();
    let first = get(&fake, "/download/demo/5", &options(out.path(), 1))
        .await
        .unwrap();
    std::fs::remove_file(&first[0].path).unwrap();
    get(&fake, "/download/demo/5", &options(out.path(), 1))
        .await
        .unwrap();
    assert_eq!(fake.calls().downloads.len(), 2);
    assert_eq!(
        manifest::read(out.path()).unwrap().len(),
        1,
        "an exact repeat is recorded once"
    );
}
