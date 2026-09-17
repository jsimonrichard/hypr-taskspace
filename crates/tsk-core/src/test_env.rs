//! Shared process-environment lock for tests.
//!
//! Tests that mutate `HOME`, XDG_*, `TSK_*`, or `PATH` must hold this lock so they
//! cannot race. Drop restores the named keys to the values they had at lock time.

use std::env;
use std::ffi::OsString;
use std::sync::{Mutex, MutexGuard};

static LOCK: Mutex<()> = Mutex::new(());

pub struct EnvLock {
    _guard: MutexGuard<'static, ()>,
    saved: Vec<(String, Option<OsString>)>,
}

/// Lock process env and restore `keys` when the guard is dropped.
pub fn lock(keys: &[&str]) -> EnvLock {
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let saved = keys
        .iter()
        .map(|k| ((*k).to_string(), env::var_os(k)))
        .collect();
    EnvLock {
        _guard: guard,
        saved,
    }
}

impl Drop for EnvLock {
    fn drop(&mut self) {
        for (key, value) in &self.saved {
            match value {
                Some(v) => env::set_var(key, v),
                None => env::remove_var(key),
            }
        }
    }
}
