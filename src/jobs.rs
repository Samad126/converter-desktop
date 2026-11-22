//! What both windows share: which targets a file offers, whether an engine is missing, and running a conversion.
use crate::runtime;
use converter_core::{engines, formats, progress::ProgressFn, Config, ConvertOptions};
use std::path::PathBuf;

/// (target id, label). Media targets carry a `media:` prefix so one string can name either matrix.
pub fn targets_for(ext: &str) -> Vec<(String, String)> {
    let m = formats::matrix();
    let mut v: Vec<(String, String)> = formats::targets_for(ext).iter().filter_map(|id| m.targets.get(id).map(|t| (id.clone(), t.label.clone()))).collect();
    for id in converter_core::media::media_targets_for(ext) { if let Some(t) = m.media.get(id) { v.push((format!("media:{id}"), t.label.clone())); } }
    v
}

/// Targets every one of `exts` can become (a mixed drop only offers what all files share).
pub fn common_targets(exts: &[String]) -> Vec<(String, String)> {
    let mut it = exts.iter();
    let Some(first) = it.next() else { return vec![] };
    let mut out = targets_for(first);
    for e in it { let t = targets_for(e); out.retain(|(id, _)| t.iter().any(|(x, _)| x == id)); }
    out
}

/// The engine a target needs, when it is not installed yet.
pub fn missing_engine(cfg: &Config, ext: &str, target: &str) -> Option<&'static str> {
    if target.starts_with("media:") {
        let t = engines::tool("ffmpeg")?;
        return engines::resolve(t, cfg, None).is_none().then_some(t.name);
    }
    // A conversion the built-in converters can do never asks for an install.
    converter_core::pipelines::missing_tool(cfg, &formats::resolve_conversion(ext, target)?)
}

/// Starts a conversion with the settings as they are right now (output folder, overwrite behaviour).
pub fn spawn(path: PathBuf, target: String, progress: ProgressFn) -> tokio::task::JoinHandle<converter_core::Result<PathBuf>> {
    let cfg = crate::settings::config();
    let out_dir = crate::settings::get().output_dir;
    runtime::get().spawn(async move {
        let opts = ConvertOptions { out_dir, ..ConvertOptions::default() };
        match target.strip_prefix("media:") {
            Some(id) => converter_core::media::convert_media(&cfg, &path, id, opts, progress).await,
            None => converter_core::convert_file(&cfg, &path, &target, opts).await.map(|mut v| v.remove(0)),
        }
    })
}

/// What the Install button calls an engine (the raw name for most; `python` is the bundled Python + packages).
pub fn engine_label(name: &str) -> String { if name == "python" { "Python tools".into() } else { name.to_string() } }

/// Shows `path` in the system file manager (selects it where the platform supports that).
pub fn reveal(path: &std::path::Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(path.parent().unwrap_or(path)).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn mixed_drop_offers_only_shared_targets() {
        let both = common_targets(&[".png".into(), ".jpg".into()]);
        assert!(both.iter().any(|(id, _)| id == "webp"));
        let mixed = common_targets(&[".png".into(), ".docx".into()]);
        let docx = targets_for(".docx");
        assert!(mixed.iter().all(|(id, _)| docx.iter().any(|(d, _)| d == id)));
        assert!(common_targets(&[]).is_empty());
    }
    #[test] fn audio_files_offer_media_targets() {
        assert!(targets_for(".wav").iter().any(|(id, _)| id == "media:mp3"));
    }
}

#[cfg(test)]
mod builtin_tests {
    use super::*;
    #[test] fn built_in_formats_are_offered() {
        let ids = |ext: &str| targets_for(ext).into_iter().map(|(id, _)| id).collect::<Vec<_>>();
        for t in ["pdf", "docx", "xlsx", "markdown", "txt", "hwpx"] { assert!(ids(".hwp").contains(&t.to_string()), "hwp -> {t}"); }
        for t in ["txt", "docx", "xlsx", "png", "tiff", "cbz", "pptx"] { assert!(ids(".pdf").contains(&t.to_string()), "pdf -> {t}"); }
        assert!(ids(".msg").contains(&"pdf".to_string()) && ids(".sbv").contains(&"srt".to_string()) && ids(".eot").contains(&"woff".to_string()));
        assert!(ids(".dng").contains(&"jpg-image".to_string()) && ids(".lz4").contains(&"zip".to_string()));
    }
    #[test] fn a_built_in_pair_never_asks_for_an_install() {
        // Even with an empty engines folder, nothing is missing for pairs the core can do itself.
        let cfg = Config { engines_dir: std::env::temp_dir().join("no-engines-desktop-test"), ..Config::default() };
        for (ext, t) in [(".hwp", "docx"), (".hwpx", "markdown"), (".msg", "txt"), (".sbv", "vtt"), (".lz4", "zip"), (".avro", "json"), (".dds", "png-image")] {
            assert_eq!(missing_engine(&cfg, ext, t), None, "{ext} -> {t}");
        }
    }
}
