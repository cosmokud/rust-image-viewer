#![windows_subsystem = "windows"]

use std::ffi::OsString;
use std::process::{Command, ExitCode, Stdio};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

fn main() -> ExitCode {
    // GStreamer will spawn this process and communicate via redirected stdio.
    // We must preserve argv + stdio, but we want the *real* scanner to run without
    // creating a visible console window.

    let real_scanner: OsString = std::env::var_os("GST_PLUGIN_SCANNER_REAL")
        .or_else(|| std::env::var_os("GST_PLUGIN_SCANNER_REAL_1_0"))
        .unwrap_or_default();

    if real_scanner.is_empty() {
        return ExitCode::from(1);
    }

    let mut cmd = Command::new(real_scanner);
    cmd.args(std::env::args_os().skip(1))
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    match cmd.status() {
        Ok(status) => ExitCode::from(status.code().unwrap_or(1) as u8),
        Err(_) => ExitCode::from(1),
    }
}
