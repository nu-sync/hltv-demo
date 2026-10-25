//! One process may drive the shared Chrome profile at a time. A second one would either fail to
//! launch or attach to the first browser, and then downloads and pages cross over. The lock is a file
//! holding a pid, and a pid that no longer runs is treated as a crash, not a holder. The file and its
//! format are the TypeScript version's, so the two implementations exclude each other too.

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

/// The held lock. Dropping it releases the profile, however the run ends.
#[derive(Debug)]
pub struct Lock {
    path: PathBuf,
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[derive(Debug)]
pub enum Acquired {
    Held(Lock),
    Busy { pid: Option<u32> },
}

pub fn acquire(path: &Path) -> std::io::Result<Acquired> {
    // On a first run the profile's folder does not exist yet, and the lock is the first thing to need it.
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    for attempt in 0..2 {
        match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(mut file) => {
                file.write_all(std::process::id().to_string().as_bytes())?;
                return Ok(Acquired::Held(Lock {
                    path: path.to_path_buf(),
                }));
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                let holder = fs::read_to_string(path)
                    .ok()
                    .and_then(|s| s.trim().parse::<u32>().ok());
                match holder {
                    Some(pid) if attempt == 0 && pid > 0 && !is_running(pid) => {
                        let _ = fs::remove_file(path);
                    }
                    _ => return Ok(Acquired::Busy { pid: holder }),
                }
            }
            Err(error) => return Err(error),
        }
    }
    Ok(Acquired::Busy { pid: None })
}

/// The pid of a running Chrome that already has `profile` open, read from Chrome's own
/// `SingletonLock` (a symlink to `<host>-<pid>`). This catches the window a person opened to answer
/// the challenge, which a second Chrome cannot share. A link left by a crash is not a holder.
pub fn chrome_holder(profile: &Path) -> Option<u32> {
    let target = fs::read_link(profile.join("SingletonLock")).ok()?;
    let (_, pid) = target.to_str()?.rsplit_once('-')?;
    let pid = pid.parse::<u32>().ok().filter(|&pid| pid > 0)?;
    is_running(pid).then_some(pid)
}

#[cfg(unix)]
fn is_running(pid: u32) -> bool {
    // SAFETY: signal 0 performs only the existence and permission check.
    let alive = unsafe { libc::kill(pid as libc::pid_t, 0) } == 0;
    alive || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Without a way to ask, every recorded holder counts as running: a false "busy" is safe, a shared
/// profile is not.
#[cfg(not(unix))]
fn is_running(_pid: u32) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_holder_then_busy_until_it_drops() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profile.lock");
        let Acquired::Held(lock) = acquire(&path).unwrap() else {
            panic!("first acquire")
        };
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            std::process::id().to_string()
        );
        assert!(
            matches!(acquire(&path).unwrap(), Acquired::Busy { pid: Some(pid) } if pid == std::process::id())
        );
        drop(lock);
        assert!(matches!(acquire(&path).unwrap(), Acquired::Held(_)));
    }

    #[test]
    fn a_first_run_whose_profile_folder_does_not_exist_gets_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fresh").join("profile.lock");
        assert!(matches!(acquire(&path).unwrap(), Acquired::Held(_)));
    }

    #[test]
    fn a_dead_holder_is_a_crash_not_a_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profile.lock");
        fs::write(&path, "999999999").unwrap();
        assert!(matches!(acquire(&path).unwrap(), Acquired::Held(_)));
    }

    #[cfg(unix)]
    #[test]
    fn chrome_holding_the_profile_is_found_and_a_stale_link_is_not() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(chrome_holder(dir.path()), None);
        let link = dir.path().join("SingletonLock");
        std::os::unix::fs::symlink(format!("host.local-{}", std::process::id()), &link).unwrap();
        assert_eq!(chrome_holder(dir.path()), Some(std::process::id()));
        fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink("host.local-999999999", &link).unwrap();
        assert_eq!(chrome_holder(dir.path()), None);
    }
}
