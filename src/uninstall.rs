//! "Remove all data" and "Uninstall": everything the app put on this computer, and where it can, the app itself.
//!
//! What counts as the app's data is fixed: a folder named `converter` inside the user's config, data and cache folders
//! (settings, downloaded engines, temp files). Folders redirected by `CONVERTER_*` environment variables are never touched,
//! and every delete refuses a path whose last component is not exactly `converter`.
//!   * Linux: removes the per-user install made by `install.sh` (`~/.local/share/converter-desktop` + the menu entry).
//!   * macOS: removes `Converter.app` (falls back to the Finder's Trash if the folder is not writable).
//!   * Windows: starts the installer's own `Uninstall.exe` (which asks about the data), or, for the portable zip, only the data is removed.
use std::path::{Path, PathBuf};

/// What the Uninstall button did, for the message shown before the app closes.
pub struct Done { pub message: String, pub quit: bool }

pub fn data_dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    for base in [dirs::config_dir(), dirs::data_dir(), dirs::cache_dir()].into_iter().flatten() {
        let p = base.join("converter");
        if !v.contains(&p) { v.push(p); }
    }
    v
}

fn is_app_dir(p: &Path) -> bool { p.file_name().is_some_and(|n| n == "converter") && p.components().count() >= 3 }

/// Deletes the given app folders. A symlink is unlinked, never followed. Returns how many existed, or what failed.
pub fn purge(dirs: &[PathBuf]) -> Result<usize, String> {
    let (mut removed, mut errors) = (0, Vec::new());
    for d in dirs {
        if !is_app_dir(d) { errors.push(format!("refused to delete {}", d.display())); continue; }
        match std::fs::symlink_metadata(d) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => errors.push(format!("{}: {e}", d.display())),
            Ok(m) => {
                let r = if m.file_type().is_symlink() { std::fs::remove_file(d).or_else(|_| std::fs::remove_dir(d)) } else { std::fs::remove_dir_all(d) };
                match r { Ok(()) => removed += 1, Err(e) => errors.push(format!("{}: {e}", d.display())) }
            }
        }
    }
    if errors.is_empty() { Ok(removed) } else { Err(errors.join("; ")) }
}

// ---- where the app itself lives, per platform (pure, so they are unit-tested everywhere)

/// Linux: the files `install.sh` created, if `exe` is the copy it installed.
pub fn linux_install_files(exe: &Path, data_home: &Path) -> Option<Vec<PathBuf>> {
    let dest = data_home.join("converter-desktop");
    exe.starts_with(&dest).then(|| vec![dest, data_home.join("applications").join("converter-desktop.desktop")])
}

/// macOS: the `.app` bundle containing `exe`.
pub fn mac_bundle(exe: &Path) -> Option<PathBuf> {
    exe.ancestors().find(|a| a.extension().is_some_and(|e| e == "app")).map(Path::to_path_buf)
}

/// Windows: the installer's uninstaller, next to the exe.
pub fn windows_uninstaller(exe: &Path) -> Option<PathBuf> {
    let p = exe.parent()?.join("Uninstall.exe");
    p.is_file().then_some(p)
}

fn remove_path(p: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(p) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => std::fs::remove_dir_all(p),
        Ok(_) => std::fs::remove_file(p),
    }
}

/// Everything the app wrote: the start-at-login entry, then the data folders.
pub fn remove_all_data() -> Result<Done, String> {
    let _ = crate::autostart::set(false);
    let n = purge(&data_dirs())?;
    Ok(Done { message: if n == 0 { "There was no data to remove. Closing.".into() } else { "All settings, downloaded engines and temporary files were removed. Closing.".into() }, quit: true })
}

/// Data plus the app. The app can delete its own files on Linux and macOS (the running program stays in memory);
/// on Windows the installer's uninstaller does it, because a running .exe cannot delete itself.
pub fn uninstall_app() -> Result<Done, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if cfg!(windows) {
        return match windows_uninstaller(&exe) {
            Some(u) => {
                let _ = crate::autostart::set(false);
                std::process::Command::new(&u).spawn().map_err(|e| format!("Could not start the uninstaller: {e}"))?;
                Ok(Done { message: "Starting the uninstaller. Closing.".into(), quit: true })
            }
            None => { let d = remove_all_data()?; Ok(Done { message: format!("{} This copy is the portable one: delete its folder to finish.", d.message.trim_end_matches(" Closing.")), quit: true }) }
        };
    }
    let data = remove_all_data()?;
    let app_note = if cfg!(target_os = "macos") {
        match mac_bundle(&exe) {
            Some(b) => match remove_path(&b) {
                Ok(()) => "The app was removed.".to_string(),
                Err(_) => match trash_in_finder(&b) { true => "The app was moved to the Trash.".into(), false => format!("Could not remove {}; drag it to the Trash.", b.display()) },
            },
            None => "This is not an app bundle; delete the program yourself.".into(),
        }
    } else {
        match dirs::data_dir().and_then(|h| linux_install_files(&exe, &h)) {
            Some(files) => { let bad: Vec<_> = files.iter().filter(|f| remove_path(f).is_err()).collect(); if bad.is_empty() { "The app was removed.".into() } else { "Could not remove every app file.".to_string() } }
            None => "This copy was not installed with install.sh: delete its folder to finish.".into(),
        }
    };
    Ok(Done { message: format!("{} {app_note} Closing.", data.message.trim_end_matches(" Closing.")), quit: true })
}

#[cfg(target_os = "macos")]
fn trash_in_finder(p: &Path) -> bool {
    let s = p.display().to_string().replace('\\', "\\\\").replace('"', "\\\"");
    std::process::Command::new("osascript").args(["-e", &format!("tell application \"Finder\" to delete POSIX file \"{s}\"")]).status().is_ok_and(|s| s.success())
}
#[cfg(not(target_os = "macos"))]
fn trash_in_finder(_: &Path) -> bool { false }

#[cfg(test)]
mod tests {
    use super::*;
    fn tmp(n: &str) -> PathBuf { let d = std::env::temp_dir().join(format!("uninst-{}-{n}", std::process::id())); let _ = std::fs::remove_dir_all(&d); std::fs::create_dir_all(&d).unwrap(); d }

    #[test] fn purge_removes_only_folders_named_converter() {
        let root = tmp("purge");
        let ok = root.join("converter"); std::fs::create_dir_all(ok.join("engines/x")).unwrap(); std::fs::write(ok.join("settings.json"), "{}").unwrap();
        let other = root.join("documents"); std::fs::create_dir_all(&other).unwrap(); std::fs::write(other.join("keep.txt"), "x").unwrap();
        assert_eq!(purge(&[ok.clone(), root.join("converter-missing").join("converter")]).unwrap(), 1);
        assert!(!ok.exists());
        assert!(purge(&[other.clone()]).is_err(), "a folder with another name is refused");
        assert!(other.join("keep.txt").exists());
        assert!(purge(&[PathBuf::from("/converter")]).is_err(), "too shallow");
    }
    #[test] fn purge_unlinks_a_symlink_without_touching_its_target() {
        #[cfg(unix)] {
            let root = tmp("link"); let target = root.join("precious"); std::fs::create_dir_all(&target).unwrap(); std::fs::write(target.join("f"), "x").unwrap();
            let link = root.join("converter"); std::os::unix::fs::symlink(&target, &link).unwrap();
            assert_eq!(purge(&[link.clone()]).unwrap(), 1);
            assert!(!link.exists() && target.join("f").exists());
        }
    }
    #[test] fn data_dirs_are_all_named_converter() { assert!(data_dirs().iter().all(|d| is_app_dir(d))); }
    #[test] fn linux_files_are_found_only_for_the_installed_copy() {
        let home = Path::new("/home/u/.local/share");
        let f = linux_install_files(Path::new("/home/u/.local/share/converter-desktop/converter-desktop"), home).unwrap();
        assert_eq!(f[0], home.join("converter-desktop")); assert_eq!(f[1], home.join("applications/converter-desktop.desktop"));
        assert!(linux_install_files(Path::new("/home/u/Downloads/converter-desktop/converter-desktop"), home).is_none());
        assert!(linux_install_files(Path::new("/home/u/.local/share/converter-desktop-evil/x"), home).is_none());
    }
    #[test] fn mac_bundle_is_the_app_folder() {
        assert_eq!(mac_bundle(Path::new("/Applications/Converter.app/Contents/MacOS/converter-desktop")).unwrap(), Path::new("/Applications/Converter.app"));
        assert!(mac_bundle(Path::new("/usr/local/bin/converter-desktop")).is_none());
    }
    #[test] fn windows_uninstaller_is_only_returned_when_present() {
        let d = tmp("win"); let exe = d.join("converter-desktop.exe");
        assert!(windows_uninstaller(&exe).is_none());
        std::fs::write(d.join("Uninstall.exe"), "x").unwrap();
        assert_eq!(windows_uninstaller(&exe).unwrap(), d.join("Uninstall.exe"));
    }
}
