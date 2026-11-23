//! One-click install of helper tools that have no portable download, through the system package manager.
//! The core only knows a human-readable hint ("winget install --id X", "brew install x", "apt-get install -y x");
//! this turns that hint into a real command and runs it, so nobody has to open a terminal.
use std::process::{Command, Stdio};

#[derive(Debug, PartialEq)]
pub struct Plan { pub program: &'static str, pub args: Vec<String> }

/// Turns a hint into a runnable command; `None` for hints that are not a command ("no standard Windows build", pip, ...).
pub fn plan(hint: &str) -> Option<Plan> {
    let words: Vec<&str> = hint.split_whitespace().map(|w| w.trim_matches('"')).collect();
    let tail = |from: usize| words[from..].iter().map(|s| s.to_string());
    match words.as_slice() {
        ["winget", "install", ..] => Some(Plan { program: "winget", args: ["install".to_string()].into_iter().chain(tail(2))
            .chain(["-e", "--accept-package-agreements", "--accept-source-agreements"].map(String::from)).collect() }),
        ["brew", "install", ..] => Some(Plan { program: "brew", args: ["install".to_string()].into_iter().chain(tail(2)).collect() }),
        // apt needs root; pkexec shows the desktop's own password prompt.
        ["apt-get", "install", ..] => Some(Plan { program: "pkexec", args: ["apt-get".to_string(), "install".to_string()].into_iter().chain(tail(2)).collect() }),
        _ => None,
    }
}

/// The matching removal command for the same hint (what `plan` installed).
pub fn uninstall_plan(hint: &str) -> Option<Plan> {
    let words: Vec<&str> = hint.split_whitespace().map(|w| w.trim_matches('"')).collect();
    let tail = |from: usize| words[from..].iter().map(|s| s.to_string());
    match words.as_slice() {
        ["winget", "install", ..] => {
            // winget install flags that make no sense for uninstall are not carried over; only the package selector is.
            let id = words.iter().position(|w| *w == "--id").and_then(|i| words.get(i + 1))?;
            Some(Plan { program: "winget", args: ["uninstall", "--id", id, "-e", "--silent", "--accept-source-agreements"].map(String::from).to_vec() })
        }
        ["brew", "install", ..] => Some(Plan { program: "brew", args: ["uninstall".to_string()].into_iter().chain(tail(2)).collect() }),
        ["apt-get", "install", ..] => Some(Plan { program: "pkexec", args: ["apt-get".to_string(), "remove".to_string()].into_iter().chain(tail(2)).collect() }),
        _ => None,
    }
}

/// Where a person can download the tool by hand, for when there is no package-manager command or it failed.
/// Official project pages / release pages only.
pub fn download_page(name: &str) -> Option<&'static str> {
    Some(match name {
        "libreoffice" => "https://www.libreoffice.org/download/download-libreoffice/",
        "poppler" => if cfg!(windows) { "https://github.com/oschwartz10612/poppler-windows/releases" } else { "https://poppler.freedesktop.org/" },
        "pandoc" => "https://pandoc.org/installing.html",
        "qpdf" => "https://github.com/qpdf/qpdf/releases",
        "7zip" => "https://www.7-zip.org/download.html",
        "ffmpeg" => "https://ffmpeg.org/download.html",
        "yt-dlp" => "https://github.com/yt-dlp/yt-dlp/releases",
        "heif" => "https://github.com/strukturag/libheif",
        "zstd" => "https://github.com/facebook/zstd/releases",
        "assimp" => "https://github.com/assimp/assimp/releases",
        "calibre" => "https://calibre-ebook.com/download",
        "tesseract" => if cfg!(windows) { "https://github.com/UB-Mannheim/tesseract/wiki" } else { "https://tesseract-ocr.github.io/tessdoc/Installation.html" },
        "python" => "https://www.python.org/downloads/",
        _ => return None,
    })
}

/// Short name of what is doing the install, for the progress text.
pub fn manager_name(p: &Plan) -> &'static str { if p.program == "pkexec" { "apt" } else { p.program } }

/// Blocking: run from a worker thread. Window-less on Windows so no console flashes up.
/// `Ok(true)` = the command did its work; `Ok(false)` = there was nothing to do (already installed / already gone).
pub fn run(p: &Plan) -> Result<bool, String> {
    let candidates: &[&str] = if p.program == "brew" { &["brew", "/opt/homebrew/bin/brew", "/usr/local/bin/brew"] } else { &[p.program] };
    for prog in candidates {
        let mut c = Command::new(prog);
        c.args(&p.args).stdin(Stdio::null());
        #[cfg(windows)]
        { use std::os::windows::process::CommandExt; c.creation_flags(0x0800_0000); }
        match c.output() {
            Ok(o) if o.status.success() => return Ok(true),
            Ok(o) if nothing_to_do(o.status.code()) => return Ok(false),
            Ok(o) => {
                // A per-machine program cannot be installed/removed silently without administrator rights (MSI 1603, 740, 1625):
                // ask Windows for them once, with its normal permission prompt, instead of just failing.
                let text = format!("{}
{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
                if cfg!(windows) && p.program == "winget" && needs_admin(&text) { return run_elevated(p); }
                return Err(failure_text(&o.stdout, &o.stderr));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    Err(format!("{} is not available on this computer, so it cannot be installed automatically.", manager_name(p)))
}

/// The installer reported a failure that is what a missing administrator token looks like.
fn needs_admin(output: &str) -> bool {
    let o = output.to_lowercase();
    ["exit code: 1603", "exit code: 740", "exit code: 1625", "requires elevation", "administrator"].iter().any(|k| o.contains(k))
}

/// PowerShell one-liner that runs `winget <args>` elevated (UAC prompt), hidden, waits, and exits with winget's code.
/// 7777 means the prompt was declined or the program could not start.
fn elevated_script(p: &Plan) -> String {
    let args = p.args.iter().map(|a| format!("\"{}\"", a.replace('"', ""))).collect::<Vec<_>>().join(",");
    format!("try {{ $p = Start-Process -FilePath '{}' -ArgumentList {args} -Verb RunAs -PassThru -WindowStyle Hidden; $p.WaitForExit(); exit $p.ExitCode }} catch {{ exit 7777 }}", p.program)
}

#[cfg(windows)]
fn run_elevated(p: &Plan) -> Result<bool, String> {
    use std::os::windows::process::CommandExt;
    let o = Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", &elevated_script(p)]).stdin(Stdio::null())
        .creation_flags(0x0800_0000).output().map_err(|e| e.to_string())?;
    match o.status.code() {
        Some(0) => Ok(true),
        c if nothing_to_do(c) => Ok(false),
        Some(7777) => Err("Administrator permission was not granted.".into()),
        c => Err(format!("It still failed with administrator permission (code {}).", c.map(|c| (c as u32).to_string()).unwrap_or_else(|| "?".into()))),
    }
}
#[cfg(not(windows))]
fn run_elevated(_: &Plan) -> Result<bool, String> { Err("Administrator permission is required.".into()) }

/// winget exits non-zero when the package is already there ("No available upgrade found", "already installed").
/// That is not a failure for us: the tool is on disk and the caller just rescans for it.
fn nothing_to_do(code: Option<i32>) -> bool {
    // 0x8A15002B no upgrade, 0x8A150061 already installed, 0x8A150014 no package found (already uninstalled)
    matches!(code.map(|c| c as u32), Some(0x8A15_002B | 0x8A15_0061 | 0x8A15_0014))
}

/// Last couple of meaningful output lines (package managers print spinners and progress bars first).
fn failure_text(stdout: &[u8], stderr: &[u8]) -> String {
    let all = format!("{}\n{}", String::from_utf8_lossy(stdout), String::from_utf8_lossy(stderr));
    let lines: Vec<&str> = all.lines().map(str::trim).filter(|l| l.chars().filter(|c| c.is_alphabetic()).count() >= 6).collect();
    let tail = &lines[lines.len().saturating_sub(2)..];
    if tail.is_empty() { "The installer failed without saying why.".into() } else { tail.join(" ") }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn winget_hint_becomes_a_silent_exact_install() {
        let p = plan("winget install --id QPDF.QPDF").unwrap();
        assert_eq!(p.program, "winget");
        assert_eq!(p.args, ["install", "--id", "QPDF.QPDF", "-e", "--accept-package-agreements", "--accept-source-agreements"]);
    }
    #[test] fn brew_and_apt_hints() {
        assert_eq!(plan("brew install --cask calibre").unwrap(), Plan { program: "brew", args: vec!["install".into(), "--cask".into(), "calibre".into()] });
        let a = plan("apt-get install -y libreoffice-writer libreoffice-calc").unwrap();
        assert_eq!((a.program, a.args.len()), ("pkexec", 5));
        assert_eq!(&a.args[..3], ["apt-get", "install", "-y"]);
    }
    #[test] fn uninstall_plans_mirror_the_install_hints() {
        let w = uninstall_plan("winget install --id QPDF.QPDF").unwrap();
        assert_eq!((w.program, w.args.as_slice()), ("winget", ["uninstall", "--id", "QPDF.QPDF", "-e", "--silent", "--accept-source-agreements"].map(String::from).as_slice()));
        assert_eq!(uninstall_plan("brew install --cask calibre").unwrap().args, ["uninstall", "--cask", "calibre"]);
        assert_eq!(uninstall_plan("apt-get install -y a b").unwrap().args, ["apt-get", "remove", "-y", "a", "b"]);
        assert!(uninstall_plan("no standard Windows build").is_none());
    }
    #[test] fn every_known_tool_has_an_https_download_page() {
        for t in converter_core::engines::TOOLS { assert!(download_page(t.name).is_some_and(|u| u.starts_with("https://")), "no page for {}", t.name); }
        assert!(download_page("nope").is_none());
    }
    #[test] fn admin_failures_are_recognised() {
        assert!(needs_admin("Starting package uninstall...
Uninstall failed with exit code: 1603"));
        assert!(!needs_admin("No package found matching input criteria."));
    }
    #[test] fn elevated_script_quotes_every_argument() {
        let s = elevated_script(&plan("winget install --id 7zip.7zip").unwrap());
        assert!(s.contains("-Verb RunAs") && s.contains("\"install\",\"--id\",\"7zip.7zip\""), "{s}");
    }
    #[test] fn non_commands_have_no_plan() {
        assert!(plan("no standard Windows build").is_none());
        assert!(plan("no standard winget package").is_none());
        assert!(plan("pip install \"yt-dlp[default]\"").is_none());
    }
    #[test] fn winget_already_installed_codes_count_as_success() {
        assert!(nothing_to_do(Some(0x8A15_002Bu32 as i32)) && nothing_to_do(Some(0x8A15_0061u32 as i32)) && nothing_to_do(Some(0x8A15_0014u32 as i32)));
        assert!(!nothing_to_do(Some(1)) && !nothing_to_do(None));
    }
    #[test] fn failure_text_skips_spinner_noise() {
        let out = b"\\\r - \r  \nFound QPDF [QPDF.QPDF]\nNo applicable installer found for this system\n";
        assert_eq!(failure_text(out, b""), "Found QPDF [QPDF.QPDF] No applicable installer found for this system");
    }
}
