//! Windows shell identity so toasts and the taskbar show "Tarn", not
//! "Windows PowerShell".

/// Same as the bundle identifier in tauri.conf.json. The NSIS/MSI
/// installers register a Start-menu shortcut with this AppUserModelID.
#[cfg_attr(not(windows), allow(dead_code))]
pub const APP_USER_MODEL_ID: &str = "app.tarn.desktop";
pub const APP_NAME: &str = "Tarn";

#[cfg(windows)]
pub fn init() {
    set_process_aumid();
    if let Some(icon) = crate::notify_icon() {
        register_aumid(APP_USER_MODEL_ID, APP_NAME, Some(&icon));
    } else {
        register_aumid(APP_USER_MODEL_ID, APP_NAME, None);
    }
}

#[cfg(not(windows))]
pub fn init() {}

#[cfg(windows)]
fn set_process_aumid() {
    #[link(name = "shell32")]
    extern "system" {
        fn SetCurrentProcessExplicitAppUserModelID(app_id: *const u16) -> i32;
    }
    let mut id: Vec<u16> = APP_USER_MODEL_ID.encode_utf16().collect();
    id.push(0);
    let hr = unsafe { SetCurrentProcessExplicitAppUserModelID(id.as_ptr()) };
    if hr != 0 {
        eprintln!("SetCurrentProcessExplicitAppUserModelID: HRESULT 0x{hr:08X}");
    }
}

#[cfg(windows)]
fn register_aumid(aumid: &str, display: &str, icon: Option<&str>) {
    use std::ptr;
    type Hkey = isize;
    const HKEY_CURRENT_USER: Hkey = 0x8000_0001u32 as Hkey;
    const KEY_SET_VALUE: u32 = 0x0002;
    const KEY_CREATE_SUB_KEY: u32 = 0x0004;
    const KEY_WRITE: u32 = 0x20000 | KEY_SET_VALUE | KEY_CREATE_SUB_KEY;
    const REG_OPTION_NON_VOLATILE: u32 = 0;
    const REG_SZ: u32 = 1;

    #[link(name = "advapi32")]
    extern "system" {
        fn RegCreateKeyExW(
            hkey: Hkey,
            sub: *const u16,
            reserved: u32,
            class: *const u16,
            options: u32,
            sam: u32,
            sec: *const u8,
            result: *mut Hkey,
            disposition: *mut u32,
        ) -> i32;
        fn RegSetValueExW(
            hkey: Hkey,
            name: *const u16,
            reserved: u32,
            ty: u32,
            data: *const u8,
            len: u32,
        ) -> i32;
        fn RegCloseKey(hkey: Hkey) -> i32;
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    let sub = wide(&format!(r"Software\Classes\AppUserModelId\{aumid}"));
    let mut key: Hkey = 0;
    let mut disp = 0u32;
    let err = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            sub.as_ptr(),
            0,
            ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            ptr::null(),
            &mut key,
            &mut disp,
        )
    };
    if err != 0 || key == 0 {
        eprintln!("register AppUserModelId: {err}");
        return;
    }
    let set = |name: &str, val: &str| {
        let n = wide(name);
        let v = wide(val);
        unsafe {
            RegSetValueExW(
                key,
                n.as_ptr(),
                0,
                REG_SZ,
                v.as_ptr() as *const u8,
                (v.len() * 2) as u32,
            )
        };
    };
    set("DisplayName", display);
    if let Some(path) = icon {
        let uri = if path.starts_with("file:") {
            path.to_string()
        } else {
            let n = path.replace('\\', "/");
            if n.starts_with('/') {
                format!("file://{n}")
            } else {
                format!("file:///{n}")
            }
        };
        set("IconUri", &uri);
        set("DesktopToastIconUri", &uri);
    }
    unsafe { RegCloseKey(key) };
}
