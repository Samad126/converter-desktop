//! Converter must never run elevated. An administrator process only receives drag-and-drop from other administrator
//! windows, so an elevated copy shows an empty Ctrl+drag ring and the drop zone refuses files. If it was started
//! elevated (for example by a setup run as administrator), start a normal copy through Explorer and exit.

/// True when this process runs with a full administrator token (a split-token account that said "yes" to UAC).
/// Accounts with UAC switched off, and standard users, are not "elevated" in this sense and are left alone.
#[cfg(windows)]
pub fn running_elevated() -> bool {
    use std::ffi::c_void;
    #[link(name = "advapi32")]
    extern "system" {
        fn OpenProcessToken(process: isize, access: u32, token: *mut isize) -> i32;
        fn GetTokenInformation(token: isize, class: i32, info: *mut c_void, len: u32, returned: *mut u32) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" { fn GetCurrentProcess() -> isize; fn CloseHandle(h: isize) -> i32; }
    const TOKEN_QUERY: u32 = 0x0008;
    const TOKEN_ELEVATION_TYPE: i32 = 18;
    const FULL: u32 = 2; // TokenElevationTypeFull
    unsafe {
        let mut token = 0isize;
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 { return false; }
        let (mut kind, mut returned) = (0u32, 0u32);
        let ok = GetTokenInformation(token, TOKEN_ELEVATION_TYPE, &mut kind as *mut u32 as *mut c_void, 4, &mut returned);
        CloseHandle(token);
        ok != 0 && kind == FULL
    }
}
#[cfg(not(windows))]
pub fn running_elevated() -> bool { false }

/// Starts an unelevated copy via Explorer (a child of an elevated process would inherit the elevation) and returns true if it was started.
/// Guarded by a short-lived stamp file so a machine where Explorer is elevated too can never relaunch in a loop.
#[cfg(windows)]
pub fn relaunch_unelevated() -> bool {
    let stamp = std::env::temp_dir().join("converter-relaunch.stamp");
    let recent = std::fs::metadata(&stamp).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|e| e.as_secs() < 15);
    if recent { return false; }
    let Ok(exe) = std::env::current_exe() else { return false };
    let _ = std::fs::write(&stamp, b"");
    std::process::Command::new("explorer.exe").arg(exe).spawn().is_ok()
}
#[cfg(not(windows))]
pub fn relaunch_unelevated() -> bool { false }

#[cfg(test)]
mod tests {
    #[test] fn checking_elevation_never_panics_and_is_false_off_windows() {
        let _ = super::running_elevated();
        #[cfg(not(windows))] assert!(!super::running_elevated() && !super::relaunch_unelevated());
    }
}
