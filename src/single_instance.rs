//! Single-instance application support for Windows.
//!
//! This module provides functionality to ensure only one instance of the application
//! runs at a time. When a second instance is launched, it sends its file path to the
//! first instance via a temp file, then exits. The first instance receives the path
//! and opens the file in its existing window.
//!
//! The behavior can be toggled via the `single_instance` setting in config.ini.

use std::ffi::OsStr;
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use winapi::shared::minwindef::FALSE;
use winapi::shared::winerror::ERROR_ALREADY_EXISTS;
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::handleapi::CloseHandle;
use winapi::um::synchapi::CreateMutexW;
use winapi::um::winnt::HANDLE;

/// Unique name for the application mutex (prevents multiple instances).
const MUTEX_NAME: &str = "Global\\RustImageViewer_SingleInstance_A7F3B2C1";

/// Get the path to the IPC file used for communication between instances.
fn get_ipc_file_path() -> PathBuf {
    std::env::temp_dir().join("rust_image_viewer_ipc.txt")
}

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
        // Signal the listener thread to stop
        self.shutdown_flag.store(true, Ordering::SeqCst);

        // Wait for listener thread to finish
        if let Some(handle) = self.listener_thread.take() {
            let _ = handle.join();
        }

        // Release the mutex
        if !self.mutex_handle.is_null() {
            unsafe {
                CloseHandle(self.mutex_handle);
            }
        }

        // Clean up IPC file
        let _ = fs::remove_file(get_ipc_file_path());
    }
}

/// Convert a Rust string to a wide (UTF-16) null-terminated string for Windows API.
fn to_wide_null(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
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

    // Try to create/acquire the mutex
    let mutex_name_wide = to_wide_null(MUTEX_NAME);
    let mutex_handle =
        unsafe { CreateMutexW(std::ptr::null_mut(), FALSE, mutex_name_wide.as_ptr()) };

    if mutex_handle.is_null() {
        return SingleInstanceResult::Disabled;
    }

    let last_error = unsafe { GetLastError() };

    if last_error == ERROR_ALREADY_EXISTS {
        // Another instance already has the mutex - we are secondary
        unsafe {
            CloseHandle(mutex_handle);
        }

        // Send our file path to the primary instance via temp file
        if let Some(path) = file_path {
            let ipc_path = get_ipc_file_path();
            // Write with a unique marker to avoid partial reads
            let content = format!("OPEN:{}", path.to_string_lossy());
            let _ = fs::write(&ipc_path, content);
        }

        return SingleInstanceResult::Secondary;
    }

    // We are the primary instance
    // Clean up any stale IPC file from a previous crash
    let _ = fs::remove_file(get_ipc_file_path());

    // Start the file watcher thread
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let shutdown_flag_clone = Arc::clone(&shutdown_flag);

    let listener_thread = thread::spawn(move || {
        run_file_watcher(shutdown_flag_clone, on_file_received);
    });

    SingleInstanceResult::Primary(SingleInstanceLock {
        mutex_handle,
        shutdown_flag,
        listener_thread: Some(listener_thread),
    })
}

/// Watch for IPC file and call callback when a file path is received.
fn run_file_watcher(
    shutdown_flag: Arc<AtomicBool>,
    on_file_received: impl Fn(PathBuf) + Send + 'static,
) {
    let ipc_path = get_ipc_file_path();

    loop {
        if shutdown_flag.load(Ordering::SeqCst) {
            break;
        }

        // Check if IPC file exists
        if ipc_path.exists() {
            // Read and process the file
            if let Ok(content) = fs::read_to_string(&ipc_path) {
                // Delete the file immediately to avoid double-processing
                let _ = fs::remove_file(&ipc_path);

                // Parse the content
                if let Some(path_str) = content.strip_prefix("OPEN:") {
                    let path = PathBuf::from(path_str);
                    if path.exists() {
                        on_file_received(path);
                    }
                }
            }
        }

        // Poll every 50ms
        thread::sleep(Duration::from_millis(50));
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
