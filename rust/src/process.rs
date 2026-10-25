//! Process-level concerns the two binaries share: logging to stderr and shutdown signals. The library
//! installs neither, so a program embedding it keeps its own.

use std::fmt;

/// Diagnostics on stderr when `RUST_LOG` asks for them (`RUST_LOG=hltv_demo=debug` shows launch,
/// arrival, and download timings). Without it nothing is printed, and stdout stays the result.
pub fn init_logging() {
    if std::env::var_os("RUST_LOG").is_none() {
        return;
    }
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .try_init();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    Interrupt,
    Terminate,
    Hangup,
}

impl Signal {
    /// The shell's convention: 128 plus the signal number.
    pub fn exit_code(self) -> u8 {
        match self {
            Self::Hangup => 129,
            Self::Interrupt => 130,
            Self::Terminate => 143,
        }
    }
}

impl fmt::Display for Signal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Interrupt => "SIGINT",
            Self::Terminate => "SIGTERM",
            Self::Hangup => "SIGHUP",
        })
    }
}

/// Resolves when the process is asked to stop. A binary that receives it drops its work, which
/// closes Chrome and releases the profile lock, as Playwright's handlers did for the TypeScript
/// version; without this a Ctrl-C would leave Chrome holding the profile.
#[cfg(unix)]
pub async fn shutdown() -> Signal {
    use tokio::signal::unix::{SignalKind, signal};
    let (Ok(mut interrupt), Ok(mut terminate), Ok(mut hangup)) = (
        signal(SignalKind::interrupt()),
        signal(SignalKind::terminate()),
        signal(SignalKind::hangup()),
    ) else {
        return std::future::pending().await;
    };
    tokio::select! {
        _ = interrupt.recv() => Signal::Interrupt,
        _ = terminate.recv() => Signal::Terminate,
        _ = hangup.recv() => Signal::Hangup,
    }
}

#[cfg(not(unix))]
pub async fn shutdown() -> Signal {
    match tokio::signal::ctrl_c().await {
        Ok(()) => Signal::Interrupt,
        Err(_) => std::future::pending().await,
    }
}
