//! Single-instance application support for Windows.
//!
//! This module ensures only one instance of the application runs at a time.
//! Secondary instances forward their file path to the primary instance over a
//! named pipe and then exit.
//!
//! The behavior can be toggled via the `single_instance` setting in config.ini.

use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Write};
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

use interprocess::local_socket::{prelude::*, GenericNamespaced, ListenerOptions, Stream};

use winapi::shared::minwindef::FALSE;
use winapi::shared::winerror::ERROR_ALREADY_EXISTS;
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::handleapi::CloseHandle;
use winapi::um::synchapi::CreateMutexW;
use winapi::um::winnt::HANDLE;

/// Unique name for the application mutex (prevents multiple instances).
const MUTEX_NAME: &str = "Global\\RustImageViewer_SingleInstance_A7F3B2C1";

/// Interprocess local socket name used to forward open-file requests.
/// On Windows this maps to a namespaced transport backed by named pipes.
const IPC_SOCKET_NAME: &str = "RustImageViewer_SingleInstance_A7F3B2C1.sock";
const IPC_WAKE_MESSAGE: &str = "WAKE";

/// Result of attempting to acquire the single-instance lock.
pub enum SingleInstanceResult {
    /// This is the first (primary) instance.
    Primary(SingleInstanceLock),
    /// Another instance is already running; the file path was sent to it.
    Secondary,
    /// Single-instance mode is disabled or an error occurred.
    Disabled,
}

/// RAII guard that holds the single-instance mutex.
/// When dropped, releases the mutex allowing another instance to become primary.
pub struct SingleInstanceLock {
    mutex_handle: HANDLE,
    shutdown_flag: Arc<AtomicBool>,
    listener_thread: Option<thread::JoinHandle<()>>,
}

// SAFETY: HANDLE is a raw pointer but we manage its lifetime carefully.
unsafe impl Send for SingleInstanceLock {}

impl Drop for SingleInstanceLock {
    fn drop(&mut self) {
        // Signal listener shutdown and wake blocked socket accept.
        self.shutdown_flag.store(true, Ordering::SeqCst);
        wake_pipe_listener();

        // Wait for listener thread to finish.
        if let Some(handle) = self.listener_thread.take() {
            let _ = handle.join();
        }

        // Release the mutex.
        if !self.mutex_handle.is_null() {
            unsafe {
                CloseHandle(self.mutex_handle);
            }
        }
    }
}

/// Convert a Rust string to a wide (UTF-16) null-terminated string for Windows API.
fn to_wide_null(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn send_message_to_primary(message: &str) -> bool {
    let name = match IPC_SOCKET_NAME.to_ns_name::<GenericNamespaced>() {
        Ok(name) => name,
        Err(_) => return false,
    };

    let mut stream = match Stream::connect(name) {
        Ok(stream) => stream,
        Err(_) => return false,
    };

    let mut payload = String::with_capacity(message.len() + 1);
    payload.push_str(message);
    payload.push('\n');

    if stream.write_all(payload.as_bytes()).is_err() {
        return false;
    }

    stream.flush().is_ok()
}

fn send_file_path_to_primary(path: &PathBuf) -> bool {
    let payload = format!("OPEN:{}", path.to_string_lossy());
    let sent = send_message_to_primary(&payload);
    if sent {
        tracing::debug!(target: "single_instance", path = %path.display(), "forwarded file path to primary");
    }
    sent
}

fn wake_pipe_listener() {
    let _ = send_message_to_primary(IPC_WAKE_MESSAGE);
}

/// Attempt to acquire the single-instance lock.
///
/// If `enabled` is false, returns `Disabled` immediately.
/// If this is the first instance, returns `Primary` with the lock guard.
/// If another instance exists, sends `file_path` to it and returns `Secondary`.
pub fn try_acquire_lock(
    enabled: bool,
    file_path: Option<&PathBuf>,
    on_file_received: impl Fn(PathBuf) + Send + 'static,
) -> SingleInstanceResult {
    if !enabled {
        return SingleInstanceResult::Disabled;
    }

    // Try to create/acquire the mutex.
    let mutex_name_wide = to_wide_null(MUTEX_NAME);
    let mutex_handle =
        unsafe { CreateMutexW(std::ptr::null_mut(), FALSE, mutex_name_wide.as_ptr()) };

    if mutex_handle.is_null() {
        return SingleInstanceResult::Disabled;
    }

    let last_error = unsafe { GetLastError() };

    if last_error == ERROR_ALREADY_EXISTS {
        // Another instance already has the mutex - we are secondary.
        unsafe {
            CloseHandle(mutex_handle);
        }

        if let Some(path) = file_path {
            let _ = send_file_path_to_primary(path);
        }

        return SingleInstanceResult::Secondary;
    }

    // We are the primary instance.
    tracing::debug!(target: "single_instance", "primary instance listener starting");
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let shutdown_flag_clone = Arc::clone(&shutdown_flag);

    let listener_thread = thread::spawn(move || {
        run_socket_listener(shutdown_flag_clone, on_file_received);
    });

    SingleInstanceResult::Primary(SingleInstanceLock {
        mutex_handle,
        shutdown_flag,
        listener_thread: Some(listener_thread),
    })
}

/// Listen for local-socket messages and call callback when a file path is received.
fn run_socket_listener(
    shutdown_flag: Arc<AtomicBool>,
    on_file_received: impl Fn(PathBuf) + Send + 'static,
) {
    let name = match IPC_SOCKET_NAME.to_ns_name::<GenericNamespaced>() {
        Ok(name) => name,
        Err(_) => return,
    };

    let listener = match ListenerOptions::new().name(name).create_sync() {
        Ok(listener) => listener,
        Err(_) => return,
    };

    for conn_result in listener.incoming() {
        if shutdown_flag.load(Ordering::SeqCst) {
            break;
        }

        let conn = match conn_result {
            Ok(conn) => conn,
            Err(_) => continue,
        };

        let mut reader = BufReader::new(conn);
        let mut message = String::new();
        if reader.read_line(&mut message).is_err() {
            continue;
        }

        let message = message.trim_end_matches(&['\r', '\n'][..]);
        if message == IPC_WAKE_MESSAGE {
            if shutdown_flag.load(Ordering::SeqCst) {
                break;
            }
        } else if let Some(path_str) = message.strip_prefix("OPEN:") {
            let path = PathBuf::from(path_str.trim());
            if path.exists() {
                tracing::debug!(target: "single_instance", path = %path.display(), "received open request from secondary instance");
                on_file_received(path);
            }
        }
    }
}

/// Channel-based file receiver for use with egui's event loop.
pub struct FileReceiver {
    receiver: crossbeam_channel::Receiver<PathBuf>,
}

impl FileReceiver {
    /// Create a new file receiver and return the sender callback.
    pub fn new() -> (Self, impl Fn(PathBuf) + Send + 'static) {
        let (sender, receiver) = crossbeam_channel::unbounded();
        let callback = move |path: PathBuf| {
            let _ = sender.send(path);
        };
        (FileReceiver { receiver }, callback)
    }

    /// Try to receive a file path without blocking.
    pub fn try_recv(&self) -> Option<PathBuf> {
        self.receiver.try_recv().ok()
    }
}
