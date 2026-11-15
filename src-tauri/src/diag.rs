//! Redacted diagnostic ring buffer. Cookie *values* are never stored.
//! In debug builds, `debug` / failure helpers print extra context.
//! Soft failures also become Sentry/GlitchTip breadcrumbs (not events).

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const CAP: usize = 400;

static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn log(msg: impl AsRef<str>) {
    let msg = redact(msg.as_ref());
    eprintln!("{msg}");
    push_line(&msg);
    breadcrumb("log", &msg, sentry::Level::Info);
}

/// Extra detail only in debug / when TARN_DEBUG=1.
pub fn debug(msg: impl AsRef<str>) {
    if !verbose() {
        return;
    }
    let msg = redact(msg.as_ref());
    eprintln!("[dbg] {msg}");
    push_line(&format!("dbg {msg}"));
}

/// Soft/expected API failure (e.g. lyrics 400). Logged + breadcrumb, no event.
pub fn soft_fail(kind: &str, msg: impl AsRef<str>) {
    let msg = redact(msg.as_ref());
    let line = format!("{kind}: {msg}");
    eprintln!("{line}");
    push_line(&line);
    breadcrumb(kind, &msg, sentry::Level::Warning);
}

/// Unexpected failure worth an event (5xx, panic-adjacent, etc.).
#[allow(dead_code)]
pub fn error(kind: &str, msg: impl AsRef<str>) {
    let msg = redact(msg.as_ref());
    let line = format!("{kind}: {msg}");
    eprintln!("{line}");
    push_line(&line);
    breadcrumb(kind, &msg, sentry::Level::Error);
    sentry::with_scope(
        |scope| {
            scope.set_tag("kind", kind);
        },
        || {
            sentry::capture_message(&line, sentry::Level::Error);
        },
    );
}

pub fn dump() -> String {
    let g = LINES.lock().ok();
    match g {
        Some(g) => g.join("\n"),
        None => String::new(),
    }
}

/// Parse YouTubei JSON error bodies into a short reason string.
pub fn yt_error_reason(body: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        let code = v
            .pointer("/error/code")
            .and_then(|x| x.as_i64())
            .map(|n| n.to_string())
            .unwrap_or_default();
        let status = v
            .pointer("/error/status")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let message = v
            .pointer("/error/message")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let detail = v
            .pointer("/error/errors/0/reason")
            .and_then(|x| x.as_str())
            .or_else(|| v.pointer("/error/errors/0/message").and_then(|x| x.as_str()))
            .unwrap_or("");
        let mut parts = Vec::new();
        if !code.is_empty() {
            parts.push(code);
        }
        if !status.is_empty() {
            parts.push(status.to_string());
        }
        if !message.is_empty() {
            parts.push(message.to_string());
        }
        if !detail.is_empty() && detail != message {
            parts.push(detail.to_string());
        }
        if !parts.is_empty() {
            return parts.join(" | ");
        }
    }
    body.chars().take(220).collect()
}

pub fn verbose() -> bool {
    cfg!(debug_assertions) || std::env::var_os("TARN_DEBUG").is_some()
}

fn push_line(msg: &str) {
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

fn breadcrumb(category: &str, message: &str, level: sentry::Level) {
    sentry::add_breadcrumb(sentry::Breadcrumb {
        category: Some(category.into()),
        message: Some(message.into()),
        level,
        ..Default::default()
    });
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
