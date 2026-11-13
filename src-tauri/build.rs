use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let target = std::env::var("TARGET").unwrap_or_default();
    println!("cargo:rustc-env=TARGET_TRIPLE={target}");
    ensure_sidecar(&target);
    tauri_build::build();
}

fn ensure_sidecar(target: &str) {
    if target.is_empty() || target.contains("android") || target.contains("ios") {
        return;
    }
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let name = if target.contains("windows") {
        format!("yt-dlp-{target}.exe")
    } else {
        format!("yt-dlp-{target}")
    };
    let dest = manifest.join("binaries").join(&name);
    if is_present(&dest) {
        return;
    }
    let script = manifest.join("../scripts/fetch-ytdlp.mjs");
    let status = Command::new("node")
        .arg(&script)
        .arg(target)
        .status();
    match status {
        Ok(s) if s.success() && is_present(&dest) => {}
        Ok(s) => panic!(
            "failed to fetch the yt-dlp sidecar (exit {s}). Run: node scripts/fetch-ytdlp.mjs"
        ),
        Err(e) => panic!(
            "node is required to fetch the yt-dlp sidecar ({e}). Run: node scripts/fetch-ytdlp.mjs"
        ),
    }
}

fn is_present(p: &Path) -> bool {
    p.is_file() && p.metadata().map(|m| m.len() > 1_000_000).unwrap_or(false)
}
