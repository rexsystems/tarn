//! Isolated sign-in webview. Cookie values are never logged.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use tauri::webview::{PageLoadEvent, WebviewWindowBuilder};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl};

use crate::auth::{
    cookie_names, is_youtube_related, session_ready_from_cookies, to_header, CookieRec,
};

const LOGIN_LABEL: &str = "login";

/// accounts.google.com → music.youtube.com after a successful sign-in.
const LOGIN_URL: &str = "https://accounts.google.com/ServiceLogin?service=youtube&passive=true&continue=https%3A%2F%2Fmusic.youtube.com%2F&followup=https%3A%2F%2Fmusic.youtube.com%2F&hl=en";

pub fn open_login(app: AppHandle, on_session: impl Fn(String) + Send + Sync + 'static) -> Result<()> {
    if let Some(w) = app.get_webview_window(LOGIN_LABEL) {
        let _ = w.set_focus();
        return Ok(());
    }

    let done = Arc::new(AtomicBool::new(false));
    let on_session = Arc::new(on_session);
    let app_for_load = app.clone();
    let done_for_load = done.clone();
    let on_for_load = on_session.clone();

    let data_dir = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("tarn")
        .join("login-webview");
    let _ = std::fs::create_dir_all(&data_dir);

    let url: url::Url = LOGIN_URL.parse().map_err(|e| anyhow!("{e}"))?;

    let mut builder = WebviewWindowBuilder::new(&app, LOGIN_LABEL, WebviewUrl::External(url))
        .title("Sign in")
        .inner_size(520.0, 760.0)
        .resizable(true)
        .center();

    // Isolated profile so we can close it after capture (YouTube rotates
    // cookies if the same session stays open).
    builder = builder.incognito(true);
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
        if !on_music {
            return;
        }
        let done = done_for_load.clone();
        let on_session = on_for_load.clone();
        let app = app_for_load.clone();
        tauri::async_runtime::spawn(async move {
            // Give the page a beat to finish writing auth cookies.
            tokio::time::sleep(Duration::from_millis(400)).await;
            let cookies = match collect_cookies(&window) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("login cookies: {e}");
                    return;
                }
            };
            if !session_ready_from_cookies(&cookies) {
                // Landed on YTM but not signed in (or still mid-redirect).
                eprintln!(
                    "login: on music.youtube.com, session not ready (cookies: {:?})",
                    cookie_names(&cookies)
                );
                return;
            }
            if done.swap(true, Ordering::SeqCst) {
                return;
            }
            let header = to_header(&cookies);
            let names = cookie_names(&cookies);
            eprintln!("login: session captured ({} cookies: {:?})", names.len(), names);
            on_session(header);
            let _ = window.clear_all_browsing_data();
            let _ = window.close();
            let _ = app.emit("auth", serde_json::json!({ "signed_in": true }));
        });
    });

    let win = builder.build().map_err(|e| anyhow!("login window: {e}"))?;
    let app_close = app.clone();
    let done_close = done.clone();
    win.on_window_event(move |ev| {
        if let tauri::WindowEvent::Destroyed = ev {
            if !done_close.load(Ordering::SeqCst) {
                let _ = app_close.emit("auth", serde_json::json!({ "signed_in": false, "cancelled": true }));
            }
        }
    });
    Ok(())
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
        .incognito(true)
        .build()
        .map_err(|e| anyhow!("{e}"))?;
    // Wait for a load, then read names.
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


fn cookie_name(c: &tauri::webview::cookie::Cookie<'_>) -> String { c.name().to_string() }
fn cookie_value(c: &tauri::webview::cookie::Cookie<'_>) -> String { c.value().to_string() }
fn cookie_domain(c: &tauri::webview::cookie::Cookie<'_>) -> String { c.domain().unwrap_or("").to_string() }
fn cookie_path(c: &tauri::webview::cookie::Cookie<'_>) -> String { c.path().unwrap_or("/").to_string() }
fn cookie_secure(c: &tauri::webview::cookie::Cookie<'_>) -> bool { c.secure().unwrap_or(true) }
