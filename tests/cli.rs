//! The CLI as a user runs it: the real binary, real exit codes, real output. Only paths that fail
//! before a page loads are here; anything that needs hltv.org lives in `cargo xtask live-check`.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run_with(args: &[&str], env: &[(&str, &Path)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_hltv-demo"));
    // A Chrome that does not exist: a test that reached a launch would say so, and exit 2.
    command
        .args(args)
        .env("CHROME", "/nonexistent/chrome")
        .env_remove("RUST_LOG");
    for (key, value) in env {
        command.env(key, value);
    }
    command.output().unwrap()
}

fn run(args: &[&str]) -> Output {
    run_with(args, &[])
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A stand-in Chrome that records how it was launched and exits, so no browser ever opens.
fn stand_in_chrome(dir: &Path) -> (PathBuf, PathBuf) {
    let chrome = dir.join("chrome");
    let args = dir.join("args");
    let script = format!(
        "#!/bin/sh\nfor arg in \"$@\"; do echo \"$arg\"; done > \"{}\"\nexit 1\n",
        args.display()
    );
    std::fs::write(&chrome, script).unwrap();
    std::fs::set_permissions(&chrome, std::fs::Permissions::from_mode(0o755)).unwrap();
    (chrome, args)
}

#[test]
fn help_prints_usage_and_exits_0() {
    let output = run(&["--help"]);
    assert_eq!(output.status.code(), Some(0));
    let text = stdout(&output);
    assert!(text.contains("hltv-demo search"), "{text}");
    assert!(text.contains("hltv-demo download <url>"), "{text}");
    assert!(text.contains("HLTV_DEMO_PROFILE"), "{text}");
}

#[test]
fn no_command_prints_usage_and_exits_0() {
    let output = run(&[]);
    assert_eq!(output.status.code(), Some(0));
    assert!(stdout(&output).contains("hltv-demo resolve <match-url>"));
}

#[test]
fn an_unknown_command_is_a_usage_error() {
    let output = run(&["frobnicate"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("frobnicate"));
}

#[test]
fn an_unknown_flag_is_a_usage_error() {
    assert_eq!(run(&["search", "--nope"]).status.code(), Some(2));
}

#[test]
fn a_flag_without_a_value_is_a_usage_error() {
    let output = run(&["search", "--limit"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("--limit"));
}

#[test]
fn a_limit_above_100_is_a_usage_error_pointing_at_offset() {
    let output = run(&["search", "--limit", "500"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("between 1 and 100"));
    assert!(stderr(&output).contains("--offset"));
    assert_eq!(
        run(&["download", "/results", "--limit", "0"]).status.code(),
        Some(2)
    );
}

#[test]
fn a_non_numeric_team_id_is_a_usage_error() {
    assert_eq!(
        run(&["search", "--team", "vitality"]).status.code(),
        Some(2)
    );
}

#[test]
fn a_map_that_cannot_be_a_map_name_is_a_usage_error() {
    let output = run(&["search", "--map", "cache&team=1"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("is not a map name"));
}

#[test]
fn resolve_without_a_url_is_a_usage_error() {
    let output = run(&["resolve"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("MATCH_URL"));
}

#[test]
fn a_url_off_hltv_fails_fast_with_a_remediation_without_opening_a_browser() {
    let output = run(&["download", "https://example.com/matches/1/x"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("is not an hltv.org URL"));
    assert!(stderr(&output).contains("/download/demo/"));
    assert!(
        !stderr(&output).contains("no Chrome found"),
        "it reached the browser"
    );
}

#[test]
fn an_hltv_page_we_do_not_read_is_a_domain_failure() {
    let output = run(&["resolve", "https://www.hltv.org/player/7998/s1mple"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("is not a match, results, or demo URL"));
}

#[test]
fn a_missing_chrome_is_a_prerequisite_failure() {
    let output = run(&["search", "--team", "9565"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("no Chrome found at /nonexistent/chrome"));
    assert!(stderr(&output).contains("CHROME=/path/to/chrome"));
}

#[test]
fn headless_reaches_the_browser_launch_so_the_challenge_it_meets_is_legible() {
    let dir = tempfile::tempdir().unwrap();
    let (chrome, args) = stand_in_chrome(dir.path());
    let profile = dir.path().join("profile").join("chrome");
    let env = [
        ("CHROME", chrome.as_path()),
        ("HLTV_DEMO_PROFILE", profile.as_path()),
    ];

    let headed = run_with(&["search"], &env);
    assert_eq!(headed.status.code(), Some(2), "{}", stderr(&headed));
    let launched = std::fs::read_to_string(&args).unwrap();
    let argv: Vec<&str> = launched.lines().collect();
    assert!(!argv.contains(&"--headless"), "{argv:?}");
    let user_data_dir = format!("--user-data-dir={}", profile.display());
    for expected in [
        user_data_dir.as_str(),
        "--disable-blink-features=AutomationControlled",
        "--use-mock-keychain",
        "--password-store=basic",
        "--no-first-run",
    ] {
        assert!(argv.contains(&expected), "{expected} missing from {argv:?}");
    }
    assert!(argv.iter().all(|arg| !arg.starts_with("---")), "{argv:?}");
    assert!(!argv.contains(&"--enable-automation"), "{argv:?}");

    run_with(&["search", "--headless"], &env);
    assert!(
        std::fs::read_to_string(&args)
            .unwrap()
            .lines()
            .any(|arg| arg == "--headless")
    );
    assert!(
        !dir.path().join("profile").join("chrome.lock").exists(),
        "the lock was released"
    );
}

#[test]
fn a_profile_another_run_holds_is_a_prerequisite_failure() {
    let dir = tempfile::tempdir().unwrap();
    let (chrome, _) = stand_in_chrome(dir.path());
    let profile = dir.path().join("profile");
    std::fs::write(
        dir.path().join("profile.lock"),
        std::process::id().to_string(),
    )
    .unwrap();
    let output = run_with(
        &["search"],
        &[
            ("CHROME", chrome.as_path()),
            ("HLTV_DEMO_PROFILE", profile.as_path()),
        ],
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains(&format!("is in use by pid {}", std::process::id())),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_profile_open_in_another_chrome_is_a_prerequisite_failure_not_a_missing_chrome() {
    let dir = tempfile::tempdir().unwrap();
    let (chrome, args) = stand_in_chrome(dir.path());
    let profile = dir.path().join("profile");
    std::fs::create_dir_all(&profile).unwrap();
    std::os::unix::fs::symlink(
        format!("host.local-{}", std::process::id()),
        profile.join("SingletonLock"),
    )
    .unwrap();
    let output = run_with(
        &["search"],
        &[
            ("CHROME", chrome.as_path()),
            ("HLTV_DEMO_PROFILE", profile.as_path()),
        ],
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("is in use by pid"),
        "{}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("close the Chrome window"),
        "{}",
        stderr(&output)
    );
    assert!(!args.exists(), "no second Chrome was launched");
}
