//! Background daemon reachability probe — keeps the UI responsive.

use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;

use lae_core::is_daemon_running;

pub struct AsyncDaemonChecker {
    result_tx: Sender<bool>,
    result_rx: Receiver<bool>,
    in_flight: bool,
}

impl AsyncDaemonChecker {
    pub fn new() -> Self {
        let (result_tx, result_rx) = mpsc::channel();
        Self {
            result_tx,
            result_rx,
            in_flight: false,
        }
    }

    pub fn spawn_check(&mut self) {
        if self.in_flight {
            return;
        }
        self.in_flight = true;
        let tx = self.result_tx.clone();
        thread::spawn(move || {
            let _ = tx.send(is_daemon_running());
        });
    }

    /// Returns `Some(running)` when a background check has finished.
    pub fn poll(&mut self) -> Option<bool> {
        match self.result_rx.try_recv() {
            Ok(running) => {
                self.in_flight = false;
                Some(running)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.in_flight = false;
                None
            }
        }
    }
}
