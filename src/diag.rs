//! Opt-in freeze/crash forensics. Disabled sessions pay one cached bool load per call site.
//!
//! Primary switch: an `env.local` file next to the executable, so the setting travels with the
//! portable folder and survives a double-click launch (Explorer passes no process env). Copy this
//! beside `rust-image-viewer.exe`:
//!
//! ```text
//! # -- diagnostics on; logs land in <app dir>/logs instead of %TEMP%
//! RIV_DIAG=1
//! RIV_DIAG_DIR=logs
//! RIV_DIAG_FRAME_MS=16
//! RIV_DIAG_HANG_MS=2000
//! # RIV_MANGA_TRACE=1
//! ```
//!
//! Syntax: `KEY=VALUE` per line, `#`/`;` comment lines, blank lines skipped, CRLF tolerated,
//! surrounding quotes and a leading `export ` optional, duplicates resolve to the last one.
//! Only `RIV_*` keys are read. Search order: executable directory, then the working directory.
//!
//! Override for one session (process env beats `env.local`):
//! PowerShell `$env:RIV_DIAG='1'`, run, then `Remove-Item Env:RIV_DIAG`; cmd: `set RIV_DIAG=1`.
//!
//!   `RIV_DIAG=1`       enable the sink below.
//!   `RIV_MANGA_TRACE=1` alias with the same sink (its heal-rule lines land here too).
//!   `RIV_DIAG_DIR`     log directory, relative = resolved against the exe dir
//!                      (default `<temp>/rust-image-viewer`).
//!   `RIV_DIAG_FRAME_MS` slow-frame threshold in ms (default 16).
//!   `RIV_DIAG_HANG_MS`  watchdog stall threshold in ms (default 2000).
//!
//! Files: `diag-<pid>-<unix_ts>.log`, one flush per line so the tail survives an abort
//! (`panic = "abort"` in Cargo.toml), 32 MB cap per file, 3 newest files kept.
//!
//! `--probe-video <path>...` (see `probe_video.rs`) writes its decode report here too, and turns
//! this sink on by itself so a headless probe leaves a log even without `RIV_DIAG`.
//!
//! Not captured: process thread count and peak RSS. Both need platform calls outside std, and
//! a diagnostic that pulls in winapi features is a diagnostic nobody enables by default.
//! shortcut: no thread/RSS snapshot, add GetProcessMemoryInfo if hangs need process-level data.

use std::collections::HashMap;
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
static CONFIG: OnceLock<Config> = OnceLock::new();
static ORIGIN: OnceLock<Instant> = OnceLock::new();
static HEARTBEAT: AtomicU64 = AtomicU64::new(0);
static FRAME_STARTED_MS: AtomicU64 = AtomicU64::new(0);
static SUMMARY_MS: AtomicU64 = AtomicU64::new(u64::MAX);
static STAGE: Mutex<&'static str> = Mutex::new("?");
/// Last stages the frame entered, oldest first, with the instant each was entered. The watchdog
/// prints their durations so the blocking segment is named, not just the enclosing stage.
static STAGES: Mutex<Vec<(&'static str, Instant)>> = Mutex::new(Vec::new());
/// Segments printed per HANG / SLOW line.
const STAGE_SEGMENTS: usize = 8;
/// Indices already breadcrumbed: the first decode/probe per index names the file, repeats do not.
static BREADCRUMBS: Mutex<Option<std::collections::HashSet<usize>>> = Mutex::new(None);
const BREADCRUMB_CAP: usize = 8192;

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
    alpha_fallbacks: u64,
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
    AlphaFallback,
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
    alpha_fallbacks: 0,
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
    /// Total time the upload pass spent this frame, and how many items it touched.
    pub upload_ms: f64,
    pub upload_items: usize,
    pub backlog: (usize, usize, usize, usize),
    pub inflight: usize,
    pub retry: usize,
    pub ttv_pending: usize,
}

/// `env.local` file name, in the exe directory (portable folder) or the working directory.
const CONFIG_FILE: &str = "env.local";

/// Where the settings came from; the startup line names it.
enum ConfigSource {
    /// Absolute path of the `env.local` that was read.
    File(PathBuf),
    ProcessEnv,
    None,
}

/// Resolved once: `env.local` values with the real process env layered on top (env wins).
struct Config {
    values: HashMap<String, String>,
    source: ConfigSource,
    /// Process env had at least one `RIV_*` key, so some file value may be shadowed.
    env_overrides: bool,
    /// Directory of the running exe; relative `RIV_DIAG_DIR` resolves against it.
    exe_dir: Option<PathBuf>,
}

/// Cached: `false` unless one of the enabling flags is set.
pub fn enabled() -> bool {
    *ENABLED.get_or_init(|| flag("RIV_DIAG") || flag("RIV_MANGA_TRACE"))
}

/// `true` only for the literal value `1`.
pub fn flag(key: &str) -> bool {
    setting(key).as_deref() == Some("1")
}

/// Every `RIV_*` switch resolves here: process env first, then `env.local`, and an empty value
/// counts as unset so `RIV_DIAG_DIR=` falls back to the default directory.
pub fn setting(key: &str) -> Option<String> {
    config()
        .values
        .get(&key.to_ascii_uppercase())
        .filter(|value| !value.is_empty())
        .cloned()
}

/// Line every diag session starts with: switch state, config source, effective values.
/// Callers print it only when `enabled()`, because a disabled session writes nothing anywhere.
pub fn status_line() -> String {
    let config = config();
    let source = match &config.source {
        ConfigSource::File(path) => path.display().to_string(),
        ConfigSource::ProcessEnv => "process env".to_string(),
        ConfigSource::None => "none".to_string(),
    };
    format!(
        "diag start pid={} enabled={} source={} env_overrides={} riv_diag={} riv_manga_trace={} riv_diag_dir={} frame_ms={} hang_ms={}",
        std::process::id(),
        if enabled() { "yes" } else { "no" },
        source,
        if config.env_overrides { "yes" } else { "no" },
        setting("RIV_DIAG").unwrap_or_else(|| "0".to_string()),
        setting("RIV_MANGA_TRACE").unwrap_or_else(|| "0".to_string()),
        log_dir().display(),
        setting_u64("RIV_DIAG_FRAME_MS", DEFAULT_FRAME_MS),
        setting_u64("RIV_DIAG_HANG_MS", DEFAULT_HANG_MS),
    )
}

fn config() -> &'static Config {
    CONFIG.get_or_init(|| {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf));
        let cwd = std::env::current_dir().ok();
        let path = find_config(exe_dir.as_deref(), cwd.as_deref());
        let file = path
            .as_deref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| parse_env_file(&text))
            .unwrap_or_default();
        let (values, env_overrides) = merge(file, std::env::vars());
        let source = match (path, env_overrides) {
            (Some(path), _) => ConfigSource::File(path),
            (None, true) => ConfigSource::ProcessEnv,
            (None, false) => ConfigSource::None,
        };
        Config {
            values,
            source,
            env_overrides,
            exe_dir,
        }
    })
}

/// Exe directory first, working directory second; first existing file wins. Both roots are
/// absolute, so the returned path is too. Reads nothing.
fn find_config(exe_dir: Option<&Path>, cwd: Option<&Path>) -> Option<PathBuf> {
    [exe_dir, cwd]
        .into_iter()
        .flatten()
        .map(|dir| dir.join(CONFIG_FILE))
        .find(|path| path.is_file())
}

/// `KEY=VALUE` lines; `#`/`;` comment lines, blank lines skipped, CRLF tolerated, one pair of
/// surrounding quotes and a leading `export ` stripped. Duplicates: last wins. Non-`RIV_` keys are
/// dropped at the door, so a stray `PATH=` in the file cannot reach the app.
fn parse_env_file(text: &str) -> HashMap<String, String> {
    let mut values = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_uppercase();
        if !key.starts_with("RIV_") {
            continue;
        }
        values.insert(key, unquote(value.trim()));
    }
    values
}

fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    let quoted = bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''));
    if quoted {
        value[1..value.len() - 1].to_string()
    } else {
        value.to_string()
    }
}

/// Process env wins over the file, so `$env:RIV_DIAG='0'` still switches one run off.
fn merge(
    file: HashMap<String, String>,
    env: impl Iterator<Item = (String, String)>,
) -> (HashMap<String, String>, bool) {
    let mut values = file;
    let mut overrides = false;
    for (key, value) in env {
        let key = key.to_ascii_uppercase();
        if key.starts_with("RIV_") {
            values.insert(key, value);
            overrides = true;
        }
    }
    (values, overrides)
}

fn setting_u64(key: &str, default: u64) -> u64 {
    setting(key)
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(default)
}

fn frame_threshold() -> Duration {
    Duration::from_millis(setting_u64("RIV_DIAG_FRAME_MS", DEFAULT_FRAME_MS))
}

fn hang_threshold() -> Duration {
    Duration::from_millis(setting_u64("RIV_DIAG_HANG_MS", DEFAULT_HANG_MS))
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
            // First line: how diag was switched on and from where, then the effective config.
            write_line(&format!("{} log={}", status_line(), path.display()));
        }
        Err(_) => {
            let _ = SINK.set(None);
        }
    }

    install_panic_hook();
    spawn_watchdog();
}

fn log_dir() -> PathBuf {
    resolve_log_dir(
        setting("RIV_DIAG_DIR").as_deref(),
        config().exe_dir.as_deref(),
    )
}

/// Relative `RIV_DIAG_DIR` resolves against the exe directory so `RIV_DIAG_DIR=logs` keeps the
/// logs inside the portable folder; absolute paths are used as given.
fn resolve_log_dir(value: Option<&str>, exe_dir: Option<&Path>) -> PathBuf {
    match value.filter(|value| !value.is_empty()) {
        Some(value) => {
            let path = PathBuf::from(value.trim());
            match exe_dir {
                Some(base) if path.is_relative() => base.join(path),
                _ => path,
            }
        }
        None => std::env::temp_dir().join("rust-image-viewer"),
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

/// Per-file decode/upload forensics: names the file (and its numbers) behind a slow or failed
/// decode. Gated by `RIV_MANGA_TRACE`, rate-limited to two lines per second so a scroll storm
/// cannot fill the log. Disabled sessions pay one cached bool load; `line` is a closure so the
/// formatting work is skipped too.
pub fn trace_file(tag: &str, line: impl FnOnce() -> String) -> bool {
    const MIN_INTERVAL_MS: u64 = 500;
    static ENABLED_FILE: OnceLock<bool> = OnceLock::new();
    static LAST_MS: AtomicU64 = AtomicU64::new(u64::MAX);

    if !*ENABLED_FILE.get_or_init(|| flag("RIV_MANGA_TRACE")) {
        return false;
    }

    let now = ORIGIN.get_or_init(Instant::now).elapsed().as_millis() as u64;
    let mut last = LAST_MS.load(Ordering::Relaxed);
    loop {
        // `u64::MAX` = never written, so the first line is not swallowed by the window.
        if last != u64::MAX && now.saturating_sub(last) < MIN_INTERVAL_MS {
            return false;
        }
        match LAST_MS.compare_exchange_weak(last, now, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(actual) => last = actual,
        }
    }

    write_line(&format!("[{}] {}", tag, line()));
    true
}

/// Per-index breadcrumb before a decode or probe: a hard kill leaves the offending file as the
/// last line in the log. First time per index only, and `line` is not even built on a repeat.
pub fn note_file(index: usize, tag: &str, line: impl FnOnce() -> String) -> bool {
    if !enabled() {
        return false;
    }

    let mut seen = locked(&BREADCRUMBS);
    let seen = seen.get_or_insert_with(std::collections::HashSet::new);
    if seen.len() >= BREADCRUMB_CAP {
        // Indices are reused per folder: an unbounded set would silence later folders.
        seen.clear();
    }
    if !seen.insert(index) {
        return false;
    }

    write_line(&format!("[{tag}] {}", line()));
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
        Counter::AlphaFallback => &mut counters.alpha_fallbacks,
    };
    *slot = slot.saturating_add(delta);
}

/// Name the stage the frame is in; the watchdog prints the last one it saw and the durations of
/// the last few.
pub fn note_stage(name: &'static str) {
    if !enabled() {
        return;
    }
    *locked(&STAGE) = name;
    let mut stages = locked(&STAGES);
    stages.push((name, Instant::now()));
    let excess = stages.len().saturating_sub(STAGE_SEGMENTS * 4);
    if excess > 0 {
        stages.drain(..excess);
    }
}

/// ` segments=upload:12ms cache_insert:4801ms paint:3ms` for the last stages entered. Each
/// duration runs until the next stage was entered (or now), so the slow one is named.
fn stage_segments() -> String {
    let stages = locked(&STAGES);
    if stages.len() < 2 {
        return String::new();
    }

    let now = Instant::now();
    let tail = &stages[stages.len().saturating_sub(STAGE_SEGMENTS)..];
    let mut out = String::from(" segments=");
    for (position, (name, started)) in tail.iter().enumerate() {
        let ended = tail.get(position + 1).map_or(now, |(_, next)| *next);
        let millis = ended.saturating_duration_since(*started).as_secs_f64() * 1000.0;
        if position > 0 {
            out.push(' ');
        }
        out.push_str(name);
        out.push_str(&format!(":{millis:.0}ms"));
    }
    out
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
        "frame {total_ms:.1}ms stage={} repaint={} visible={} uploads={} upload_ms={:.1} upload_items={} backlog=loads:{} dec:{} dim:{} probes:{} inflight={} retry={} ttv={}",
        locked(&STAGE).to_string(),
        stats.repaint,
        stats.visible,
        stats.uploads,
        stats.upload_ms,
        stats.upload_items,
        stats.backlog.0,
        stats.backlog.1,
        stats.backlog.2,
        stats.backlog.3,
        stats.inflight,
        stats.retry,
        stats.ttv_pending,
    );
    let segments = stage_segments();
    *locked(&LAST_FRAME) = Some(format!("{line}{segments}"));

    if stats.total >= frame_threshold() {
        count(Counter::SlowFrames, 1);
        write_line(&format!("SLOW {line}{segments}"));
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
            "summary 1s frames={} slow={} gst_probes={} probe_waits={} probe_timeouts={} dim_attempts={} dim_chunks={} dim_parked={} dim_flushed={} dim_dropped={} alpha_fallback={}",
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
            spent.alpha_fallbacks,
        ));
    }
}

fn stage_snapshot() -> String {
    locked(&STAGE).to_string()
}

fn counters_snapshot() -> String {
    let counters = *locked(&COUNTERS);
    format!(
        "frames={} slow={} gst_probes={} probe_waits={} probe_timeouts={} dim_attempts={} dim_chunks={} dim_parked={} dim_flushed={} dim_dropped={} alpha_fallback={}",
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
        counters.alpha_fallbacks,
    )
}

/// Dump what the stuck frame was doing. Called from the watchdog thread only.
fn dump_hang(stalled_ms: u128) {
    let stage = stage_snapshot();
    write_line(&format!(
        "HANG no frame completed for {stalled_ms}ms; last stage={stage} since_frame_start={}ms{}",
        origin()
            .elapsed()
            .as_millis()
            .saturating_sub(FRAME_STARTED_MS.load(Ordering::Relaxed) as u128),
        stage_segments()
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
    fn env_file_parses_comments_quotes_and_duplicates() {
        let text = "# comment\r\n; also a comment\r\n\r\nexport RIV_DIAG=\"1\"\r\n\
                    riv_diag_hang_ms = '4000'\r\n\
                    RIV_DIAG_DIR=logs\r\nRIV_DIAG_DIR=roomy logs\r\n\
                    PATH=/nope\r\nno_equals_here\r\n";
        let values = parse_env_file(text);
        assert_eq!(values.get("RIV_DIAG").map(String::as_str), Some("1"));
        assert_eq!(
            values.get("RIV_DIAG_HANG_MS").map(String::as_str),
            Some("4000")
        );
        // Duplicate: last wins, inner spaces of an unquoted value kept.
        assert_eq!(
            values.get("RIV_DIAG_DIR").map(String::as_str),
            Some("roomy logs")
        );
        // Unknown key dropped, malformed line skipped.
        assert!(!values.contains_key("PATH"));
        assert_eq!(values.len(), 3);
    }

    #[test]
    fn process_env_wins_over_the_file() {
        let file = parse_env_file("RIV_DIAG=1\nRIV_DIAG_HANG_MS=2000\n");
        let env = vec![
            ("RIV_DIAG_HANG_MS".to_string(), "9000".to_string()),
            ("RIV_MANGA_TRACE".to_string(), "1".to_string()),
            ("PATH".to_string(), "ignored".to_string()),
        ];

        let (values, overrides) = merge(file, env.into_iter());

        assert!(overrides);
        assert_eq!(
            values.get("RIV_DIAG_HANG_MS").map(String::as_str),
            Some("9000")
        );
        assert_eq!(values.get("RIV_DIAG").map(String::as_str), Some("1"));
        assert_eq!(values.get("RIV_MANGA_TRACE").map(String::as_str), Some("1"));
        assert!(!values.contains_key("PATH"));
    }

    #[test]
    fn relative_diag_dir_lands_beside_the_exe() {
        let base = Path::new("appdir");
        assert_eq!(resolve_log_dir(Some("logs"), Some(base)), base.join("logs"));

        let absolute = std::env::temp_dir().join("riv-diag-absolute");
        let as_text = absolute.display().to_string();
        assert_eq!(resolve_log_dir(Some(&as_text), Some(base)), absolute);

        let default = std::env::temp_dir().join("rust-image-viewer");
        assert_eq!(resolve_log_dir(Some(""), Some(base)), default);
        assert_eq!(resolve_log_dir(None, None), default);
    }

    #[test]
    fn env_local_beside_the_exe_wins_over_the_working_directory() {
        let root = std::env::temp_dir().join(format!("riv-diag-config-{}", std::process::id()));
        let exe_dir = root.join("app");
        let cwd = root.join("cwd");
        std::fs::create_dir_all(&exe_dir).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();

        assert_eq!(find_config(Some(&exe_dir), Some(&cwd)), None);
        std::fs::write(cwd.join(CONFIG_FILE), b"RIV_DIAG=1\n").unwrap();
        assert_eq!(
            find_config(Some(&exe_dir), Some(&cwd)),
            Some(cwd.join(CONFIG_FILE))
        );
        std::fs::write(exe_dir.join(CONFIG_FILE), b"RIV_DIAG=1\n").unwrap();
        assert_eq!(
            find_config(Some(&exe_dir), Some(&cwd)),
            Some(exe_dir.join(CONFIG_FILE))
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_env_local_writes_nothing() {
        let dir = std::env::temp_dir().join(format!("riv-diag-missing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        // No file anywhere: nothing to read, nothing created, diag stays off.
        assert_eq!(find_config(None, Some(&dir)), None);
        assert!(!enabled());
        assert!(!dir.join(CONFIG_FILE).exists());
        assert!(!dir.join("logs").exists());

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
            upload_ms: 0.0,
            upload_items: 0,
            backlog: (0, 0, 0, 0),
            inflight: 0,
            retry: 0,
            ttv_pending: 0,
        });
        assert!(stage_snapshot().is_empty() || stage_snapshot() == "?");
    }

    #[test]
    fn stage_segments_name_the_slow_one() {
        // A segment lasts until the next stage starts, so a fixed base keeps it deterministic.
        let base = Instant::now();
        *locked(&STAGES) = vec![
            ("upload", base - Duration::from_millis(4801)),
            ("cache_insert", base - Duration::from_millis(1)),
            ("paint", base),
        ];
        let segments = stage_segments();
        assert!(segments.contains("upload:4800ms"), "{segments}");
        assert!(segments.contains("cache_insert:1ms"), "{segments}");
        assert!(segments.ends_with("paint:0ms"), "{segments}");
        assert!(segments.starts_with(" segments="), "{segments}");
    }
}
