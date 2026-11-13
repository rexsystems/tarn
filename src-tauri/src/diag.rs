//! Redacted diagnostic ring buffer. Cookie *values* are never stored.

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const CAP: usize = 400;

static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn log(msg: impl AsRef<str>) {
    let msg = redact(msg.as_ref());
    eprintln!("{msg}");
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let line = format!("{ts} {msg}");
    if let Ok(mut g) = LINES.lock() {
        g.push(line);
        let extra = g.len().saturating_sub(CAP);
        if extra > 0 {
            g.drain(0..extra);
        }
    }
}

pub fn dump() -> String {
    let g = LINES.lock().ok();
    match g {
        Some(g) => g.join("\n"),
        None => String::new(),
    }
}

fn redact(s: &str) -> String {
    // Strip anything that looks like a cookie assignment of a secret name.
    let mut out = s.to_string();
    for name in [
        "SAPISID=",
        "__Secure-1PSID=",
        "__Secure-3PSID=",
        "__Secure-3PAPISID=",
        "__Secure-1PAPISID=",
        "LOGIN_INFO=",
        "SID=",
        "SSID=",
        "HSID=",
        "SIDCC=",
        "__Secure-1PSIDCC=",
        "__Secure-3PSIDCC=",
        "__Secure-1PSIDTS=",
        "__Secure-3PSIDTS=",
        "APISID=",
        "__Secure-3PSIDTS=",
    ] {
        if let Some(i) = out.find(name) {
            let rest = &out[i + name.len()..];
            let end = rest.find([';', ' ', '"']).unwrap_or(rest.len());
            out.replace_range(i + name.len()..i + name.len() + end, "<redacted>");
        }
    }
    out
}
