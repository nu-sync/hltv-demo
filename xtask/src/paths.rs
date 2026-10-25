use std::path::{Path, PathBuf};
use std::process::Command;

/// The hltv-demo crate root: the folder holding Cargo.toml, src/, and tests/.
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives inside the crate")
        .to_path_buf()
}

/// Build the two binaries, then the folder they are in: the profile folder this xtask was built into,
/// so the binaries checked are the ones the source tree produces right now.
pub fn binaries() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot locate xtask: {e}"))?;
    let dir = exe.parent().ok_or("xtask has no folder")?.to_path_buf();
    let mut build = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    build
        .current_dir(root())
        .args(["build", "--quiet", "--package", "hltv-demo", "--bins"]);
    if dir.file_name().is_some_and(|name| name == "release") {
        build.arg("--release");
    }
    let status = build
        .status()
        .map_err(|e| format!("cannot run cargo: {e}"))?;
    if !status.success() {
        return Err(format!("cargo build exited {status}"));
    }
    Ok(dir)
}

/// The first `name` on PATH.
pub fn which(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
}
