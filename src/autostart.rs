//! "Start with this computer". Linux: an XDG autostart entry that launches the app hidden in the tray.
//! Windows (registry Run key) and macOS (LaunchAgent) are not implemented yet.
use std::path::{Path, PathBuf};

pub const SUPPORTED: bool = cfg!(target_os = "linux");

fn entry_in(config_home: &Path) -> PathBuf { config_home.join("autostart").join("converter-desktop.desktop") }

fn desktop_entry(exe: &Path) -> String {
    // Exec needs quoting when the path has spaces; `%` must be doubled in .desktop files.
    let exe = exe.display().to_string().replace('%', "%%");
    let exec = if exe.contains(' ') { format!("\"{exe}\"") } else { exe };
    format!("[Desktop Entry]\nType=Application\nName=Converter\nComment=File converter (tray)\nExec={exec} --background\nTerminal=false\nX-GNOME-Autostart-enabled=true\n")
}

pub fn set_in(config_home: &Path, exe: &Path, on: bool) -> std::io::Result<()> {
    let file = entry_in(config_home);
    if on {
        std::fs::create_dir_all(file.parent().unwrap())?;
        std::fs::write(file, desktop_entry(exe))
    } else {
        match std::fs::remove_file(file) { Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e), _ => Ok(()) }
    }
}

pub fn is_enabled_in(config_home: &Path) -> bool { entry_in(config_home).is_file() }

/// Applies the setting for the real user. `Err` carries a sentence for the UI.
pub fn set(on: bool) -> Result<(), String> {
    if !SUPPORTED { return Err("Starting with the computer isn't available on this platform yet.".into()); }
    let home = dirs::config_dir().ok_or("Cannot find your config folder.")?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    set_in(&home, &exe, on).map_err(|e| format!("Could not change the autostart entry: {e}"))
}

pub fn is_enabled() -> bool { dirs::config_dir().is_some_and(|h| is_enabled_in(&h)) }

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
    #[test] fn paths_with_spaces_and_percent_are_escaped() {
        let t = desktop_entry(Path::new("/home/a b/100%/conv"));
        assert!(t.contains("Exec=\"/home/a b/100%%/conv\" --background"), "{t}");
    }
}
