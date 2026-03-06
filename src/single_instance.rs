//! Single-instance application support for Windows.
//!
//! This module ensures only one instance of the application runs at a time.
//! Secondary instances forward their file path to the primary instance over a
//! named pipe and then exit.
//!
//! The behavior can be toggled via the `single_instance` setting in config.ini.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use winapi::shared::minwindef::{DWORD, FALSE};
use winapi::shared::winerror::{ERROR_ALREADY_EXISTS, ERROR_PIPE_CONNECTED};
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::fileapi::{CreateFileW, ReadFile, WriteFile, OPEN_EXISTING};
use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
use winapi::um::namedpipeapi::{ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe};
use winapi::um::synchapi::CreateMutexW;
use winapi::um::winbase::{
    PIPE_ACCESS_INBOUND, PIPE_READMODE_MESSAGE, PIPE_TYPE_MESSAGE, PIPE_UNLIMITED_INSTANCES,
    PIPE_WAIT,
};
use winapi::um::winnt::{GENERIC_WRITE, HANDLE};

/// Unique name for the application mutex (prevents multiple instances).
const MUTEX_NAME: &str = "Global\\RustImageViewer_SingleInstance_A7F3B2C1";

/// Named pipe used to forward open-file requests from secondary instances.
const IPC_PIPE_NAME: &str = r"\\.\pipe\RustImageViewer_SingleInstance_A7F3B2C1";
const IPC_BUFFER_SIZE: usize = 4096;
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
        // Signal listener shutdown and wake blocked ConnectNamedPipe.
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

fn create_server_pipe() -> HANDLE {
    let pipe_name = to_wide_null(IPC_PIPE_NAME);
    unsafe {
        CreateNamedPipeW(
            pipe_name.as_ptr(),
            PIPE_ACCESS_INBOUND,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
            PIPE_UNLIMITED_INSTANCES,
            0,
            IPC_BUFFER_SIZE as u32,
            0,
            std::ptr::null_mut(),
        )
    }
}

fn send_message_to_primary(message: &str) -> bool {
    let pipe_name = to_wide_null(IPC_PIPE_NAME);
    let handle = unsafe {
        CreateFileW(
            pipe_name.as_ptr(),
            GENERIC_WRITE,
            0,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };

    if handle == INVALID_HANDLE_VALUE {
        return false;
    }

    let bytes = message.as_bytes();
    let mut written: DWORD = 0;
    let ok = unsafe {
        WriteFile(
            handle,
            bytes.as_ptr() as *const _,
            bytes.len() as u32,
            &mut written,
            std::ptr::null_mut(),
        ) != 0
    };

    unsafe {
        CloseHandle(handle);
    }

    ok && written > 0
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
        run_pipe_listener(shutdown_flag_clone, on_file_received);
    });

    SingleInstanceResult::Primary(SingleInstanceLock {
        mutex_handle,
        shutdown_flag,
        listener_thread: Some(listener_thread),
    })
}

/// Listen for pipe messages and call callback when a file path is received.
fn run_pipe_listener(
    shutdown_flag: Arc<AtomicBool>,
    on_file_received: impl Fn(PathBuf) + Send + 'static,
) {
    loop {
        if shutdown_flag.load(Ordering::SeqCst) {
            break;
        }

        let pipe = create_server_pipe();
        if pipe == INVALID_HANDLE_VALUE {
            thread::sleep(Duration::from_millis(10));
            continue;
        }

        let connected = unsafe {
            let ok = ConnectNamedPipe(pipe, std::ptr::null_mut());
            if ok != 0 {
                true
            } else {
                GetLastError() == ERROR_PIPE_CONNECTED
            }
        };

        if connected {
            let mut buffer = vec![0u8; IPC_BUFFER_SIZE];
            let mut bytes_read: DWORD = 0;

            let read_ok = unsafe {
                ReadFile(
                    pipe,
                    buffer.as_mut_ptr() as *mut _,
                    buffer.len() as u32,
                    &mut bytes_read,
                    std::ptr::null_mut(),
                ) != 0
            };

            if read_ok && bytes_read > 0 {
                buffer.truncate(bytes_read as usize);

                if let Ok(message) = String::from_utf8(buffer) {
                    if message == IPC_WAKE_MESSAGE {
                        // Internal wake-up to break blocking ConnectNamedPipe during shutdown.
                    } else if let Some(path_str) = message.strip_prefix("OPEN:") {
                        let path = PathBuf::from(path_str.trim());
                        if path.exists() {
                            tracing::debug!(target: "single_instance", path = %path.display(), "received open request from secondary instance");
                            on_file_received(path);
                        }
                    }
                }
            }

            unsafe {
                DisconnectNamedPipe(pipe);
            }
        }

        unsafe {
            CloseHandle(pipe);
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
