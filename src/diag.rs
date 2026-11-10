//! Opt-in freeze/crash forensics. Disabled sessions pay one cached bool load per call site.
//!
//! Env (PowerShell: `$env:RIV_DIAG=1`; cmd: `set RIV_DIAG=1`):
//!   `RIV_DIAG=1`       enable the sink below.
//!   `RIV_MANGA_TRACE=1` alias with the same sink (its heal-rule lines land here too).
//!   `RIV_DIAG_DIR`     log directory (default `<temp>/rust-image-viewer`).
//!   `RIV_DIAG_FRAME_MS` slow-frame threshold in ms (default 16).
//!   `RIV_DIAG_HANG_MS`  watchdog stall threshold in ms (default 2000).
//!
//! Files: `diag-<pid>-<unix_ts>.log`, one flush per line so the tail survives an abort
//! (`panic = "abort"` in Cargo.toml), 32 MB cap per file, 3 newest files kept.
//!
//! Not captured: process thread count and peak RSS. Both need platform calls outside std, and
//! a diagnostic that pulls in winapi features is a diagnostic nobody enables by default.
//! shortcut: no thread/RSS snapshot, add GetProcessMemoryInfo if hangs need process-level data.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Per-file ceiling; past it the file is truncated and a boundary line is written.
const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
const KEEP_FILES: usize = 3;
const HANG_TICK: Duration = Duration::from_millis(250);
/// A stuck process repeats its snapshot at most this often: a hung frame must not fill the disk.
const HANG_REPEAT: Duration = Duration::from_secs(2);
const DEFAULT_FRAME_MS: u64 = 16;
const DEFAULT_HANG_MS: u64 = 2000;
const SUMMARY_INTERVAL: Duration = Duration::from_secs(1);

static ENABLED: OnceLock<bool> = OnceLock::new();
static ORIGIN: OnceLock<Instant> = OnceLock::new();
static HEARTBEAT: AtomicU64 = AtomicU64::new(0);
static FRAME_STARTED_MS: AtomicU64 = AtomicU64::new(0);
static SUMMARY_MS: AtomicU64 = AtomicU64::new(u64::MAX);
static STAGE: Mutex<&'static str> = Mutex::new("?");

/// Aggregated load-path counters, printed once per second and in a hang snapshot.
#[derive(Clone, Copy, Default, Debug)]
struct Counters {
    frames: u64,
    slow_frames: u64,
    gst_probes: u64,
    probe_waits: u64,
    probe_timeouts: u64,
    dim_attempts: u64,
    dim_chunks: u64,
    dim_parked: u64,
    dim_flushed: u64,
    dim_dropped: u64,
}

/// What the per-second summary and a hang snapshot aggregate.
#[derive(Clone, Copy, Debug)]
pub enum Counter {
    Frames,
    SlowFrames,
    GstProbes,
    ProbeWaits,
    ProbeTimeouts,
    DimAttempts,
    DimChunks,
    DimParked,
    DimFlushed,
    DimDropped,
}

static COUNTERS: Mutex<Counters> = Mutex::new(Counters {
    frames: 0,
    slow_frames: 0,
    gst_probes: 0,
    probe_waits: 0,
    probe_timeouts: 0,
    dim_attempts: 0,
    dim_chunks: 0,
    dim_parked: 0,
    dim_flushed: 0,
    dim_dropped: 0,
});

/// The most recent frame, for the watchdog: it runs while the UI thread is stuck.
static LAST_FRAME: Mutex<Option<String>> = Mutex::new(None);

struct Sink {
    file: std::fs::File,
    written: u64,
    path: PathBuf,
}

static SINK: OnceLock<Option<Mutex<Sink>>> = OnceLock::new();

/// One frame's worth of trace fields, filled by the update loop.
pub struct FrameStats {
    pub total: Duration,
    pub repaint: &'static str,
    pub visible: usize,
    pub uploads: usize,
    pub backlog: (usize, usize, usize, usize),
    pub inflight: usize,
    pub retry: usize,
    pub ttv_pending: usize,
}

/// Cached: `false` unless one of the enabling flags is set.
pub fn enabled() -> bool {
    *ENABLED.get_or_init(|| env_flag("RIV_DIAG") || env_flag("RIV_MANGA_TRACE"))
}

fn env_flag(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| value == "1")
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var_os(name)
        .and_then(|value| value.to_str().and_then(|text| text.trim().parse().ok()))
        .unwrap_or(default)
}

fn frame_threshold() -> Duration {
    Duration::from_millis(env_u64("RIV_DIAG_FRAME_MS", DEFAULT_FRAME_MS))
}

fn hang_threshold() -> Duration {
    Duration::from_millis(env_u64("RIV_DIAG_HANG_MS", DEFAULT_HANG_MS))
}

fn locked<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}

fn origin() -> Instant {
    *ORIGIN.get_or_init(Instant::now)
}

/// Install the sink, panic hook and watchdog. No-op when disabled.
pub fn init() {
    if !enabled() {
        return;
    }

    let dir = log_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(log_file_name(std::process::id(), unix_seconds()));
    match open_sink(&path) {
        Ok(sink) => {
            let _ = SINK.set(Some(Mutex::new(sink)));
            prune_old_logs(&dir, KEEP_FILES);
            *locked(&STAGE) = "start";
            write_line(&format!(
                "diag start pid={} log={}",
                std::process::id(),
                path.display()
            ));
        }
        Err(_) => {
            let _ = SINK.set(None);
        }
    }

    install_panic_hook();
    spawn_watchdog();
}

fn log_dir() -> PathBuf {
    match std::env::var_os("RIV_DIAG_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => std::env::temp_dir().join("rust-image-viewer"),
    }
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

fn log_file_name(pid: u32, unix_ts: u64) -> String {
    format!("diag-{pid}-{unix_ts}.log")
}

/// Sort key embedded in the file name; 0 for names this module did not write.
fn log_timestamp(name: &str) -> Option<u64> {
    let rest = name.strip_prefix("diag-")?.strip_suffix(".log")?;
    let (_, stamp) = rest.split_once('-')?;
    stamp.parse().ok()
}

fn truncate_file(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
}

fn open_sink(path: &Path) -> std::io::Result<Sink> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let written = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    Ok(Sink {
        file,
        written,
        path: path.to_path_buf(),
    })
}

/// Delete every log but the `keep` newest. Fails silently: rotation is not worth a panic.
fn prune_old_logs(dir: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    let mut logs: Vec<(u64, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            log_timestamp(&name).map(|stamp| (stamp, entry.path()))
        })
        .collect();
    logs.sort_unstable_by(|a, b| b.0.cmp(&a.0));

    for (_, path) in logs.into_iter().skip(keep) {
        let _ = std::fs::remove_file(path);
    }
}

/// Append one line and flush it. Never panics: a broken sink must not break the app.
pub fn write_line(line: &str) {
    if !enabled() {
        return;
    }

    let Some(sink) = SINK.get().and_then(|sink| sink.as_ref()) else {
        return;
    };

    let elapsed_ms = origin().elapsed().as_millis();
    let mut sink = locked(sink);
    if sink.written >= MAX_FILE_BYTES {
        // Truncate in place: the newest file must stay the one an operator has open.
        if let Ok(file) = truncate_file(&sink.path) {
            sink.file = file;
            sink.written = 0;
        }
        let _ = writeln!(sink.file, "[{elapsed_ms:>9}ms] log truncated at cap");
    }

    let written =
        writeln!(sink.file, "[{elapsed_ms:>9}ms] {line}").and_then(|()| sink.file.flush());
    if written.is_ok() {
        sink.written += line.len() as u64 + 16;
    }
}

/// Tagged load-path line; returns `false` when diag is off so a caller may fall back to stderr.
pub fn trace(tag: &str, message: &str) -> bool {
    if !enabled() {
        return false;
    }

    write_line(&format!("[{}] {message}", tag));
    true
}

pub fn count(counter: Counter, delta: u64) {
    if !enabled() {
        return;
    }

    let mut counters = locked(&COUNTERS);
    let slot = match counter {
        Counter::Frames => &mut counters.frames,
        Counter::SlowFrames => &mut counters.slow_frames,
        Counter::GstProbes => &mut counters.gst_probes,
        Counter::ProbeWaits => &mut counters.probe_waits,
        Counter::ProbeTimeouts => &mut counters.probe_timeouts,
        Counter::DimAttempts => &mut counters.dim_attempts,
        Counter::DimChunks => &mut counters.dim_chunks,
        Counter::DimParked => &mut counters.dim_parked,
        Counter::DimFlushed => &mut counters.dim_flushed,
        Counter::DimDropped => &mut counters.dim_dropped,
    };
    *slot = slot.saturating_add(delta);
}

/// Name the stage the frame is in; the watchdog prints the last one it saw.
pub fn note_stage(name: &'static str) {
    if !enabled() {
        return;
    }
    *locked(&STAGE) = name;
}

/// Heartbeat for the watchdog: one bump per completed frame.
pub fn beat() {
    if !enabled() {
        return;
    }
    HEARTBEAT.fetch_add(1, Ordering::Relaxed);
}

/// Trace one frame. Slow frames and one summary line per second; otherwise nothing.
pub fn frame(stats: &FrameStats) {
    if !enabled() {
        return;
    }

    beat();
    count(Counter::Frames, 1);
    let total_ms = stats.total.as_secs_f64() * 1000.0;
    let line = format!(
        "frame {total_ms:.1}ms stage={} repaint={} visible={} uploads={} backlog=loads:{} dec:{} dim:{} probes:{} inflight={} retry={} ttv={}",
        locked(&STAGE).to_string(),
        stats.repaint,
        stats.visible,
        stats.uploads,
        stats.backlog.0,
        stats.backlog.1,
        stats.backlog.2,
        stats.backlog.3,
        stats.inflight,
        stats.retry,
        stats.ttv_pending,
    );
    *locked(&LAST_FRAME) = Some(line.clone());

    if stats.total >= frame_threshold() {
        count(Counter::SlowFrames, 1);
        write_line(&format!("SLOW {line}"));
    }

    let elapsed_ms = origin().elapsed().as_millis() as u64;
    let last = SUMMARY_MS.load(Ordering::Relaxed);
    let due =
        last == u64::MAX || elapsed_ms.saturating_sub(last) >= SUMMARY_INTERVAL.as_millis() as u64;
    if due {
        SUMMARY_MS.store(elapsed_ms, Ordering::Relaxed);
        let mut counters = locked(&COUNTERS);
        let spent = *counters;
        *counters = Counters::default();
        write_line(&format!(
            "summary 1s frames={} slow={} gst_probes={} probe_waits={} probe_timeouts={} dim_attempts={} dim_chunks={} dim_parked={} dim_flushed={} dim_dropped={}",
            spent.frames,
            spent.slow_frames,
            spent.gst_probes,
            spent.probe_waits,
            spent.probe_timeouts,
            spent.dim_attempts,
            spent.dim_chunks,
            spent.dim_parked,
            spent.dim_flushed,
            spent.dim_dropped,
        ));
    }
}

fn stage_snapshot() -> String {
    locked(&STAGE).to_string()
}

fn counters_snapshot() -> String {
    let counters = *locked(&COUNTERS);
    format!(
        "frames={} slow={} gst_probes={} probe_waits={} probe_timeouts={} dim_attempts={} dim_chunks={} dim_parked={} dim_flushed={} dim_dropped={}",
        counters.frames,
        counters.slow_frames,
        counters.gst_probes,
        counters.probe_waits,
        counters.probe_timeouts,
        counters.dim_attempts,
        counters.dim_chunks,
        counters.dim_parked,
        counters.dim_flushed,
        counters.dim_dropped,
    )
}

/// Dump what the stuck frame was doing. Called from the watchdog thread only.
fn dump_hang(stalled_ms: u128) {
    let stage = stage_snapshot();
    write_line(&format!(
        "HANG no frame completed for {stalled_ms}ms; last stage={stage} since_frame_start={}ms",
        origin()
            .elapsed()
            .as_millis()
            .saturating_sub(FRAME_STARTED_MS.load(Ordering::Relaxed) as u128)
    ));
    write_line(&format!("HANG counters {}", counters_snapshot()));
    if let Some(frame) = locked(&LAST_FRAME).clone() {
        write_line(&format!("HANG last {frame}"));
    }
}

/// Bumped by the frame loop so the watchdog can time the current frame from its start.
pub fn note_frame_start() {
    if !enabled() {
        return;
    }
    FRAME_STARTED_MS.store(origin().elapsed().as_millis() as u64, Ordering::Relaxed);
}

fn spawn_watchdog() {
    let spawned = std::thread::Builder::new()
        .name("diag-watchdog".to_string())
        .spawn(|| {
            let limit = hang_threshold();
            let mut last_beat = HEARTBEAT.load(Ordering::Relaxed);
            let mut stalled = Duration::ZERO;
            let mut dumped_at: Option<Instant> = None;

            loop {
                std::thread::sleep(HANG_TICK);
                let beat = HEARTBEAT.load(Ordering::Relaxed);
                if beat != last_beat {
                    last_beat = beat;
                    stalled = Duration::ZERO;
                    dumped_at = None;
                    continue;
                }

                stalled += HANG_TICK;
                if stalled < limit {
                    continue;
                }
                if dumped_at.is_some_and(|at| at.elapsed() < HANG_REPEAT) {
                    continue;
                }

                dumped_at = Some(Instant::now());
                dump_hang(stalled.as_millis());
            }
        });

    if spawned.is_err() {
        write_line("diag watchdog thread failed to start");
    }
}

/// Panic message, location and backtrace into the same file, then the default hook.
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if enabled() {
            let payload = info
                .payload()
                .downcast_ref::<&str>()
                .map(|text| (*text).to_string())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "<non-string panic payload>".to_string());
            let location = info
                .location()
                .map(|location| location.to_string())
                .unwrap_or_else(|| "<unknown>".to_string());
            // Flushed before this returns: `panic = "abort"` kills the process right after.
            write_line(&format!(
                "PANIC {payload}\nPANIC at {location}\n{}",
                std::backtrace::Backtrace::force_capture()
            ));
        }

        default_hook(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_name_carries_pid_and_timestamp() {
        let name = log_file_name(4242, 1_700_000_000);
        assert_eq!(name, "diag-4242-1700000000.log");
        assert_eq!(log_timestamp(&name), Some(1_700_000_000));
    }

    #[test]
    fn foreign_file_names_are_ignored_by_rotation() {
        assert_eq!(log_timestamp("notes.txt"), None);
        assert_eq!(log_timestamp("diag-1.log"), None);
        assert_eq!(log_timestamp("diag-1-abc.log"), None);
        assert_eq!(log_timestamp("diag-7-12.log"), Some(12));
    }

    #[test]
    fn rotation_keeps_the_newest_files() {
        let dir = std::env::temp_dir().join(format!("riv-diag-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for stamp in [10_u64, 30, 20, 40] {
            std::fs::write(dir.join(log_file_name(1, stamp)), b"x").unwrap();
        }
        std::fs::write(dir.join("unrelated.txt"), b"x").unwrap();

        prune_old_logs(&dir, 3);

        let mut left: Vec<u64> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter_map(|entry| log_timestamp(&entry.file_name().to_string_lossy()))
            .collect();
        left.sort_unstable();
        assert_eq!(left, vec![20, 30, 40]);
        assert!(dir.join("unrelated.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn disabled_module_pays_one_cached_flag() {
        // No RIV_DIAG in the test environment: the flag resolves once and every entry point
        // short-circuits before touching the sink.
        assert!(!enabled());
        assert!(!trace("probe", "should not be written"));
        count(Counter::Frames, 1);
        note_stage("test");
        note_frame_start();
        beat();
        frame(&FrameStats {
            total: Duration::from_millis(50),
            repaint: "none",
            visible: 0,
            uploads: 0,
            backlog: (0, 0, 0, 0),
            inflight: 0,
            retry: 0,
            ttv_pending: 0,
        });
        assert!(stage_snapshot().is_empty() || stage_snapshot() == "?");
    }
}
