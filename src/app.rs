//! Detecting, stopping and starting the TickTick desktop app.
//!
//! The app caches every task in memory at startup and writes whole rows back
//! from that cache, so the database should only be edited while it is closed.

use anyhow::{Result, bail};
use std::path::PathBuf;
use std::process::Command;

const EXE: &str = "TickTick.exe";

pub fn is_running() -> bool {
    !pids().is_empty()
}

#[cfg(windows)]
pub fn pids() -> Vec<u32> {
    let out = Command::new("tasklist")
        .args(["/FI", &format!("IMAGENAME eq {EXE}"), "/FO", "CSV", "/NH"])
        .output();
    let Ok(out) = out else { return Vec::new() };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let cols: Vec<&str> = l.split("\",\"").collect();
            if cols.len() > 1 && cols[0].trim_matches('"').eq_ignore_ascii_case(EXE) {
                cols[1].trim_matches('"').parse().ok()
            } else {
                None
            }
        })
        .collect()
}

/// On Linux the app runs under wine; its process command line contains the exe path.
#[cfg(not(windows))]
pub fn pids() -> Vec<u32> {
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    dir.filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .filter(|pid| {
            std::fs::read(format!("/proc/{pid}/cmdline"))
                .map(|c| {
                    let first = c.split(|b| *b == 0).next().unwrap_or_default();
                    let s = String::from_utf8_lossy(first).to_ascii_lowercase();
                    s.ends_with("\\ticktick.exe")
                        || s.ends_with("/ticktick.exe")
                        || s == "ticktick.exe"
                })
                .unwrap_or(false)
        })
        .collect()
}

/// Ask TickTick to close; if it is still running after `grace_secs` (closing
/// usually just minimizes it to the tray), kill it unless `graceful_only`.
/// The app commits each change as it happens and SQLite's WAL survives a kill.
pub fn stop(graceful_only: bool, grace_secs: u64) -> Result<bool> {
    if !is_running() {
        return Ok(false);
    }
    request_stop(false)?;
    if wait_exit(grace_secs) {
        return Ok(true);
    }
    if graceful_only {
        bail!("TickTick did not exit within {grace_secs}s (it may have minimized to the tray)");
    }
    request_stop(true)?;
    if wait_exit(15) {
        return Ok(true);
    }
    bail!("TickTick is still running after a forced kill");
}

fn wait_exit(secs: u64) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    while std::time::Instant::now() < deadline {
        if !is_running() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    !is_running()
}

#[cfg(windows)]
fn request_stop(force: bool) -> Result<()> {
    let mut args = vec!["/IM", EXE];
    if force {
        args.push("/F");
    }
    Command::new("taskkill").args(&args).output()?;
    Ok(())
}

#[cfg(not(windows))]
fn request_stop(force: bool) -> Result<()> {
    // SIGTERM lets wine shut the process down; SIGKILL for --force.
    let sig = if force { "-KILL" } else { "-TERM" };
    for pid in pids() {
        Command::new("kill")
            .args([sig, &pid.to_string()])
            .output()?;
    }
    Ok(())
}

pub fn find_exe(explicit: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = explicit {
        return Some(PathBuf::from(p));
    }
    if let Ok(p) = std::env::var("TICKTICK_EXE") {
        return Some(PathBuf::from(p));
    }
    let mut candidates = Vec::new();
    for var in ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"] {
        if let Ok(base) = std::env::var(var) {
            candidates.push(PathBuf::from(&base).join("TickTick").join(EXE));
            candidates.push(
                PathBuf::from(&base)
                    .join("Programs")
                    .join("TickTick")
                    .join(EXE),
            );
        }
    }
    if let Ok(drive) = std::env::var("SystemDrive") {
        candidates.push(PathBuf::from(format!("{drive}\\TickTick\\{EXE}")));
    }
    candidates.into_iter().find(|p| p.exists())
}

pub fn start(exe: Option<&str>) -> Result<PathBuf> {
    let Some(path) = find_exe(exe) else {
        bail!("Could not find {EXE}. Pass --exe <path> or set TICKTICK_EXE.");
    };
    let mut cmd = if cfg!(windows) || path.extension().is_none() {
        Command::new(&path)
    } else {
        // A Windows exe from Linux: run it through wine.
        let mut c = Command::new("wine");
        c.arg(&path);
        c
    };
    if let Some(dir) = path.parent() {
        cmd.current_dir(dir);
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    Ok(path)
}
