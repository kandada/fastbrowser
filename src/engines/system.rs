// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! System / default browser discovery for the `system` engine and the `auto`
//! chain (desktop / pip). Only **CDP-capable** browsers (Chrome / Edge /
//! Chromium / Brave) are returned; Safari / Firefox are skipped (they do not
//! speak CDP).
//!
//! Discovery order:
//!   1. explicit `Config.browser_path`
//!   2. `CHROME_PATH`
//!   3. the platform **default browser** (http handler) when
//!      `Config.use_default_browser` and it is CDP-capable
//!   4. well-known installation paths for Chrome / Edge / Chromium / Brave
//!
//! All external access goes through [`SystemEnv`] so the logic is unit-testable
//! with a fake environment.

use std::path::{Path, PathBuf};

use crate::config::Config;

/// Injectable environment (filesystem / env vars / commands).
pub trait SystemEnv {
    fn os(&self) -> &'static str;
    fn file_exists(&self, p: &Path) -> bool;
    fn var(&self, key: &str) -> Option<String>;
    fn read(&self, p: &Path) -> Option<String>;
    fn run(&self, cmd: &str, args: &[&str]) -> Option<String>;
}

/// Real environment.
pub struct RealEnv;

impl SystemEnv for RealEnv {
    fn os(&self) -> &'static str {
        if cfg!(target_os = "macos") {
            "macos"
        } else if cfg!(target_os = "windows") {
            "windows"
        } else if cfg!(target_os = "linux") {
            "linux"
        } else {
            "other"
        }
    }
    fn file_exists(&self, p: &Path) -> bool {
        p.is_file() || p.is_dir()
    }
    fn var(&self, key: &str) -> Option<String> {
        std::env::var(key).ok().filter(|s| !s.trim().is_empty())
    }
    fn read(&self, p: &Path) -> Option<String> {
        std::fs::read_to_string(p).ok()
    }
    fn run(&self, cmd: &str, args: &[&str]) -> Option<String> {
        let out = std::process::Command::new(cmd).args(args).output().ok()?;
        if !out.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }
}

/// Is this path a CDP-drivable browser? (Chrome/Edge/Chromium/Brave, not
/// Safari/Firefox.)
pub fn is_cdp_capable(path: &str) -> bool {
    let l = path.to_ascii_lowercase();
    if l.contains("safari") || l.contains("firefox") {
        return false;
    }
    ["chrome", "chromium", "msedge", "microsoft edge", "brave"]
        .iter()
        .any(|k| l.contains(k))
}

/// Discover a CDP-capable system/default browser, honoring the config.
pub fn find_system_browser(config: &Config) -> Option<PathBuf> {
    find_system_browser_with(config, &RealEnv)
}

/// Like [`find_system_browser`] but with an injectable environment (tests).
pub fn find_system_browser_with(config: &Config, env: &dyn SystemEnv) -> Option<PathBuf> {
    // 1) explicit browser_path
    if let Some(p) = &config.browser_path {
        let pb = PathBuf::from(p);
        if env.file_exists(&pb) {
            return Some(pb);
        }
    }
    // 2) CHROME_PATH
    if let Some(p) = env.var("CHROME_PATH") {
        let pb = PathBuf::from(p);
        if env.file_exists(&pb) {
            return Some(pb);
        }
    }
    // 3) platform default browser (only when CDP-capable)
    if config.use_default_browser {
        if let Some(p) = default_browser(env)
            .filter(|p| is_cdp_capable(&p.to_string_lossy()) && env.file_exists(p))
        {
            return Some(p);
        }
    }
    // 4) well-known install paths
    known_paths(env)
        .into_iter()
        .find(|p| env.file_exists(p) && is_cdp_capable(&p.to_string_lossy()))
}

/// Well-known installation paths for CDP browsers.
fn known_paths(env: &dyn SystemEnv) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    match env.os() {
        "macos" => {
            let apps = [
                "Google Chrome.app/Contents/MacOS/Google Chrome",
                "Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
                "Brave Browser.app/Contents/MacOS/Brave Browser",
                "Chromium.app/Contents/MacOS/Chromium",
            ];
            for a in apps {
                out.push(PathBuf::from(format!("/Applications/{a}")));
            }
            if let Some(home) = env.var("HOME") {
                for a in apps {
                    out.push(PathBuf::from(format!("{home}/Applications/{a}")));
                }
            }
        }
        "windows" => {
            let pf = env
                .var("ProgramFiles")
                .unwrap_or_else(|| r"C:\Program Files".into());
            let pf86 = env
                .var("ProgramFiles(x86)")
                .unwrap_or_else(|| r"C:\Program Files (x86)".into());
            let local = env
                .var("LocalAppData")
                .unwrap_or_else(|| r"C:\Users\Public".into());
            let bins = [
                (r"Google\Chrome\Application\chrome.exe", true),
                (r"Microsoft\Edge\Application\msedge.exe", true),
                (r"Google\Chrome\Application\chrome.exe", false),
            ];
            out.push(PathBuf::from(format!(
                r"{pf}\Google\Chrome\Application\chrome.exe"
            )));
            out.push(PathBuf::from(format!(
                r"{pf86}\Google\Chrome\Application\chrome.exe"
            )));
            out.push(PathBuf::from(format!(
                r"{local}\Google\Chrome\Application\chrome.exe"
            )));
            out.push(PathBuf::from(format!(
                r"{pf}\Microsoft\Edge\Application\msedge.exe"
            )));
            out.push(PathBuf::from(format!(
                r"{pf86}\Microsoft\Edge\Application\msedge.exe"
            )));
            out.push(PathBuf::from(format!(
                r"{pf}\BraveSoftware\Brave-Browser\Application\brave.exe"
            )));
            let _ = bins;
        }
        "linux" => {
            for name in [
                "google-chrome",
                "google-chrome-stable",
                "chromium",
                "chromium-browser",
                "microsoft-edge",
                "microsoft-edge-stable",
                "brave-browser",
            ] {
                if let Some(p) = env.run("which", &[name]) {
                    out.push(PathBuf::from(p));
                }
            }
        }
        _ => {}
    }
    out
}

/// The platform's default http browser, resolved to an executable path (best
/// effort; may be a non-CDP browser — the caller filters).
pub fn default_browser(env: &dyn SystemEnv) -> Option<PathBuf> {
    match env.os() {
        "linux" => default_browser_linux(env),
        "macos" => default_browser_macos(env),
        "windows" => default_browser_windows(env),
        _ => None,
    }
}

fn default_browser_linux(env: &dyn SystemEnv) -> Option<PathBuf> {
    // `xdg-settings get default-web-browser` -> "google-chrome.desktop"
    let desktop = env.run("xdg-settings", &["get", "default-web-browser"])?;
    let exec = desktop_exec(env, desktop.trim())?;
    resolve_exec(env, &exec)
}

fn desktop_exec(env: &dyn SystemEnv, desktop_name: &str) -> Option<String> {
    let mut dirs: Vec<String> = vec![
        "/usr/share/applications".into(),
        "/usr/local/share/applications".into(),
        "/var/lib/flatpak/exports/share/applications".into(),
    ];
    if let Some(home) = env.var("HOME") {
        dirs.push(format!("{home}/.local/share/applications"));
        dirs.push(format!(
            "{home}/.local/share/flatpak/exports/share/applications"
        ));
    }
    for d in dirs {
        // Build with '/' explicitly: `.desktop` files live on Linux, and
        // `PathBuf::join` would use '\' on (non-Linux) CI hosts, so the path we
        // ask the env to read would not match its POSIX key.
        let joined = format!("{d}/{desktop_name}");
        let path = Path::new(&joined);
        if let Some(content) = env.read(path) {
            if let Some(exec) = content
                .lines()
                .find_map(|line| line.trim().strip_prefix("Exec=").map(|s| s.to_string()))
            {
                return Some(exec);
            }
        }
    }
    None
}

/// Resolve a desktop `Exec=` value (or a bare command) to an executable path.
fn resolve_exec(env: &dyn SystemEnv, exec: &str) -> Option<PathBuf> {
    // Drop field codes (%U, %F, ...) and take the first token.
    let first = exec
        .split_whitespace()
        .find(|t| !t.starts_with('%'))?
        .trim_matches('"');
    // POSIX absolute (a leading '/') is treated as absolute on every host, so
    // the `.desktop` resolution works identically in cross-OS tests.
    if first.starts_with('/') || PathBuf::from(first).is_absolute() {
        return Some(PathBuf::from(first));
    }
    env.run("which", &[first]).map(PathBuf::from)
}

fn default_browser_macos(env: &dyn SystemEnv) -> Option<PathBuf> {
    // Parse the LaunchServices handler plist (converted to JSON by plutil) for
    // the http handler's bundle id, then map it to an app path.
    let home = env.var("HOME")?;
    let plist = format!(
        "{home}/Library/Preferences/com.apple.LaunchServices/com.apple.launchservices.secure.plist"
    );
    let json = env.run("plutil", &["-convert", "json", "-o", "-", &plist])?;
    let v: serde_json::Value = serde_json::from_str(&json).ok()?;
    let handlers = v.get("LSHandlers")?.as_array()?;
    let mut bundle: Option<String> = None;
    for h in handlers {
        if h.get("LSHandlerURLScheme").and_then(|s| s.as_str()) == Some("http") {
            bundle = h
                .get("LSHandlerRoleAll")
                .or_else(|| h.get("LSHandlerRoleViewer"))
                .and_then(|s| s.as_str())
                .map(|s| s.to_string());
            if bundle.is_some() {
                break;
            }
        }
    }
    let bundle = bundle?;
    let app = match bundle.as_str() {
        "com.google.chrome" => "Google Chrome.app",
        "com.microsoft.edgemac" => "Microsoft Edge.app",
        "com.brave.Browser" => "Brave Browser.app",
        "org.chromium.Chromium" => "Chromium.app",
        _ => return None,
    };
    // The binary name is the app name without the ".app" suffix.
    let bin = app.trim_end_matches(".app");
    Some(PathBuf::from(format!(
        "/Applications/{app}/Contents/MacOS/{bin}"
    )))
}

fn default_browser_windows(env: &dyn SystemEnv) -> Option<PathBuf> {
    // ProgId for the http association.
    let out = env.run(
        "reg",
        &[
            "query",
            r"HKCU\Software\Microsoft\Windows\Shell\Associations\UrlAssociations\http\UserChoice",
            "/v",
            "ProgId",
        ],
    )?;
    let prog_id = out
        .lines()
        .find(|l| l.contains("REG_SZ"))
        .and_then(|l| l.split("REG_SZ").nth(1))
        .map(|s| s.trim().to_string())?;
    // Command line for that ProgId.
    let cmd_out = env.run(
        "reg",
        &[
            "query",
            &format!(r"HKCR\{prog_id}\shell\open\command"),
            "/ve",
        ],
    )?;
    let line = cmd_out.lines().find(|l| l.contains("REG_SZ"))?;
    let raw = line.split("REG_SZ").nth(1)?.trim();
    // First quoted token is the exe path.
    let exe = if let Some(rest) = raw.strip_prefix('"') {
        rest.split('"').next()?.to_string()
    } else {
        raw.split_whitespace().next()?.to_string()
    };
    Some(PathBuf::from(exe))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeEnv {
        os: &'static str,
        files: std::collections::HashMap<String, String>,
        cmds: std::collections::HashMap<String, String>,
    }
    impl SystemEnv for FakeEnv {
        fn os(&self) -> &'static str {
            self.os
        }
        fn file_exists(&self, p: &Path) -> bool {
            self.files.contains_key(&p.to_string_lossy().to_string())
        }
        fn var(&self, _k: &str) -> Option<String> {
            None
        }
        fn read(&self, p: &Path) -> Option<String> {
            self.files.get(&p.to_string_lossy().to_string()).cloned()
        }
        fn run(&self, cmd: &str, args: &[&str]) -> Option<String> {
            let key = format!("{cmd} {}", args.join(" "));
            self.cmds.get(&key).cloned()
        }
    }

    fn fake(os: &'static str) -> FakeEnv {
        FakeEnv {
            os,
            files: Default::default(),
            cmds: Default::default(),
        }
    }

    #[test]
    fn cdp_capability_filters_safari_firefox() {
        assert!(is_cdp_capable(
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
        ));
        assert!(is_cdp_capable(
            "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe"
        ));
        assert!(is_cdp_capable("/usr/bin/chromium"));
        assert!(!is_cdp_capable(
            "/Applications/Safari.app/Contents/MacOS/Safari"
        ));
        assert!(!is_cdp_capable("/usr/bin/firefox"));
    }

    #[test]
    fn explicit_browser_path_wins() {
        let mut e = fake("linux");
        e.files.insert("/opt/my/chrome".into(), "bin".into());
        let cfg = Config {
            browser_path: Some("/opt/my/chrome".into()),
            ..Config::default()
        };
        assert_eq!(
            find_system_browser_with(&cfg, &e),
            Some(PathBuf::from("/opt/my/chrome"))
        );
    }

    #[test]
    fn linux_default_browser_resolved_from_desktop_exec() {
        let mut e = fake("linux");
        e.cmds.insert(
            "xdg-settings get default-web-browser".into(),
            "google-chrome.desktop".into(),
        );
        e.files.insert(
            "/usr/share/applications/google-chrome.desktop".into(),
            "[Desktop Entry]\nName=Chrome\nExec=/usr/bin/google-chrome-stable %U\n".into(),
        );
        e.files
            .insert("/usr/bin/google-chrome-stable".into(), "bin".into());
        let cfg = Config::default();
        assert_eq!(
            find_system_browser_with(&cfg, &e),
            Some(PathBuf::from("/usr/bin/google-chrome-stable"))
        );
    }

    #[test]
    fn non_cdp_default_falls_back_to_known_paths() {
        let mut e = fake("linux");
        // Default is Firefox (not CDP) → skipped.
        e.cmds.insert(
            "xdg-settings get default-web-browser".into(),
            "firefox.desktop".into(),
        );
        e.files.insert(
            "/usr/share/applications/firefox.desktop".into(),
            "Exec=/usr/bin/firefox %u\n".into(),
        );
        e.files.insert("/usr/bin/firefox".into(), "bin".into());
        // A known CDP browser is installed.
        e.cmds
            .insert("which chromium".into(), "/usr/bin/chromium".into());
        e.files.insert("/usr/bin/chromium".into(), "bin".into());
        let cfg = Config::default();
        assert_eq!(
            find_system_browser_with(&cfg, &e),
            Some(PathBuf::from("/usr/bin/chromium"))
        );
    }

    #[test]
    fn use_default_browser_false_skips_default() {
        let mut e = fake("linux");
        e.cmds.insert(
            "xdg-settings get default-web-browser".into(),
            "google-chrome.desktop".into(),
        );
        e.files.insert(
            "/usr/share/applications/google-chrome.desktop".into(),
            "Exec=/usr/bin/google-chrome %U\n".into(),
        );
        e.files
            .insert("/usr/bin/google-chrome".into(), "bin".into());
        let cfg = Config {
            use_default_browser: false,
            ..Config::default()
        };
        // No known paths present and default disabled → none.
        assert_eq!(find_system_browser_with(&cfg, &e), None);
    }
}
