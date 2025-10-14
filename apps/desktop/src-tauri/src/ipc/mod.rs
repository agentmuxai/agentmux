// IPC (Inter-Process Communication) module for single-instance mode
//
// This module enables CLI commands to communicate with a running GUI instance
// by sending HTTP requests to a local IPC server.

pub mod lock;
pub mod server;
pub mod client;
pub mod protocol;

pub use lock::{LockFile, read_lock_file, write_lock_file, remove_lock_file, is_lock_stale};
pub use server::start_ipc_server;
pub use client::send_ipc_command;
pub use protocol::{IpcCommand, IpcResponse};
