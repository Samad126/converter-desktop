//! What both windows share: which targets a file offers, whether an engine is missing, and running a conversion.
use crate::runtime;
use converter_core::{engines, formats, progress::ProgressFn, Config, ConvertOptions};
use std::path::PathBuf;
use std::sync::Arc;

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
    let tool = if target.starts_with("media:") { "ffmpeg" } else { converter_core::pipelines::required_tool_for(&formats::resolve_conversion(ext, target)?)? };
    let t = engines::tool(tool)?;
    engines::resolve(t, cfg, None).is_none().then_some(t.name)
}

pub fn spawn(cfg: Arc<Config>, path: PathBuf, target: String, progress: ProgressFn) -> tokio::task::JoinHandle<converter_core::Result<PathBuf>> {
    runtime::get().spawn(async move {
        let opts = ConvertOptions::default();
        match target.strip_prefix("media:") {
            Some(id) => converter_core::media::convert_media(&cfg, &path, id, opts, progress).await,
            None => converter_core::convert_file(&cfg, &path, &target, opts).await.map(|mut v| v.remove(0)),
        }
    })
}

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
