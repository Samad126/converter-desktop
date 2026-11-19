//! "Start with this computer". Linux: an XDG autostart entry; macOS: a LaunchAgent plist; Windows: a value under
//! `HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run`. All launch the app with `--background`. Everything is per user:
//! no administrator rights, and removing the entry (the toggle) undoes it completely.
use std::path::{Path, PathBuf};

pub const SUPPORTED: bool = cfg!(any(target_os = "linux", target_os = "macos", windows));

// ---- macOS: ~/Library/LaunchAgents/com.converter.desktop.plist
pub const MAC_LABEL: &str = "com.converter.desktop";

fn xml_escape(s: &str) -> String { s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;") }

pub fn launch_agent_plist(exe: &Path) -> String {
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n  <key>Label</key>\n  <string>{MAC_LABEL}</string>\n  <key>ProgramArguments</key>\n  <array>\n    <string>{}</string>\n    <string>--background</string>\n  </array>\n  <key>RunAtLoad</key>\n  <true/>\n</dict>\n</plist>\n", xml_escape(&exe.display().to_string()))
}

fn mac_agent_path(library: &Path) -> PathBuf { library.join("LaunchAgents").join(format!("{MAC_LABEL}.plist")) }

pub fn set_mac_in(library: &Path, exe: &Path, on: bool) -> std::io::Result<()> {
    let file = mac_agent_path(library);
    if on { std::fs::create_dir_all(file.parent().unwrap())?; std::fs::write(file, launch_agent_plist(exe)) }
    else { match std::fs::remove_file(file) { Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e), _ => Ok(()) } }
}

// ---- Windows: the Run key value is the quoted exe path plus the flag
pub fn windows_run_value(exe: &Path) -> String { format!("\"{}\" --background", exe.display()) }

#[cfg(windows)]
pub const WINDOWS_RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

#[cfg(windows)]
pub fn set_windows_in(subkey: &str, exe: &Path, on: bool) -> std::io::Result<()> {
    use winreg::{enums::*, RegKey};
    let key = RegKey::predef(HKEY_CURRENT_USER).create_subkey_with_flags(subkey, KEY_SET_VALUE | KEY_QUERY_VALUE)?.0;
    if on { key.set_value("Converter", &windows_run_value(exe)) }
    else { match key.delete_value("Converter") { Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e), _ => Ok(()) } }
}

#[cfg(windows)]
pub fn is_enabled_windows_in(subkey: &str) -> bool {
    use winreg::{enums::*, RegKey};
    RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(subkey, KEY_READ).and_then(|k| k.get_value::<String, _>("Converter")).is_ok()
}

// ---- Linux


fn entry_in(config_home: &Path) -> PathBuf { config_home.join("autostart").join("converter-desktop.desktop") }

fn desktop_entry(exe: &Path) -> String {
    // Exec needs quoting when the path has spaces; `%` must be doubled in .desktop files.
    let exe = exe.display().to_string().replace('%', "%%");
    let exec = if exe.contains(' ') { format!("\"{exe}\"") } else { exe };
    format!("[Desktop Entry]\nType=Application\nName=Converter\nComment=File converter (tray)\nExec={exec} --background\nTerminal=false\nX-GNOME-Autostart-enabled=true\n")
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn set_in(config_home: &Path, exe: &Path, on: bool) -> std::io::Result<()> {
    let file = entry_in(config_home);
    if on {
        std::fs::create_dir_all(file.parent().unwrap())?;
        std::fs::write(file, desktop_entry(exe))
    } else {
        match std::fs::remove_file(file) { Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e), _ => Ok(()) }
    }
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn is_enabled_in(config_home: &Path) -> bool { entry_in(config_home).is_file() }

/// Applies the setting for the real user. `Err` carries a sentence for the UI.
pub fn set(on: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    #[cfg(target_os = "linux")]
    { let home = dirs::config_dir().ok_or("Cannot find your config folder.")?; set_in(&home, &exe, on).map_err(|e| format!("Could not change the autostart entry: {e}")) }
    #[cfg(target_os = "macos")]
    { let lib = dirs::home_dir().ok_or("Cannot find your home folder.")?.join("Library"); set_mac_in(&lib, &exe, on).map_err(|e| format!("Could not change the login item: {e}")) }
    #[cfg(windows)]
    { set_windows_in(WINDOWS_RUN_KEY, &exe, on).map_err(|e| format!("Could not change the startup entry: {e}")) }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    { let _ = (exe, on); Err("Starting with the computer isn't available on this platform.".into()) }
}

pub fn is_enabled() -> bool {
    #[cfg(target_os = "linux")]
    { dirs::config_dir().is_some_and(|h| is_enabled_in(&h)) }
    #[cfg(target_os = "macos")]
    { dirs::home_dir().is_some_and(|h| mac_agent_path(&h.join("Library")).is_file()) }
    #[cfg(windows)]
    { is_enabled_windows_in(WINDOWS_RUN_KEY) }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    { false }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tmp(n: &str) -> PathBuf { let d = std::env::temp_dir().join(format!("autostart-{}-{n}", std::process::id())); let _ = std::fs::remove_dir_all(&d); d }

    #[test] fn enable_writes_a_hidden_start_entry_and_disable_removes_it() {
        let home = tmp("a");
        assert!(!is_enabled_in(&home));
        set_in(&home, Path::new("/opt/conv/converter-desktop"), true).unwrap();
        let text = std::fs::read_to_string(entry_in(&home)).unwrap();
        assert!(text.contains("Exec=/opt/conv/converter-desktop --background\n") && text.contains("[Desktop Entry]"));
        assert!(is_enabled_in(&home));
        set_in(&home, Path::new("/x"), false).unwrap();
        assert!(!is_enabled_in(&home));
        set_in(&home, Path::new("/x"), false).unwrap(); // disabling twice is fine
    }
    #[test] fn launch_agent_runs_at_login_with_the_background_flag_and_escapes_the_path() {
        let plist = launch_agent_plist(Path::new("/Applications/Conv & Co/converter-desktop"));
        assert!(plist.contains("<key>RunAtLoad</key>\n  <true/>") && plist.contains("<string>--background</string>"));
        assert!(plist.contains("/Applications/Conv &amp; Co/converter-desktop"), "{plist}");
        let lib = tmp("mac"); assert!(!mac_agent_path(&lib).exists());
        set_mac_in(&lib, Path::new("/x/y"), true).unwrap(); assert!(mac_agent_path(&lib).is_file());
        set_mac_in(&lib, Path::new("/x/y"), false).unwrap(); assert!(!mac_agent_path(&lib).exists());
        set_mac_in(&lib, Path::new("/x/y"), false).unwrap(); // idempotent
    }
    #[test] fn windows_run_value_is_quoted_so_spaces_survive() {
        assert_eq!(windows_run_value(Path::new(r"C:\Program Files\Conv\converter-desktop.exe")), r#""C:\Program Files\Conv\converter-desktop.exe" --background"#);
    }
    #[cfg(windows)]
    #[test] fn registry_entry_is_written_and_removed_under_a_private_test_key() {
        // A throwaway key, never the real Run key, so a test run cannot start anything at login.
        let key = format!(r"Software\ConverterDesktopTest-{}", std::process::id());
        assert!(!is_enabled_windows_in(&key));
        set_windows_in(&key, Path::new(r"C:\x\y.exe"), true).unwrap();
        assert!(is_enabled_windows_in(&key));
        set_windows_in(&key, Path::new(r"C:\x\y.exe"), false).unwrap();
        assert!(!is_enabled_windows_in(&key));
        let _ = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).delete_subkey_all(&key);
    }
    #[test] fn paths_with_spaces_and_percent_are_escaped() {
        let t = desktop_entry(Path::new("/home/a b/100%/conv"));
        assert!(t.contains("Exec=\"/home/a b/100%%/conv\" --background"), "{t}");
    }
}
