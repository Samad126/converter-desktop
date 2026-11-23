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

/// Deletes one app folder, retrying briefly: antivirus scanners and just-exited helper processes often hold a file for a moment.
fn purge_retry(d: &Path) -> Result<usize, String> {
    let mut last = purge(std::slice::from_ref(&d.to_path_buf()));
    for _ in 0..5 {
        if last.is_ok() { break; }
        std::thread::sleep(std::time::Duration::from_millis(250));
        last = purge(std::slice::from_ref(&d.to_path_buf()));
    }
    last
}

/// Windows only: files in use cannot be deleted by their own process. Hand the leftovers to a hidden `cmd` that waits for this
/// app to exit and then deletes them. Only folders already vetted by `is_app_dir` are ever passed in.
#[cfg(windows)]
fn delete_after_exit(dirs: &[PathBuf]) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let mut script = String::from("ping -n 6 127.0.0.1 >nul");
    for d in dirs { script.push_str(&format!(" & rmdir /s /q \"{}\"", d.display())); }
    std::process::Command::new("cmd").arg("/C").raw_arg(&script).creation_flags(0x0800_0008) // CREATE_NO_WINDOW | DETACHED_PROCESS
        .spawn().map(|_| ()).map_err(|e| format!("could not schedule the cleanup: {e}"))
}
#[cfg(not(windows))]
fn delete_after_exit(_: &[PathBuf]) -> Result<(), String> { Err("files are in use".into()) }

/// What the removal is doing right now, polled by the Settings page so a long uninstall does not look frozen.
pub static STATUS: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
fn set_status(s: String) { if let Ok(mut g) = STATUS.lock() { *g = s; } }

/// System-wide helper programs shown on the Engines page (found outside Converter's own folder) that can be uninstalled
/// through winget/brew/apt. The Settings dialog lists these before the user confirms.
pub fn system_tools() -> Vec<(&'static str, crate::sysinstall::Plan)> {
    let cfg = converter_core::Config::default();
    converter_core::engines::TOOLS.iter().filter_map(|t| {
        let found = converter_core::engines::resolve(t, &cfg, None)?;
        if found.starts_with(&cfg.engines_dir) { return None; } // our own copies go with the data folders
        Some((t.name, crate::sysinstall::uninstall_plan(t.hint())?))
    }).collect()
}

/// Uninstalls every tool from `system_tools`. Returns "" or a sentence naming what could not be removed.
fn uninstall_system_tools() -> String {
    let mut bad = Vec::new();
    let tools = system_tools();
    let n = tools.len();
    for (i, (name, plan)) in tools.into_iter().enumerate() {
        set_status(format!("Uninstalling {name} ({} of {n})… big programs can take a few minutes.", i + 1));
        if let Err(e) = crate::sysinstall::run(&plan) { bad.push(format!("{name} ({e})")); }
    }
    if bad.is_empty() { String::new() } else { format!(" Could not uninstall: {}.", bad.join("; ")) }
}

/// Everything the app wrote: the start-at-login entry, then the data folders. Folders with files still in use are
/// removed right after the app closes (Windows) instead of failing the whole operation.
pub fn remove_all_data() -> Result<Done, String> {
    let _ = crate::autostart::set(false);
    let leftover_note = uninstall_system_tools();
    set_status("Deleting settings, downloaded engines and temporary files…".into());
    let (mut removed, mut locked) = (0, Vec::new());
    for d in data_dirs() {
        match purge_retry(&d) {
            Ok(n) => removed += n,
            Err(e) if cfg!(windows) && d.exists() && is_app_dir(&d) => { let _ = e; locked.push(d); }
            Err(e) => return Err(e),
        }
    }
    if !locked.is_empty() {
        delete_after_exit(&locked)?;
        return Ok(Done { message: format!("Settings, engines and temporary files are being removed. A few were still in use and go as soon as Converter closes.{leftover_note} Closing."), quit: true });
    }
    Ok(Done { message: if removed == 0 && leftover_note.is_empty() { "There was no data to remove. Closing.".into() } else { format!("All settings, downloaded engines and temporary files were removed.{leftover_note} Closing.") }, quit: true })
}

/// Data plus the app. The app can delete its own files on Linux and macOS (the running program stays in memory);
/// on Windows the installer's uninstaller does it, because a running .exe cannot delete itself.
pub fn uninstall_app() -> Result<Done, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if cfg!(windows) {
        return match windows_uninstaller(&exe) {
            Some(u) => {
                let _ = crate::autostart::set(false);
                let note = uninstall_system_tools();
                std::process::Command::new(&u).spawn().map_err(|e| format!("Could not start the uninstaller: {e}"))?;
                Ok(Done { message: format!("Starting the uninstaller.{note} Closing."), quit: true })
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
