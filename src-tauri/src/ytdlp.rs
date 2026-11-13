//! Locate, run, and occasionally refresh the bundled yt-dlp sidecar.
//!
//! Resolution order: an auto-updated copy in the app data dir, then the
//! Tauri-bundled sidecar, then a system `yt-dlp` on PATH (optional, never
//! required). The official standalone binaries can self-update with `-U`.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{anyhow, Context, Result};

const UA: &str = "Tarn/0.1.0 (+https://github.com/rexsystems/tarn)";
const RELEASE: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download";
const CHECK_EVERY_SECS: u64 = 24 * 60 * 60;

/// Windows flag that prevents a console window from flashing when a
/// console-subsystem sidecar (yt-dlp) is spawned. No-op on other OS.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Apply CREATE_NO_WINDOW and never attach a console stdin. Call this on
/// every process we spawn — never run through a shell.
pub fn silence_std(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.stdin(std::process::Stdio::null());
}

pub fn silence_tokio(cmd: &mut tokio::process::Command) {
    #[cfg(windows)]
    {
        // tokio::process::Command::creation_flags is an inherent method on
        // Windows; the flag prevents a visible console for yt-dlp.exe.
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.stdin(std::process::Stdio::null());
}

pub fn data_bin_dir() -> PathBuf {
    dirs::data_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("tarn")
        .join("bin")
}

pub fn binary_name() -> &'static str {
    if cfg!(windows) {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    }
}

/// Official release asset for this compile target.
pub fn official_asset() -> &'static str {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "yt-dlp_linux"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "yt-dlp_linux_aarch64"
    }
    #[cfg(all(target_os = "linux", target_arch = "arm"))]
    {
        "yt-dlp_linux_armv7l"
    }
    #[cfg(windows)]
    {
        "yt-dlp.exe"
    }
    #[cfg(target_os = "macos")]
    {
        "yt-dlp_macos"
    }
    #[cfg(not(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "arm"),
        windows,
        target_os = "macos"
    )))]
    {
        "yt-dlp"
    }
}

fn is_usable(p: &Path) -> bool {
    if !p.is_file() {
        return false;
    }
    match p.metadata() {
        Ok(m) if m.len() > 1_000_000 => {}
        _ => return false,
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(m) = p.metadata() {
            if m.permissions().mode() & 0o111 == 0 {
                return false;
            }
        }
    }
    true
}

fn make_executable(_p: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let p = _p;
        if let Ok(m) = p.metadata() {
            let mut perms = m.permissions();
            perms.set_mode(perms.mode() | 0o755);
            let _ = std::fs::set_permissions(p, perms);
        }
    }
}

fn target_triple() -> &'static str {
    env!("TARGET_TRIPLE")
}

/// Paths where Tauri (and cargo run) may have placed the sidecar.
fn sidecar_candidates() -> Vec<PathBuf> {
    let triple = target_triple();
    let names = [
        binary_name().to_string(),
        format!("yt-dlp-{triple}"),
        format!("yt-dlp-{triple}.exe"),
    ];
    let mut dirs = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            dirs.push(dir.to_path_buf());
            // AppImage / some layouts put extras one level up.
            if let Some(parent) = dir.parent() {
                dirs.push(parent.join("bin"));
                dirs.push(parent.to_path_buf());
            }
        }
    }
    // cargo run / cargo test: binaries live next to this crate, not the exe.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries");
    dirs.push(manifest);
    let mut out = Vec::new();
    for d in dirs {
        for n in &names {
            out.push(d.join(n));
        }
    }
    out
}

pub fn sidecar_path() -> Option<PathBuf> {
    sidecar_candidates().into_iter().find(|p| is_usable(p))
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let p = dir.join(name);
        if is_usable(&p) {
            return Some(p);
        }
        #[cfg(windows)]
        {
            let e = dir.join(format!("{name}.exe"));
            if is_usable(&e) {
                return Some(e);
            }
        }
    }
    None
}

/// Resolve a yt-dlp binary without downloading. `None` if nothing is available yet.
pub fn resolve() -> Option<PathBuf> {
    let updated = data_bin_dir().join(binary_name());
    if is_usable(&updated) {
        return Some(updated);
    }
    if let Some(p) = sidecar_path() {
        return Some(p);
    }
    which("yt-dlp")
}

fn last_check_path() -> PathBuf {
    data_bin_dir().join(".ytdlp-last-check")
}

fn checked_recently() -> bool {
    let p = last_check_path();
    let Ok(m) = p.metadata() else {
        return false;
    };
    let Ok(mtime) = m.modified() else {
        return false;
    };
    SystemTime::now()
        .duration_since(mtime)
        .map(|d| d.as_secs() < CHECK_EVERY_SECS)
        .unwrap_or(false)
}

fn touch_last_check() {
    let dir = data_bin_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(last_check_path(), b"");
}

async fn download_official(dest: &Path) -> Result<()> {
    let url = format!("{RELEASE}/{}", official_asset());
    let client = reqwest::Client::builder()
        .user_agent(UA)
        .redirect(reqwest::redirect::Policy::limited(10))
        .timeout(Duration::from_secs(180))
        .build()?;
    let resp = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("download {url}"))?;
    if !resp.status().is_success() {
        return Err(anyhow!("download {url}: HTTP {}", resp.status()));
    }
    let bytes = resp.bytes().await?;
    if bytes.len() < 1_000_000 {
        return Err(anyhow!(
            "yt-dlp download too small ({} bytes)",
            bytes.len()
        ));
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Keep the original filename + ".download" so Windows `yt-dlp.exe`
    // becomes `yt-dlp.exe.download`, not `yt-dlp.download`.
    let tmp = dest.with_file_name(format!(
        "{}.download",
        dest.file_name().unwrap_or_default().to_string_lossy()
    ));
    std::fs::write(&tmp, &bytes)?;
    make_executable(&tmp);
    replace_file(&tmp, dest)?;
    Ok(())
}

/// Atomically put `tmp` at `dest`. On Windows the destination exe is often
/// locked (the previous yt-dlp is still running), so we move it aside first.
fn replace_file(tmp: &Path, dest: &Path) -> Result<()> {
    if dest.exists() && cfg!(windows) {
        let bak = dest.with_file_name(format!(
            "{}.old",
            dest.file_name().unwrap_or_default().to_string_lossy()
        ));
        let _ = std::fs::remove_file(&bak);
        if let Err(e) = std::fs::rename(dest, &bak) {
            eprintln!("yt-dlp: could not move old binary aside: {e}");
        }
        match std::fs::rename(tmp, dest) {
            Ok(()) => {
                let _ = std::fs::remove_file(&bak);
                Ok(())
            }
            Err(e) => {
                // Put the old one back if the swap failed.
                if bak.exists() && !dest.exists() {
                    let _ = std::fs::rename(&bak, dest);
                }
                Err(e.into())
            }
        }
    } else {
        std::fs::rename(tmp, dest).map_err(Into::into)
    }
}

fn seed_from_sidecar(dest: &Path) -> Result<bool> {
    let Some(src) = sidecar_path() else {
        return Ok(false);
    };
    if src == dest {
        return Ok(true);
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = dest.with_extension("copy");
    std::fs::copy(&src, &tmp)?;
    make_executable(&tmp);
    std::fs::rename(&tmp, dest)?;
    Ok(true)
}

/// Make sure a yt-dlp binary exists, downloading one if needed.
pub async fn ensure() -> Result<PathBuf> {
    if let Some(p) = resolve() {
        return Ok(p);
    }
    let dest = data_bin_dir().join(binary_name());
    if !seed_from_sidecar(&dest)? {
        download_official(&dest)
            .await
            .context("could not fetch yt-dlp")?;
    }
    if is_usable(&dest) {
        Ok(dest)
    } else {
        Err(anyhow!("yt-dlp is not available"))
    }
}

/// Refresh the writable yt-dlp in the app data dir from the official
/// GitHub release. Safe to fire-and-forget at startup.
///
/// We download to a temp file and rename over the destination. Official
/// `yt-dlp -U` tries to overwrite itself in place, which fails on Windows
/// (the file is locked, or the bundled sidecar is read-only) and is a
/// common cause of a stale extractor.
pub async fn maybe_update() {
    if cfg!(any(target_os = "android", target_os = "ios")) {
        return;
    }
    if checked_recently() {
        return;
    }
    let dest = data_bin_dir().join(binary_name());
    if !is_usable(&dest) {
        let _ = seed_from_sidecar(&dest);
    }
    let before = version(&dest);
    match download_official(&dest).await {
        Ok(()) => {
            let after = version(&dest);
            eprintln!(
                "yt-dlp update: {} -> {}",
                before.as_deref().unwrap_or("none"),
                after.as_deref().unwrap_or("?")
            );
        }
        Err(e) => {
            eprintln!("yt-dlp update (download): {e}");
            // Last resort: `yt-dlp -U` on a writable copy. May fail on
            // Windows if the exe is locked; we already tried a replace.
            if is_usable(&dest) {
                let mut upd = tokio::process::Command::new(&dest);
                silence_tokio(&mut upd);
                match upd
                    .arg("-U")
                    .arg("--no-warnings")
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .output()
                    .await
                {
                    Ok(out) => {
                        let msg = String::from_utf8_lossy(&out.stderr);
                        let stdout = String::from_utf8_lossy(&out.stdout);
                        let line = msg
                            .lines()
                            .chain(stdout.lines())
                            .last()
                            .unwrap_or("updated");
                        eprintln!("yt-dlp update (-U): {line}");
                    }
                    Err(e) => eprintln!("yt-dlp update (-U): {e}"),
                }
            }
        }
    }
    touch_last_check();
}

pub fn version(path: &Path) -> Option<String> {
    let mut cmd = std::process::Command::new(path);
    silence_std(&mut cmd);
    let out = cmd
        .arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}
