//! A signal stops a run cleanly: the browser it launched is killed, the profile lock is released, and
//! the exit code is 128 + the signal. A stand-in Chrome that never answers holds the run at launch,
//! with the lock taken, which is the moment a Ctrl-C would otherwise leave Chrome holding the profile.

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Rig {
    _dir: tempfile::TempDir,
    chrome: PathBuf,
    pidfile: PathBuf,
    profile: PathBuf,
    lock: PathBuf,
}

/// A Chrome that records its pid and then hangs without ever printing its DevTools address.
fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let chrome = dir.path().join("chrome");
    let pidfile = dir.path().join("chrome.pid");
    std::fs::write(
        &chrome,
        format!(
            "#!/bin/sh\necho $$ > \"{}\"\nexec sleep 30\n",
            pidfile.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&chrome, std::fs::Permissions::from_mode(0o755)).unwrap();
    let profile = dir.path().join("profile");
    let lock = dir.path().join("profile.lock");
    Rig {
        _dir: dir,
        chrome,
        pidfile,
        profile,
        lock,
    }
}

fn command(binary: &str, rig: &Rig) -> Command {
    let mut command = Command::new(binary);
    command
        .env("CHROME", &rig.chrome)
        .env("HLTV_DEMO_PROFILE", &rig.profile)
        .env_remove("RUST_LOG");
    command
}

fn wait_for(what: &str, ready: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !ready() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn running(pid: i32) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Wait for the stand-in to launch, signal the tool, and check what it left behind.
fn interrupt(mut child: Child, rig: &Rig, signal: i32) -> Option<i32> {
    wait_for("the stand-in Chrome", || {
        std::fs::read_to_string(&rig.pidfile).is_ok_and(|s| !s.trim().is_empty())
    });
    let chrome: i32 = std::fs::read_to_string(&rig.pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(
        rig.lock.exists(),
        "the run holds the profile lock while Chrome launches"
    );
    assert!(running(chrome));

    // SAFETY: sends a signal to the child this test spawned.
    assert_eq!(unsafe { libc::kill(child.id() as i32, signal) }, 0);
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "the tool did not stop");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(!rig.lock.exists(), "the profile lock was released");
    wait_for("the stand-in Chrome to be killed", || !running(chrome));
    status.code()
}

#[test]
fn ctrl_c_during_a_cli_run_closes_chrome_releases_the_profile_and_exits_130() {
    let rig = rig();
    let child = command(env!("CARGO_BIN_EXE_hltv-demo"), &rig)
        .args(["search", "--team", "9565"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    assert_eq!(interrupt(child, &rig, libc::SIGINT), Some(130));
}

#[test]
fn sigterm_during_a_cli_run_exits_143() {
    let rig = rig();
    let child = command(env!("CARGO_BIN_EXE_hltv-demo"), &rig)
        .args(["resolve", "/matches/2399723/vitality-vs-9z"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    assert_eq!(interrupt(child, &rig, libc::SIGTERM), Some(143));
}

#[test]
fn sigterm_during_an_mcp_tool_call_closes_chrome_and_releases_the_profile() {
    let rig = rig();
    let mut child = command(env!("CARGO_BIN_EXE_hltv-demo-mcp"), &rig)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for message in [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search_matches","arguments":{"team":9565}}}"#,
    ] {
        writeln!(stdin, "{message}").unwrap();
    }
    stdin.flush().unwrap();
    // Keep stdin open: a closed stdin is a disconnect, which is the other way the server stops.
    let code = interrupt(child, &rig, libc::SIGTERM);
    drop(stdin);
    assert_eq!(code, Some(143));
}
