//! Platform host facts, deterministic binary lookup, process identity,
//! and parent-first process tree termination (`hermes_platform` + `process_registry_termination` parity).

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PlatformError {
    #[error("I/O failure for {context}: {source}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },
    #[error("process {pid} signal failure: {message}")]
    Signal { pid: u32, message: String },
}

/// Immutable host facts cached once per process, independent of environment tampering.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostFacts {
    pub os_family: String,
    pub os: String,
    pub arch: String,
    pub cpu_count: usize,
    pub is_wsl: bool,
    pub is_container: bool,
    pub is_termux: bool,
}

static HOST_FACTS_CACHE: OnceLock<HostFacts> = OnceLock::new();

impl HostFacts {
    /// Detect host facts once and cache them process-wide without trusting env overrides.
    pub fn detect() -> &'static HostFacts {
        HOST_FACTS_CACHE.get_or_init(|| {
            let os = env::consts::OS.to_string();
            let os_family = env::consts::FAMILY.to_string();
            let arch = env::consts::ARCH.to_string();
            let cpu_count = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1);

            let is_wsl = detect_wsl();
            let is_container = detect_container();
            let is_termux = Path::new("/data/data/com.termux/files/usr").is_dir();

            HostFacts {
                os_family,
                os,
                arch,
                cpu_count,
                is_wsl,
                is_container,
                is_termux,
            }
        })
    }
}

fn detect_wsl() -> bool {
    if !cfg!(target_os = "linux") {
        return false;
    }
    if let Ok(osrel) = fs::read_to_string("/proc/sys/kernel/osrelease") {
        let lower = osrel.to_ascii_lowercase();
        if lower.contains("microsoft") || lower.contains("wsl") {
            return true;
        }
    }
    Path::new("/proc/sys/fs/binfmt_misc/WSLInterop").exists()
}

fn detect_container() -> bool {
    if Path::new("/.dockerenv").exists() || Path::new("/run/.containerenv").exists() {
        return true;
    }
    if let Ok(cgroup) = fs::read_to_string("/proc/1/cgroup") {
        return cgroup.contains("docker")
            || cgroup.contains("kubepods")
            || cgroup.contains("containerd")
            || cgroup.contains("lxc");
    }
    false
}

/// Deterministic executable lookup along PATH without executing or installing anything.
pub struct ExecutableResolver;

impl ExecutableResolver {
    pub fn which(name: &str) -> Option<PathBuf> {
        if name.trim().is_empty() {
            return None;
        }
        let candidate = Path::new(name);
        if candidate.is_absolute() || name.contains(std::path::MAIN_SEPARATOR) {
            return is_executable(candidate).then(|| candidate.to_path_buf());
        }
        let path_var = env::var_os("PATH")?;
        for dir in env::split_paths(&path_var) {
            let full = dir.join(name);
            if is_executable(&full) {
                return Some(full);
            }
            #[cfg(windows)]
            {
                for ext in [".exe", ".cmd", ".bat"] {
                    let with_ext = dir.join(format!("{name}{ext}"));
                    if is_executable(&with_ext) {
                        return Some(with_ext);
                    }
                }
            }
        }
        None
    }
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .map(|m| m.is_file() && (m.permissions().mode() & 0o111 != 0))
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// `(pid, start_time_ticks)` fingerprint preventing recycled-PID false matches.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub start_time_ticks: Option<u64>,
}

impl ProcessIdentity {
    pub fn current() -> Self {
        let pid = std::process::id();
        Self {
            pid,
            start_time_ticks: read_proc_start_time_ticks(pid),
        }
    }

    pub fn probe(pid: u32) -> Option<Self> {
        if !pid_exists(pid) {
            return None;
        }
        Some(Self {
            pid,
            start_time_ticks: read_proc_start_time_ticks(pid),
        })
    }

    /// True only if `pid` is still alive AND (when recorded) its start time matches.
    pub fn is_still_same_process(&self) -> bool {
        if !pid_exists(self.pid) {
            return false;
        }
        match (self.start_time_ticks, read_proc_start_time_ticks(self.pid)) {
            (Some(expected), Some(actual)) => expected == actual,
            (Some(_), None) => false,
            (None, _) => true,
        }
    }
}

/// Read field 22 (`starttime`) from `/proc/<pid>/stat` safely (handles spaces/parens in comm).
pub fn read_proc_start_time_ticks(pid: u32) -> Option<u64> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let stat_path = format!("/proc/{pid}/stat");
    let content = fs::read_to_string(stat_path).ok()?;
    let rparen = content.rfind(')')?;
    let after = content.get(rparen + 2..)?;
    let fields: Vec<&str> = after.split_whitespace().collect();
    // After `(comm)`, field 3 (`state`) is index 0; field 22 (`starttime`) is index 19.
    fields.get(19)?.parse::<u64>().ok()
}

#[cfg(unix)]
pub fn pid_exists(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    if cfg!(target_os = "linux") && Path::new(&format!("/proc/{pid}")).exists() {
        return !is_zombie(pid);
    }
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(not(unix))]
pub fn pid_exists(pid: u32) -> bool {
    pid > 0
}

fn is_zombie(pid: u32) -> bool {
    if !cfg!(target_os = "linux") {
        return false;
    }
    let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    let Some(rparen) = stat.rfind(')') else {
        return false;
    };
    stat.get(rparen + 2..)
        .and_then(|s| s.chars().next())
        .is_some_and(|ch| ch == 'Z')
}

/// Snapshot direct + transitive child PIDs on Linux via `/proc/*/stat` PPID walk.
pub fn snapshot_descendant_pids(root_pid: u32) -> Vec<u32> {
    if !cfg!(target_os = "linux") {
        return Vec::new();
    }
    let mut children_map: Vec<(u32, u32)> = Vec::new();
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    for entry in entries.flatten() {
        let fname = entry.file_name();
        let Some(name_str) = fname.to_str() else {
            continue;
        };
        let Ok(pid) = name_str.parse::<u32>() else {
            continue;
        };
        if let Some(ppid) = read_ppid(pid) {
            children_map.push((pid, ppid));
        }
    }

    let mut descendants = Vec::new();
    let mut seen: HashSet<u32> = HashSet::from([root_pid]);
    let mut queue = vec![root_pid];
    while let Some(parent) = queue.pop() {
        for &(pid, ppid) in &children_map {
            if ppid == parent && seen.insert(pid) {
                descendants.push(pid);
                queue.push(pid);
            }
        }
    }
    descendants
}

fn read_ppid(pid: u32) -> Option<u32> {
    let content = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rparen = content.rfind(')')?;
    let after = content.get(rparen + 2..)?;
    // Index 0 = state, index 1 = ppid
    after.split_whitespace().nth(1)?.parse::<u32>().ok()
}

/// Terminate a process tree **parent-first** (`tools/AGENTS.md` & `process_registry_termination.py`):
/// 1. Snapshot descendants before signalling.
/// 2. Send `SIGTERM` to the recorded parent ONLY so supervisors (Chromium/shell traps) reap cleanly.
/// 3. Wait up to `grace` for parent exit.
/// 4. Send `SIGTERM`, then `SIGKILL` to surviving snapshot descendants and parent.
pub fn terminate_tree_parent_first(parent_pid: u32, grace: Duration) -> Result<(), PlatformError> {
    #[cfg(not(unix))]
    {
        let _ = (parent_pid, grace);
        return Ok(());
    }
    #[cfg(unix)]
    {
        let descendants = snapshot_descendant_pids(parent_pid);
        if pid_exists(parent_pid) {
            send_signal(parent_pid, "-TERM")?;
        }
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline {
            if !pid_exists(parent_pid) {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        // Snapshot survivors get SIGTERM then SIGKILL if still alive.
        for &pid in &descendants {
            if pid_exists(pid) {
                let _ = send_signal(pid, "-TERM");
            }
        }
        if pid_exists(parent_pid) {
            let _ = send_signal(parent_pid, "-KILL");
        }
        std::thread::sleep(Duration::from_millis(30));
        for &pid in &descendants {
            if pid_exists(pid) {
                let _ = send_signal(pid, "-KILL");
            }
        }
        Ok(())
    }
}

#[cfg(unix)]
fn send_signal(pid: u32, signal_flag: &str) -> Result<(), PlatformError> {
    let status = std::process::Command::new("kill")
        .args([signal_flag, &pid.to_string()])
        .status()
        .map_err(|source| PlatformError::Io {
            context: format!("invoking kill {signal_flag} {pid}"),
            source,
        })?;
    if !status.success() && pid_exists(pid) {
        return Err(PlatformError::Signal {
            pid,
            message: format!("kill {signal_flag} exited with {status}"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_facts_detects_non_empty_values() {
        let facts = HostFacts::detect();
        assert!(!facts.os.is_empty());
        assert!(!facts.arch.is_empty());
        assert!(facts.cpu_count >= 1);
    }

    #[test]
    fn current_process_identity_matches_itself() {
        let id = ProcessIdentity::current();
        assert!(id.is_still_same_process());
        #[cfg(target_os = "linux")]
        assert!(id.start_time_ticks.is_some());
    }

    #[test]
    fn which_finds_sh_on_unix() {
        #[cfg(unix)]
        assert!(ExecutableResolver::which("sh").is_some());
    }

    #[test]
    fn parent_first_termination_reaps_child() {
        #[cfg(unix)]
        {
            let mut child = std::process::Command::new("sleep")
                .arg("30")
                .spawn()
                .expect("spawn sleep");
            let pid = child.id();
            assert!(pid_exists(pid));
            terminate_tree_parent_first(pid, Duration::from_millis(150)).expect("terminate child");
            let _ = child.wait();
            assert!(!pid_exists(pid));
        }
    }
}
