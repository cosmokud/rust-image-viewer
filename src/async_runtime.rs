//! Shared Tokio runtime used by background workers across the app.
//!
//! The GUI stays synchronous (eframe/egui), but heavy I/O and CPU-bound blocking
//! work can be dispatched through this runtime via `spawn_blocking`.

use std::future::Future;
use std::sync::OnceLock;

static TOKIO_RUNTIME: OnceLock<Option<tokio::runtime::Runtime>> = OnceLock::new();

fn build_runtime() -> Option<tokio::runtime::Runtime> {
    let worker_threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .max(2)
        .min(16);

    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(worker_threads)
        .max_blocking_threads(worker_threads.saturating_mul(4).max(8))
        .thread_name("riv-tokio")
        .enable_time()
        .enable_io()
        .build()
        .ok()
}

fn runtime() -> Option<&'static tokio::runtime::Runtime> {
    TOKIO_RUNTIME.get_or_init(build_runtime).as_ref()
}

/// Ensure the global runtime is initialized.
pub fn init_runtime() -> bool {
    runtime().is_some()
}

/// Spawn a blocking task on the shared runtime.
#[allow(dead_code)]
pub fn spawn_blocking<F, R>(job: F) -> Option<tokio::task::JoinHandle<R>>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let runtime = runtime()?;
    Some(runtime.spawn_blocking(job))
}

/// Spawn an async future on the shared runtime.
#[allow(dead_code)]
pub fn spawn<F>(future: F) -> Option<tokio::task::JoinHandle<F::Output>>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let runtime = runtime()?;
    Some(runtime.spawn(future))
}

/// Spawn a background worker; fall back to an OS thread when Tokio is unavailable.
pub fn spawn_blocking_or_thread<F>(thread_name: &str, job: F)
where
    F: FnOnce() + Send + 'static,
{
    let mut job = Some(job);

    if let Some(runtime) = runtime() {
        if let Some(task) = job.take() {
            let _ = runtime.spawn_blocking(task);
            return;
        }
    }

    if let Some(task) = job.take() {
        let _ = std::thread::Builder::new()
            .name(thread_name.to_string())
            .spawn(task);
    }
}
