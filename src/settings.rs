//! User settings: saved as JSON in the config folder, read by every job when it starts (so changes apply immediately).
use converter_core::Config;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};

/// Whether the Ctrl+drag ring reacts. Lives here (not in `Settings`) because the tray menu flips it without a UI round trip.
pub static RING_ENABLED: AtomicBool = AtomicBool::new(true);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// `None` = next to the original file.
    pub output_dir: Option<PathBuf>,
    /// Replace an existing result instead of writing `name (1).ext`.
    pub overwrite: bool,
    pub ring: bool,
    pub autostart: bool,
}

impl Default for Settings { fn default() -> Self { Self { output_dir: None, overwrite: false, ring: !cfg!(target_os = "macos"), autostart: false } } }

pub fn config_dir() -> PathBuf {
    std::env::var_os("CONVERTER_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("converter"))
}

fn file() -> PathBuf { config_dir().join("settings.json") }

impl Settings {
    /// Missing or unreadable file → defaults (a corrupt settings file must never stop the app from starting).
    pub fn load() -> Self { std::fs::read_to_string(file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default() }

    pub fn save(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(config_dir())?;
        // Write-then-rename so a crash mid-save cannot leave half a file.
        let tmp = file().with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).expect("settings serialise"))?;
        std::fs::rename(tmp, file())
    }
}

static STORE: RwLock<Option<Settings>> = RwLock::new(None);

pub fn get() -> Settings {
    if let Some(s) = STORE.read().unwrap().clone() { return s; }
    let s = Settings::load();
    *STORE.write().unwrap() = Some(s.clone());
    s
}

/// Changes the settings, saves them, and returns the new value.
pub fn update(f: impl FnOnce(&mut Settings)) -> Settings {
    let mut s = get();
    f(&mut s);
    if let Err(e) = s.save() { eprintln!("could not save settings: {e}"); }
    *STORE.write().unwrap() = Some(s.clone());
    s
}

/// A `Config` reflecting the current settings, for the job about to start.
pub fn config() -> Arc<Config> {
    Arc::new(Config { overwrite_existing: get().overwrite, ..Config::default() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] fn defaults_are_conservative() {
        let s = Settings::default();
        assert!(s.output_dir.is_none() && !s.overwrite && s.ring == !cfg!(target_os = "macos") && !s.autostart);
    }
    #[test] fn partial_or_unknown_json_still_loads() {
        let s: Settings = serde_json::from_str(r#"{"overwrite": true, "from_a_future_version": 1}"#).unwrap();
        assert!(s.overwrite && s.ring == Settings::default().ring, "missing keys fall back to defaults");
        assert!(serde_json::from_str::<Settings>("not json").is_err());
    }
    #[test] fn roundtrip_through_the_file() {
        let dir = std::env::temp_dir().join(format!("settings-test-{}", std::process::id()));
        std::env::set_var("CONVERTER_CONFIG_DIR", &dir);
        let s = Settings { output_dir: Some("/tmp/out".into()), overwrite: true, ring: false, autostart: true };
        s.save().unwrap();
        assert_eq!(Settings::load(), s);
        std::fs::write(dir.join("settings.json"), "{ broken").unwrap();
        assert_eq!(Settings::load(), Settings::default(), "a corrupt file falls back to defaults");
        std::env::remove_var("CONVERTER_CONFIG_DIR");
    }
}
