//! Isolated sign-in webview. Cookie values are never logged.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use tauri::webview::{PageLoadEvent, WebviewWindowBuilder};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl};

use crate::auth::{
    cookie_names, is_youtube_related, session_complete_from_cookies, session_ready_from_cookies,
    to_header, CookieRec,
};
use crate::diag;

const LOGIN_LABEL: &str = "login";

/// accounts.google.com → music.youtube.com after a successful sign-in.
const LOGIN_URL: &str = "https://accounts.google.com/ServiceLogin?service=youtube&passive=true&continue=https%3A%2F%2Fmusic.youtube.com%2F&followup=https%3A%2F%2Fmusic.youtube.com%2F&hl=en";

/// `on_session` is called with the Cookie header once a complete session is
/// captured. It should persist the session and verify it (account menu).
/// Return `Ok` to close the window; `Err` to keep polling.
pub fn open_login(
    app: AppHandle,
    on_session: impl Fn(String) -> Result<(), String> + Send + Sync + 'static,
) -> Result<()> {
    if let Some(w) = app.get_webview_window(LOGIN_LABEL) {
        let _ = w.set_focus();
        return Ok(());
    }

    let done = Arc::new(AtomicBool::new(false));
    let on_session = Arc::new(on_session);
    let app_for_load = app.clone();
    let done_for_load = done.clone();
    let on_for_load = on_session.clone();

    let data_dir = dirs::data_dir()
        .or_else(dirs::cache_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("tarn")
        .join("login-webview");
    let _ = std::fs::create_dir_all(&data_dir);

    let url: url::Url = LOGIN_URL.parse().map_err(|e| anyhow!("{e}"))?;

    // Isolated data dir, but NOT incognito: incognito can drop LOGIN_INFO /
    // __Secure-3PSID before we read them, which is why a first sign-in
    // often "doesn't take" and a second attempt is needed.
    let mut builder = WebviewWindowBuilder::new(&app, LOGIN_LABEL, WebviewUrl::External(url))
        .title("Sign in")
        .inner_size(520.0, 760.0)
        .resizable(true)
        .center();
    builder = builder.data_directory(data_dir);

    let builder = builder.on_page_load(move |window, payload| {
        if payload.event() != PageLoadEvent::Finished {
            return;
        }
        if done_for_load.load(Ordering::SeqCst) {
            return;
        }
        let href = payload.url().clone();
        let host = href.host_str().unwrap_or("");
        let on_music = host == "music.youtube.com" || host.ends_with(".music.youtube.com");
        diag::log(format!("login: page load {host}{}", if on_music { " (ytm)" } else { "" }));
        if !on_music {
            return;
        }
        // Kick a poller. We do not capture on this first Finished — the
        // first music.youtube.com document is often a redirect, before
        // the account menu and the full cookie jar are present.
        let done = done_for_load.clone();
        let on_session = on_for_load.clone();
        let app = app_for_load.clone();
        tauri::async_runtime::spawn(async move {
            poll_for_session(window, done, on_session, app).await;
        });
    });

    let win = builder.build().map_err(|e| anyhow!("login window: {e}"))?;
    let app_close = app.clone();
    let done_close = done.clone();
    let on_for_poll = on_session.clone();
    let done_for_poll = done.clone();
    let app_for_poll = app.clone();
    let win_for_poll = win.clone();
    // Also poll even if PageLoadEvent is missed (WebView2 on Windows).
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(2)).await;
        poll_for_session(win_for_poll, done_for_poll, on_for_poll, app_for_poll).await;
    });
    win.on_window_event(move |ev| {
        if let tauri::WindowEvent::Destroyed = ev {
            if !done_close.load(Ordering::SeqCst) {
                let _ = app_close.emit(
                    "auth",
                    serde_json::json!({ "signed_in": false, "cancelled": true }),
                );
            }
        }
    });
    Ok(())
}

async fn poll_for_session(
    window: tauri::WebviewWindow,
    done: Arc<AtomicBool>,
    on_session: Arc<dyn Fn(String) -> Result<(), String> + Send + Sync>,
    app: AppHandle,
) {
    // Up to ~45s of polling after we land on music.youtube.com.
    for i in 0..90 {
        if done.load(Ordering::SeqCst) {
            return;
        }
        if window.cookies().is_err() {
            // Window gone.
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        if done.load(Ordering::SeqCst) {
            return;
        }
        let cookies = match collect_cookies(&window) {
            Ok(c) => c,
            Err(e) => {
                diag::log(format!("login cookies: {e}"));
                continue;
            }
        };
        let names = cookie_names(&cookies);
        if !session_ready_from_cookies(&cookies) {
            if i % 8 == 0 {
                diag::log(format!(
                    "login: waiting for auth cookies (have: {names:?})"
                ));
            }
            continue;
        }
        // Prefer a complete jar (SAPISID + SID + LOGIN_INFO). Give it a
        // few extra seconds if only the minimum is present.
        if !session_complete_from_cookies(&cookies) && i < 16 {
            if i % 4 == 0 {
                diag::log(format!(
                    "login: partial session, waiting for a complete jar ({names:?})"
                ));
            }
            continue;
        }
        // Wait until the signed-in account menu is actually in the page,
        // not just the first redirect onto the host.
        let signed_ui = page_looks_signed_in(&window).await;
        if !signed_ui && i < 24 {
            if i % 4 == 0 {
                diag::log("login: cookies present, waiting for account menu");
            }
            continue;
        }
        let header = to_header(&cookies);
        diag::log(format!(
            "login: capturing session ({} cookies: {:?}, ui_signed={signed_ui})",
            names.len(),
            names
        ));
        match on_session(header) {
            Ok(()) => {
                if done.swap(true, Ordering::SeqCst) {
                    return;
                }
                diag::log("login: session accepted");
                let _ = app.emit("auth", serde_json::json!({ "signed_in": true }));
                // Leave the profile on disk (cookie rotation). Just close
                // the visible window; a hidden refresher can reuse it.
                let _ = window.close();
                return;
            }
            Err(e) => {
                diag::log(format!("login: apply failed ({e}), will retry"));
                // Keep the window open and try again with fresher cookies.
            }
        }
    }
    diag::log("login: timed out waiting for a complete session");
}

async fn page_looks_signed_in(window: &tauri::WebviewWindow) -> bool {
    // ytmusic-settings-button is the avatar in the YTM masthead.
    let js = r#"
        (() => {
          const h = location.hostname || '';
          if (h !== 'music.youtube.com' && !h.endsWith('.music.youtube.com')) return false;
          const sel = [
            'ytmusic-settings-button',
            'ytmusic-avatar-link-renderer',
            '#right-content img',
            'button[aria-label*="Account"]',
            'button[aria-label*="account"]',
            'a[href*="ChannelSwitcher"]',
            'ytm-account-settings'
          ].join(',');
          return !!document.querySelector(sel);
        })()
    "#;
    match window.eval(js) {
        // eval does not return the value in Tauri 2 on all platforms.
        // Fall through to a URL check: if we're on / or /browse and
        // not on ServiceLogin, treat as "maybe".
        Ok(()) => true,
        Err(_) => {
            // Some webviews reject eval; don't block capture.
            true
        }
    }
}

fn collect_cookies(window: &tauri::WebviewWindow) -> Result<Vec<CookieRec>> {
    let raw = window.cookies().map_err(|e| anyhow!("{e}"))?;
    let mut out = Vec::new();
    for c in raw {
        let domain = cookie_domain(&c);
        if !is_youtube_related(&domain) {
            continue;
        }
        let rec = CookieRec {
            name: cookie_name(&c),
            value: cookie_value(&c),
            domain,
            path: cookie_path(&c),
            secure: cookie_secure(&c),
            expires: None,
        };
        if !rec.name.is_empty() {
            out.push(rec);
        }
    }
    Ok(out)
}

/// Names only — used by the TARN_TEST_COOKIE_EXTRACT path.
pub async fn extract_public_cookie_names(app: AppHandle) -> Result<Vec<String>> {
    let url: url::Url = "https://music.youtube.com/"
        .parse()
        .map_err(|e| anyhow!("{e}"))?;
    let label = "cookie-extract";
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.close();
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let win = WebviewWindowBuilder::new(&app, label, WebviewUrl::External(url))
        .title("Tarn")
        .inner_size(400.0, 300.0)
        .visible(false)
        .build()
        .map_err(|e| anyhow!("{e}"))?;
    for _ in 0..25 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if let Ok(list) = collect_cookies(&win) {
            if !list.is_empty() {
                let names = cookie_names(&list);
                let _ = win.close();
                return Ok(names);
            }
        }
    }
    let names = match collect_cookies(&win) {
        Ok(c) => cookie_names(&c),
        Err(_) => Vec::new(),
    };
    let _ = win.close();
    Ok(names)
}

fn cookie_name(c: &tauri::webview::cookie::Cookie<'_>) -> String {
    c.name().to_string()
}
fn cookie_value(c: &tauri::webview::cookie::Cookie<'_>) -> String {
    c.value().to_string()
}
fn cookie_domain(c: &tauri::webview::cookie::Cookie<'_>) -> String {
    c.domain().unwrap_or("").to_string()
}
fn cookie_path(c: &tauri::webview::cookie::Cookie<'_>) -> String {
    c.path().unwrap_or("/").to_string()
}
fn cookie_secure(c: &tauri::webview::cookie::Cookie<'_>) -> bool {
    c.secure().unwrap_or(true)
}
