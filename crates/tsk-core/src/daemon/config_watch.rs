//! Watch the tsk `config.toml` and signal a daemon restart when it changes.

use std::ffi::CString;
use std::fs;
use std::os::unix::io::RawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::config::config_text_is_valid;

const DEBOUNCE: Duration = Duration::from_millis(400);
const POLL: Duration = Duration::from_millis(250);

pub struct ConfigWatch {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ConfigWatch {
    pub fn start(path: PathBuf, shutdown: Arc<AtomicBool>, restart: Arc<AtomicBool>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let thread = thread::spawn(move || watch_loop(path, thread_stop, shutdown, restart));
        Self {
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for ConfigWatch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
struct Snapshot {
    contents: Option<String>,
}

fn read_snapshot(path: &Path) -> Snapshot {
    Snapshot {
        contents: fs::read_to_string(path).ok(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WatchDecision {
    Unchanged,
    Incomplete,
    Invalid(String),
    Restart,
}

fn evaluate_restart(path: &Path, snapshot: &Snapshot) -> WatchDecision {
    let contents = match fs::read_to_string(path) {
        Ok(raw) => Some(raw),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return WatchDecision::Invalid(err.to_string()),
    };
    if contents == snapshot.contents {
        return WatchDecision::Unchanged;
    }
    match contents {
        // Atomic saves briefly unlink the file; do not restart (or rewrite defaults).
        None => WatchDecision::Incomplete,
        Some(raw) => match config_text_is_valid(&raw) {
            Ok(()) => WatchDecision::Restart,
            Err(err) => WatchDecision::Invalid(err),
        },
    }
}

fn watch_loop(
    path: PathBuf,
    stop: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    restart: Arc<AtomicBool>,
) {
    let mut snapshot = read_snapshot(&path);
    let inotify = path.parent().and_then(InotifyWatch::open);

    if inotify.is_none() {
        eprintln!(
            "tsk daemon: inotify unavailable, polling {} for changes",
            path.display()
        );
    } else {
        eprintln!("tsk daemon watching {}", path.display());
    }

    let mut pending: Option<Instant> = None;
    let mut logged_invalid = false;
    let mut logged_missing = false;

    while !stop.load(Ordering::Relaxed)
        && !shutdown.load(Ordering::Relaxed)
        && !restart.load(Ordering::Relaxed)
    {
        let event = match &inotify {
            Some(watch) => watch.wait(POLL),
            None => {
                thread::sleep(POLL);
                false
            }
        };

        if event || snapshot_stale(&path, &snapshot) {
            pending.get_or_insert_with(Instant::now);
        }

        let Some(started) = pending else {
            continue;
        };
        if started.elapsed() < DEBOUNCE {
            continue;
        }
        pending = None;

        match evaluate_restart(&path, &snapshot) {
            WatchDecision::Restart => {
                eprintln!("tsk daemon: {} changed, restarting", path.display());
                restart.store(true, Ordering::SeqCst);
                return;
            }
            WatchDecision::Invalid(err) => {
                if !logged_invalid {
                    eprintln!(
                        "tsk daemon: {} is invalid, keeping current config: {err}",
                        path.display()
                    );
                    logged_invalid = true;
                }
                logged_missing = false;
            }
            WatchDecision::Incomplete => {
                if !logged_missing {
                    eprintln!(
                        "tsk daemon: {} missing, keeping current config",
                        path.display()
                    );
                    logged_missing = true;
                }
                logged_invalid = false;
            }
            WatchDecision::Unchanged => {
                snapshot = read_snapshot(&path);
                logged_invalid = false;
                logged_missing = false;
            }
        }
    }
}

fn snapshot_stale(path: &Path, snapshot: &Snapshot) -> bool {
    fs::read_to_string(path).ok() != snapshot.contents
}

struct InotifyWatch {
    fd: RawFd,
}

impl InotifyWatch {
    fn open(dir: &Path) -> Option<Self> {
        let c_path = CString::new(dir.to_str()?).ok()?;
        let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC | libc::IN_NONBLOCK) };
        if fd < 0 {
            return None;
        }
        let wd = unsafe {
            libc::inotify_add_watch(
                fd,
                c_path.as_ptr(),
                libc::IN_CREATE
                    | libc::IN_DELETE
                    | libc::IN_MODIFY
                    | libc::IN_MOVED_TO
                    | libc::IN_MOVED_FROM
                    | libc::IN_ATTRIB
                    | libc::IN_CLOSE_WRITE,
            )
        };
        if wd < 0 {
            unsafe { libc::close(fd) };
            return None;
        }
        Some(Self { fd })
    }

    fn wait(&self, timeout: Duration) -> bool {
        let mut pfd = libc::pollfd {
            fd: self.fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let rc = unsafe { libc::poll(&mut pfd, 1, timeout.as_millis() as i32) };
        if rc <= 0 {
            return false;
        }
        let mut buf = [0u8; 4096];
        loop {
            let n = unsafe { libc::read(self.fd, buf.as_mut_ptr().cast(), buf.len()) };
            if n <= 0 {
                break;
            }
        }
        true
    }
}

impl Drop for InotifyWatch {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.fd);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_toml(path: &Path, body: &str) {
        fs::write(path, body).unwrap();
    }

    #[test]
    fn unchanged_contents_do_not_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write_toml(&path, "workspace_count = 10\n");
        let snapshot = read_snapshot(&path);
        assert_eq!(evaluate_restart(&path, &snapshot), WatchDecision::Unchanged);
    }

    #[test]
    fn valid_edit_requests_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write_toml(&path, "[default]\nworkspace_count = 10\n");
        let snapshot = read_snapshot(&path);
        write_toml(&path, "[default]\nworkspace_count = 8\n");
        assert_eq!(evaluate_restart(&path, &snapshot), WatchDecision::Restart);
    }

    #[test]
    fn invalid_toml_keeps_running() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write_toml(&path, "[default]\nworkspace_count = 10\n");
        let snapshot = read_snapshot(&path);
        write_toml(&path, "[default\nworkspace_count = 8\n");
        assert!(matches!(
            evaluate_restart(&path, &snapshot),
            WatchDecision::Invalid(_)
        ));
    }

    #[test]
    fn missing_file_is_not_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write_toml(&path, "[default]\nworkspace_count = 10\n");
        let snapshot = read_snapshot(&path);
        fs::remove_file(&path).unwrap();
        assert_eq!(
            evaluate_restart(&path, &snapshot),
            WatchDecision::Incomplete
        );
    }

    #[test]
    fn type_error_is_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write_toml(&path, "[default]\nworkspace_count = 10\n");
        let snapshot = read_snapshot(&path);
        write_toml(&path, "[default]\nworkspace_count = \"nope\"\n");
        assert!(matches!(
            evaluate_restart(&path, &snapshot),
            WatchDecision::Invalid(_)
        ));
    }

    #[test]
    fn comment_only_change_still_restarts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write_toml(&path, "[default]\nworkspace_count = 10\n");
        let snapshot = read_snapshot(&path);
        write_toml(&path, "# tweak\n[default]\nworkspace_count = 10\n");
        assert_eq!(evaluate_restart(&path, &snapshot), WatchDecision::Restart);
    }

    #[test]
    fn watcher_sets_restart_flag_after_valid_edit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write_toml(&path, "[default]\nworkspace_count = 10\n");
        let shutdown = Arc::new(AtomicBool::new(false));
        let restart = Arc::new(AtomicBool::new(false));
        let watch = ConfigWatch::start(path.clone(), shutdown, restart.clone());
        thread::sleep(Duration::from_millis(80));
        write_toml(&path, "[default]\nworkspace_count = 8\n");
        let started = Instant::now();
        while !restart.load(Ordering::SeqCst) && started.elapsed() < Duration::from_secs(2) {
            thread::sleep(Duration::from_millis(40));
        }
        drop(watch);
        assert!(
            restart.load(Ordering::SeqCst),
            "watcher should request restart after config.toml changes"
        );
    }
}
