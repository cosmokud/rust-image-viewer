#![cfg(target_os = "windows")]

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

fn expand_env_vars(input: &str) -> String {
    use winapi::um::processenv::ExpandEnvironmentStringsW;

    let wide_in = wide(OsStr::new(input));
    unsafe {
        let needed = ExpandEnvironmentStringsW(wide_in.as_ptr(), std::ptr::null_mut(), 0);
        if needed == 0 {
            return input.to_string();
        }

        let mut buf: Vec<u16> = vec![0; needed as usize];
        let written = ExpandEnvironmentStringsW(wide_in.as_ptr(), buf.as_mut_ptr(), needed);
        if written == 0 {
            return input.to_string();
        }

        if let Some(last) = buf.last() {
            if *last == 0 {
                buf.pop();
            }
        }
        String::from_utf16_lossy(&buf)
    }
}

fn read_reg_string(
    hkey: winapi::shared::minwindef::HKEY,
    subkey: &str,
    value: &str,
) -> Option<String> {
    use winapi::shared::minwindef::DWORD;
    use winapi::um::winreg::{RegGetValueW, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ};

    let subkey_w = wide(OsStr::new(subkey));
    let value_w = wide(OsStr::new(value));

    unsafe {
        let mut data_type: DWORD = 0;
        let mut bytes: DWORD = 0;

        let status = RegGetValueW(
            hkey,
            subkey_w.as_ptr(),
            value_w.as_ptr(),
            RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ,
            &mut data_type,
            std::ptr::null_mut(),
            &mut bytes,
        );
        if status != 0 || bytes == 0 {
            return None;
        }

        let mut buf: Vec<u16> = vec![0; (bytes as usize / 2).saturating_add(1)];
        let status2 = RegGetValueW(
            hkey,
            subkey_w.as_ptr(),
            value_w.as_ptr(),
            RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr() as *mut _,
            &mut bytes,
        );
        if status2 != 0 {
            return None;
        }

        while buf.last() == Some(&0) {
            buf.pop();
        }
        let s = String::from_utf16_lossy(&buf);
        let s = s.trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }
}

fn split_path_list(value: &str) -> Vec<String> {
    value
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.trim_matches('"').to_string())
        .collect()
}

fn normalize_path_key(p: &str) -> String {
    let mut s = p.trim().trim_matches('"').replace('/', "\\");
    while s.ends_with('\\') {
        s.pop();
    }
    s.to_ascii_lowercase()
}

fn merge_path_lists(primary: &str, secondary: &str) -> String {
    use std::collections::HashSet;

    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();

    for entry in split_path_list(primary)
        .into_iter()
        .chain(split_path_list(secondary))
    {
        let key = normalize_path_key(&entry);
        if key.is_empty() {
            continue;
        }
        if seen.insert(key) {
            out.push(entry);
        }
    }

    out.join(";")
}

fn registry_effective_path() -> Option<String> {
    use winapi::um::winreg::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

    let user = read_reg_string(HKEY_CURRENT_USER, "Environment", "Path");
    let machine = read_reg_string(
        HKEY_LOCAL_MACHINE,
        r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
        "Path",
    );

    let machine_expanded = machine.as_deref().map(expand_env_vars);
    let user_expanded = user.as_deref().map(expand_env_vars);

    match (machine_expanded, user_expanded) {
        (Some(m), Some(u)) => Some(format!("{};{}", m, u)),
        (Some(m), None) => Some(m),
        (None, Some(u)) => Some(u),
        (None, None) => None,
    }
}

/// Refreshes the process `PATH` from the registry (HKLM + HKCU), merging with the current
/// process PATH. This makes DLL/plugin discovery resilient when the process is launched from
/// a parent process with a stale/sanitized environment (e.g., some browsers).
pub fn refresh_process_path_from_registry() {
    let Some(reg_path) = registry_effective_path() else {
        return;
    };

    let current = std::env::var("PATH").unwrap_or_default();
    let merged = merge_path_lists(&reg_path, &current);

    if merged != current {
        std::env::set_var("PATH", merged);
    }
}
