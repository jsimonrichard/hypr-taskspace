//! TSK control-plane daemon — single writer for `state.db`, Unix-socket RPC.

mod client;
mod config_watch;
mod server;

pub use client::{
    daemon_pid_path, daemon_request, daemon_socket_path, ensure_daemon, is_daemon_running,
    ping_daemon, ping_daemon_at, DaemonClient, DaemonResponse,
};
pub use server::{running_daemon_processes, stop_daemon, DaemonProcess, DaemonServer};
