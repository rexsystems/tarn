#![allow(clippy::items_after_test_module)]
//! Session cookie storage and YouTube auth helpers.
//! Cookie values are never written to logs.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rand::RngCore;
use sha1::{Digest, Sha1};
use zeroize::Zeroize;

const SERVICE: &str = "app.tarn.desktop";
const ACCOUNT: &str = "ytm-session";
const ORIGIN: &str = "https://music.youtube.com";

/// A single cookie, values treated as secret.
#[derive(Clone)]
pub struct CookieRec {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub expires: Option<u64>,
}

impl std::fmt::Debug for CookieRec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CookieRec")
            .field("name", &self.name)
            .field("value", &"<redacted>")
            .field("domain", &self.domain)
            .field("path", &self.path)
            .field("secure", &self.secure)
            .finish()
    }
}

impl Drop for CookieRec {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}

/// Names only — safe to print or put in a test report.
pub fn cookie_names(cookies: &[CookieRec]) -> Vec<String> {
    cookies.iter().map(|c| c.name.clone()).collect()
}

pub fn is_youtube_related(domain: &str) -> bool {
    let d = domain.trim().trim_start_matches('.').to_ascii_lowercase();
    d.is_empty()
        || d == "youtube.com"
        || d.ends_with(".youtube.com")
        || d == "google.com"
        || d.ends_with(".google.com")
        || d == "youtube-nocookie.com"
        || d.ends_with(".youtube-nocookie.com")
        || d == "gstatic.com"
        || d.ends_with(".gstatic.com")
        || d == "ytimg.com"
        || d.ends_with(".ytimg.com")
}

pub fn session_ready_from_cookies(cookies: &[CookieRec]) -> bool {
    let has = |n: &str| cookies.iter().any(|c| c.name == n && !c.value.is_empty());
    has("SAPISID")
        || has("__Secure-3PAPISID")
        || has("__Secure-1PAPISID")
        || has("__Secure-1PSID")
        || has("__Secure-3PSID")
}

/// A session we trust enough to close the login window: a SAPISID
/// (for SAPISIDHASH) and a SID-family cookie.
pub fn session_complete_from_cookies(cookies: &[CookieRec]) -> bool {
    let has = |n: &str| cookies.iter().any(|c| c.name == n && !c.value.is_empty());
    let sapi = has("SAPISID") || has("__Secure-3PAPISID") || has("__Secure-1PAPISID");
    let sid = has("__Secure-1PSID") || has("__Secure-3PSID") || has("SID") || has("LOGIN_INFO");
    sapi && sid
}

pub fn session_ready_from_header(header: &str) -> bool {
    header.contains("SAPISID=")
        || header.contains("__Secure-3PAPISID=")
        || header.contains("__Secure-1PAPISID=")
        || header.contains("__Secure-1PSID=")
        || header.contains("__Secure-3PSID=")
}

pub fn session_complete_from_header(header: &str) -> bool {
    let sapi = header.contains("SAPISID=")
        || header.contains("__Secure-3PAPISID=")
        || header.contains("__Secure-1PAPISID=");
    let sid = header.contains("__Secure-1PSID=")
        || header.contains("__Secure-3PSID=")
        || header.contains("LOGIN_INFO=")
        || header.contains("SID=");
    sapi && sid
}

/// Cookie *names* only, from a Cookie request header.
pub fn header_cookie_names(header: &str) -> Vec<String> {
    let mut names = Vec::new();
    for part in header.split(';') {
        if let Some((k, _)) = part.split_once('=') {
            let k = k.trim();
            if !k.is_empty() {
                names.push(k.to_string());
            }
        }
    }
    names
}

/// Merge a `Set-Cookie` response value into an existing Cookie header.
/// Used so rotating SIDCC / __Secure-3PSIDCC stay fresh.
pub fn merge_set_cookie(header: &str, set_cookie: &str) -> String {
    let Some((nv, _)) = set_cookie.split_once(';').map(|(a, b)| (a, Some(b))).or_else(|| {
        set_cookie.split_once('=').map(|_| (set_cookie, None))
    }) else {
        return header.to_string();
    };
    let nv = nv.trim();
    let Some((name, _)) = nv.split_once('=') else {
        return header.to_string();
    };
    let name = name.trim();
    if name.is_empty() {
        return header.to_string();
    }
    let mut parts: Vec<String> = Vec::new();
    let mut replaced = false;
    for part in header.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((k, _)) = part.split_once('=') {
            if k.trim() == name {
                parts.push(nv.to_string());
                replaced = true;
                continue;
            }
        }
        parts.push(part.to_string());
    }
    if !replaced {
        parts.push(nv.to_string());
    }
    parts.join("; ")
}

/// Cookie request header (`Name=Value; Name2=Value2`).
pub fn to_header(cookies: &[CookieRec]) -> String {
    let mut seen = std::collections::HashSet::new();
    let mut parts = Vec::new();
    for c in cookies {
        if !is_youtube_related(&c.domain) {
            continue;
        }
        if c.name.is_empty() || !seen.insert(c.name.clone()) {
            continue;
        }
        parts.push(format!("{}={}", c.name, c.value));
    }
    parts.join("; ")
}

/// Netscape HTTP cookie file (for yt-dlp `--cookies`).
pub fn to_netscape(cookies: &[CookieRec]) -> String {
    let mut out = String::from("# Netscape HTTP Cookie File\n# Generated by Tarn. Do not share.\n");
    for c in cookies {
        if !is_youtube_related(&c.domain) || c.name.is_empty() {
            continue;
        }
        let domain = if c.domain.is_empty() {
            ".youtube.com"
        } else if c.domain.starts_with('.') {
            &c.domain
        } else {
            // host-only vs subdomain: prefix a dot for google/youtube
            // Netscape: leading dot means include subdomains.
            &c.domain
        };
        let include_sub = domain.starts_with('.');
        let domain_out = if include_sub {
            domain.to_string()
        } else if domain.starts_with("youtube.") || domain.starts_with("google.")
            || domain.ends_with(".youtube.com")
            || domain.ends_with(".google.com")
            || domain == "youtube.com"
            || domain == "google.com"
        {
            format!(".{domain}")
        } else {
            domain.to_string()
        };
        let flag = if domain_out.starts_with('.') { "TRUE" } else { "FALSE" };
        let path = if c.path.is_empty() { "/" } else { &c.path };
        let secure = if c.secure { "TRUE" } else { "FALSE" };
        let exp = c.expires.unwrap_or(0);
        // tab-separated, no leading comment
        let line = format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            domain_out, flag, path, secure, exp, c.name, c.value
        );
        out.push_str(&line);
    }
    out
}

pub fn sapisid_from_header(header: &str) -> Option<String> {
    for key in ["__Secure-3PAPISID", "__Secure-1PAPISID", "SAPISID"] {
        if let Some(v) = header_value(header, key) {
            return Some(v);
        }
    }
    None
}

pub fn header_value(header: &str, name: &str) -> Option<String> {
    for part in header.split(';') {
        let part = part.trim();
        if let Some((k, v)) = part.split_once('=') {
            if k.trim() == name && !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// YouTube `PREF` cookie carries the account's `hl` / `gl` (e.g. `hl=ro&gl=RO`).
pub fn locale_from_pref(header: &str) -> Option<(String, String)> {
    let pref = header_value(header, "PREF")?;
    let mut hl = None;
    let mut gl = None;
    for part in pref.split('&') {
        let part = part.trim();
        if let Some((k, v)) = part.split_once('=') {
            let v = v.trim();
            if v.is_empty() {
                continue;
            }
            match k.trim() {
                "hl" => hl = Some(v.to_string()),
                "gl" => gl = Some(v.to_string()),
                _ => {}
            }
        }
    }
    match (hl, gl) {
        (Some(h), Some(g)) => Some((h, g)),
        (Some(h), None) => Some((h, "US".into())),
        _ => None,
    }
}


/// `(timestamp, Authorization header value)` e.g. `SAPISIDHASH 123_abc`.
pub fn sapisidhash(sapisid: &str, origin: &str, timestamp: u64) -> String {
    let mut hasher = Sha1::new();
    hasher.update(format!("{timestamp} {sapisid} {origin}").as_bytes());
    let digest = hasher.finalize();
    format!("SAPISIDHASH {timestamp}_{}", hex::encode(digest))
}

pub fn sapisidhash_now(sapisid: &str) -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    sapisidhash(sapisid, ORIGIN, ts)
}


/// Encrypted / keyring session store. Never logs cookie values.
pub struct SessionStore {
    data_dir: PathBuf,
}

impl SessionStore {
    pub fn new() -> Result<Self> {
        let data_dir = dirs::data_dir()
            .or_else(|| dirs::home_dir().map(|h| h.join(".local/share")))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("tarn");
        fs::create_dir_all(&data_dir)?;
        restrict_mode(&data_dir, 0o700);
        Ok(Self { data_dir })
    }

    pub fn save(&self, cookie_header: &str) -> Result<StoreKind> {
        if cookie_header.is_empty() {
            return Err(anyhow!("empty session"));
        }
        // Always write the encrypted file so a Windows keyring glitch
        // (or a later failed read) does not log the user out.
        let file_ok = self.save_file(cookie_header);
        match keyring_set(cookie_header) {
            Ok(()) => {
                if let Err(e) = &file_ok {
                    eprintln!("session file backup: {e}");
                }
                Ok(StoreKind::Keyring)
            }
            Err(e) => {
                eprintln!("keyring save: {e}; using encrypted file");
                file_ok?;
                Ok(StoreKind::EncryptedFile)
            }
        }
    }

    pub fn load(&self) -> Result<Option<(String, StoreKind)>> {
        match keyring_get() {
            Ok(Some(s)) if !s.is_empty() => return Ok(Some((s, StoreKind::Keyring))),
            _ => {}
        }
        if let Some(s) = self.load_file()? {
            return Ok(Some((s, StoreKind::EncryptedFile)));
        }
        Ok(None)
    }

    pub fn clear(&self) -> Result<()> {
        let _ = keyring_delete();
        let _ = fs::remove_file(self.session_path());
        Ok(())
    }

    fn session_path(&self) -> PathBuf {
        self.data_dir.join("session.bin")
    }
    fn key_path(&self) -> PathBuf {
        self.data_dir.join("master.key")
    }

    fn load_or_create_key(&self) -> Result<[u8; 32]> {
        let p = self.key_path();
        if p.exists() {
            let mut buf = [0u8; 32];
            let mut f = fs::File::open(&p)?;
            f.read_exact(&mut buf)?;
            return Ok(buf);
        }
        let mut key = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut key);
        atomic_write_0600(&p, &key)?;
        Ok(key)
    }

    fn save_file(&self, plaintext: &str) -> Result<()> {
        let key = self.load_or_create_key()?;
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&key));
        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ct = cipher
            .encrypt(nonce, plaintext.as_bytes())
            .map_err(|e| anyhow!("encrypt: {e}"))?;
        let mut blob = Vec::with_capacity(12 + ct.len());
        blob.extend_from_slice(&nonce_bytes);
        blob.extend_from_slice(&ct);
        atomic_write_0600(&self.session_path(), &blob)
    }

    fn load_file(&self) -> Result<Option<String>> {
        let p = self.session_path();
        if !p.exists() {
            return Ok(None);
        }
        let blob = fs::read(&p)?;
        if blob.len() < 13 {
            return Ok(None);
        }
        let key = self.load_or_create_key()?;
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&key));
        let nonce = Nonce::from_slice(&blob[..12]);
        let pt = cipher
            .decrypt(nonce, &blob[12..])
            .map_err(|_| anyhow!("session file could not be decrypted"))?;
        Ok(Some(String::from_utf8(pt)?))
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreKind {
    Keyring,
    EncryptedFile,
}


#[cfg_attr(not(unix), allow(unused_variables))]
fn restrict_mode(path: &Path, mode: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::metadata(path) {
            let mut perms = meta.permissions();
            perms.set_mode(mode);
            let _ = fs::set_permissions(path, perms);
        }
    }
    let _ = mode;
}

fn atomic_write_0600(path: &Path, data: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    restrict_mode(&tmp, 0o600);
    fs::rename(&tmp, path)?;
    Ok(())
}

fn keyring_set(secret: &str) -> Result<()> {
    let entry = keyring::Entry::new(SERVICE, ACCOUNT).context("keyring")?;
    entry.set_password(secret).map_err(|e| anyhow!("{e}"))
}

fn keyring_get() -> Result<Option<String>> {
    let entry = keyring::Entry::new(SERVICE, ACCOUNT).context("keyring")?;
    match entry.get_password() {
        Ok(s) => Ok(Some(s)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(anyhow!("{e}")),
    }
}

fn keyring_delete() -> Result<()> {
    let entry = keyring::Entry::new(SERVICE, ACCOUNT).context("keyring")?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(anyhow!("{e}")),
    }
}

/// Write a netscape cookie file with 0600 perms for yt-dlp.
pub fn write_netscape_file(path: &Path, cookies: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    atomic_write_0600(path, cookies.as_bytes())
}

pub fn remove_file_quiet(path: &Path) {
    let _ = fs::remove_file(path);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(name: &str, value: &str, domain: &str) -> CookieRec {
        CookieRec {
            name: name.into(),
            value: value.into(),
            domain: domain.into(),
            path: "/".into(),
            secure: true,
            expires: Some(2000000000),
        }
    }

    #[test]
    fn locale_from_pref_parses() {
        let h = "SAPISID=x; PREF=tz=Europe.Bucharest&hl=ro&gl=RO; SID=y";
        assert_eq!(locale_from_pref(h), Some(("ro".into(), "RO".into())));
        assert_eq!(locale_from_pref("SAPISID=x"), None);
    }

    #[test]
    fn header_and_ready() {
        let cookies = vec![
            c("VISITOR_INFO1_LIVE", "abc", ".youtube.com"),
            c("SAPISID", "secret-sapi", ".youtube.com"),
            c("PREF", "f1=1", ".youtube.com"),
            c("unrelated", "x", ".example.com"),
        ];
        assert!(session_ready_from_cookies(&cookies));
        let h = to_header(&cookies);
        assert!(h.contains("SAPISID=secret-sapi"));
        assert!(h.contains("VISITOR_INFO1_LIVE=abc"));
        assert!(!h.contains("unrelated"));
        assert!(session_ready_from_header(&h));
        assert_eq!(cookie_names(&cookies), vec!["VISITOR_INFO1_LIVE", "SAPISID", "PREF", "unrelated"]);
    }

    #[test]
    fn not_ready_without_sapi() {
        let cookies = vec![c("YSC", "yy", ".youtube.com"), c("CONSENT", "YES", ".youtube.com")];
        assert!(!session_ready_from_cookies(&cookies));
        assert!(!session_ready_from_header(&to_header(&cookies)));
    }

    #[test]
    fn netscape_format() {
        let cookies = vec![c("SAPISID", "val", ".youtube.com")];
        let n = to_netscape(&cookies);
        assert!(n.contains("# Netscape"));
        assert!(n.contains(".youtube.com\tTRUE\t/\tTRUE\t2000000000\tSAPISID\tval"));
    }

    #[test]
    fn sapisidhash_known() {
        // SHA1("1000000000 s3cret https://music.youtube.com")
        let out = sapisidhash("s3cret", "https://music.youtube.com", 1_000_000_000);
        let expected = {
            let mut h = Sha1::new();
            h.update(b"1000000000 s3cret https://music.youtube.com");
            format!("SAPISIDHASH 1000000000_{}", hex::encode(h.finalize()))
        };
        assert_eq!(out, expected);
        assert!(out.starts_with("SAPISIDHASH 1000000000_"));
        assert_eq!(out.len(), "SAPISIDHASH 1000000000_".len() + 40);
    }

    #[test]
    fn file_store_roundtrip_and_clear() {
        let dir = std::env::temp_dir().join(format!("tarn-auth-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let store = SessionStore { data_dir: dir.clone() };
        let secret = "SAPISID=hello; __Secure-1PSID=world";
        store.save_file(secret).unwrap();
        let loaded = store.load_file().unwrap().unwrap();
        assert_eq!(loaded, secret);
        // file is not plaintext
        let raw = fs::read(store.session_path()).unwrap();
        let raw_s = String::from_utf8_lossy(&raw);
        assert!(!raw_s.contains("hello"));
        assert!(!raw_s.contains("SAPISID"));
        store.clear().unwrap();
        assert!(store.load_file().unwrap().is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn debug_redacts_value() {
        let cookie = c("SAPISID", "super-secret-value", ".youtube.com");
        let d = format!("{cookie:?}");
        assert!(d.contains("SAPISID"));
        assert!(!d.contains("super-secret-value"));
        assert!(d.contains("<redacted>"));
    }
}


/// Parse a Cookie request header into records (domain assumed `.youtube.com`).
pub fn cookies_from_header(header: &str) -> Vec<CookieRec> {
    let mut out = Vec::new();
    for part in header.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some((n, v)) = part.split_once('=') else {
            continue;
        };
        if n.is_empty() {
            continue;
        }
        out.push(CookieRec {
            name: n.trim().to_string(),
            value: v.trim().to_string(),
            domain: ".youtube.com".into(),
            path: "/".into(),
            secure: true,
            expires: None,
        });
    }
    out
}

pub fn netscape_from_header(header: &str) -> String {
    to_netscape(&cookies_from_header(header))
}

static PENDING_SET_COOKIES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

pub fn note_set_cookies(lines: &[String]) {
    if let Ok(mut g) = PENDING_SET_COOKIES.lock() {
        g.extend(lines.iter().cloned());
    }
}

pub fn take_set_cookies() -> Vec<String> {
    PENDING_SET_COOKIES
        .lock()
        .map(|mut g| g.drain(..).collect())
        .unwrap_or_default()
}
