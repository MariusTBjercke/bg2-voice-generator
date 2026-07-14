//! Exe-relative "portable" layout resolution.
//!
//! A shipped portable ZIP puts the vendored tools + OmniVoice engine next to the
//! `.exe`, with a writable `engine-runtime/` sibling for the venv + models:
//!
//! ```text
//! <root>/bg2-voice-generator.exe
//! <root>/engine/           omnivoice_server.py, requirements-*.txt   (shipped)
//! <root>/tools/            weidu.exe, ffmpeg.exe, ffprobe.exe, python/ (shipped)
//! <root>/engine-runtime/   created first run, WRITABLE: venv/ + models/
//! ```
//!
//! Detection is a **layout probe, not a build flag**: portable iff
//! `<exe_dir>/engine/omnivoice_server.py` exists. (`tauri::is_dev()` tracks the build
//! profile, not the on-disk layout, so a release build run from the repo would
//! misreport.) In dev / non-portable runs `runtime_root` is the caller's app-data dir.

use std::path::{Path, PathBuf};

/// The directory containing the running executable. No `canonicalize()` - on Windows
/// that yields a `\\?\` extended-length path some child processes mishandle, and a
/// portable ZIP has no symlinks to resolve.
fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
}

/// True when an env var is present AND non-blank. Lets a real user-set override win
/// over a portable-layout default.
pub fn env_is_set(name: &str) -> bool {
    std::env::var(name).ok().is_some_and(|s| !s.trim().is_empty())
}

/// Probe a given exe dir for the bundled OmniVoice engine script.
fn is_portable_at(exe_dir: Option<&Path>) -> bool {
    exe_dir
        .map(|d| d.join("engine").join("omnivoice_server.py").exists())
        .unwrap_or(false)
}

/// True when the app is running from a portable layout (bundled engine script sits
/// next to the exe). Cheap filesystem probe.
pub fn is_portable_layout() -> bool {
    is_portable_at(exe_dir().as_deref())
}

/// Resolved filesystem layout for the vendored tools + local engine.
///
/// `runtime_root` is ALWAYS concrete. The `omnivoice_script`, `weidu`, `ffmpeg`, and
/// `base_python` fields are `Some` only in a portable layout - they are the values
/// the tool/engine wrappers inject as portable defaults (an env override still wins).
#[derive(Debug, Clone)]
pub struct ToolLayout {
    pub portable: bool,
    pub exe_dir: Option<PathBuf>,
    /// Where the venv + models live: `<exe_dir>/engine-runtime` (portable) or the
    /// caller's app-data dir (dev).
    pub runtime_root: PathBuf,
    pub omnivoice_script: Option<PathBuf>,
    pub weidu: Option<PathBuf>,
    pub ffmpeg: Option<PathBuf>,
    pub base_python: Option<PathBuf>,
}

impl ToolLayout {
    /// Resolve the layout from the real exe location. `app_data` is used as
    /// `runtime_root` in dev / non-portable runs.
    pub fn resolve(app_data: &Path) -> Self {
        let exe = exe_dir();
        let portable = is_portable_at(exe.as_deref());
        build_layout(portable, exe, app_data)
    }

    /// The per-machine venv the installer builds under `runtime_root/venv`. Its
    /// interpreter has omnivoice/torch; the engine prefers it once installed.
    pub fn venv_python(&self) -> PathBuf {
        venv_python_in(&self.runtime_root)
    }

    /// The success marker the installer writes after a completed provision.
    pub fn installed_marker(&self) -> PathBuf {
        installed_marker_in(&self.runtime_root)
    }

    /// True iff the venv `.installed` marker exists (a cheap filesystem probe).
    pub fn engine_installed(&self) -> bool {
        self.installed_marker().exists()
    }
}

/// The venv interpreter path derived from a runtime root. Pure (no IO) for testing:
/// `venv/Scripts/python.exe` (Windows) | `venv/bin/python3` (unix).
fn venv_python_in(runtime_root: &Path) -> PathBuf {
    let venv = runtime_root.join("venv");
    if cfg!(windows) {
        venv.join("Scripts").join("python.exe")
    } else {
        venv.join("bin").join("python3")
    }
}

/// The installed-marker path derived from a runtime root. Pure (no IO) for testing.
fn installed_marker_in(runtime_root: &Path) -> PathBuf {
    runtime_root.join("venv").join(".installed")
}

/// The vendored binary under `tools/` (`Some` only when present).
fn tool_at(tools: &Path, win: &str, unix: &str) -> Option<PathBuf> {
    let p = tools.join(if cfg!(windows) { win } else { unix });
    p.exists().then_some(p)
}

/// The shipped base interpreter (python-build-standalone), extracted under
/// `tools/python/`. `Some` only when it actually exists.
fn base_python_at(tools: &Path) -> Option<PathBuf> {
    #[cfg(windows)]
    let p = tools.join("python").join("python.exe");
    #[cfg(not(windows))]
    let p = tools.join("python").join("bin").join("python3");
    p.exists().then_some(p)
}

/// Pure layout derivation (unit-tested without touching the real exe location).
fn build_layout(portable: bool, exe: Option<PathBuf>, app_data: &Path) -> ToolLayout {
    // Portable requires a known exe dir; fall back to dev/app-data otherwise.
    let portable = portable && exe.is_some();

    let runtime_root = match (portable, &exe) {
        (true, Some(dir)) => dir.join("engine-runtime"),
        _ => app_data.to_path_buf(),
    };

    let (omnivoice_script, weidu, ffmpeg, base_python) = if portable {
        let dir = exe.as_ref().unwrap();
        let tools = dir.join("tools");
        (
            Some(dir.join("engine").join("omnivoice_server.py")),
            tool_at(&tools, "weidu.exe", "weidu"),
            tool_at(&tools, "ffmpeg.exe", "ffmpeg"),
            base_python_at(&tools),
        )
    } else {
        (None, None, None, None)
    };

    ToolLayout {
        portable,
        exe_dir: exe,
        runtime_root,
        omnivoice_script,
        weidu,
        ffmpeg,
        base_python,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_layout_uses_app_data_and_no_overrides() {
        let l = build_layout(false, Some(PathBuf::from("/opt/app")), Path::new("/app/data"));
        assert!(!l.portable);
        assert_eq!(l.runtime_root, PathBuf::from("/app/data"));
        assert!(l.omnivoice_script.is_none());
        assert!(l.weidu.is_none());
    }

    #[test]
    fn portable_derives_exe_relative_runtime_root() {
        let l = build_layout(true, Some(PathBuf::from("/games/bg2vg")), Path::new("/app/data"));
        assert!(l.portable);
        let nrm = |p: &Path| p.to_string_lossy().replace('\\', "/");
        assert_eq!(nrm(&l.runtime_root), "/games/bg2vg/engine-runtime");
        assert_eq!(nrm(l.omnivoice_script.as_deref().unwrap()), "/games/bg2vg/engine/omnivoice_server.py");
    }

    #[test]
    fn portable_without_exe_dir_falls_back_to_dev() {
        let l = build_layout(true, None, Path::new("/app/data"));
        assert!(!l.portable, "no exe dir must defeat portable mode");
        assert_eq!(l.runtime_root, PathBuf::from("/app/data"));
    }

    #[test]
    fn venv_python_derives_under_runtime_root() {
        let nrm = |p: &Path| p.to_string_lossy().replace('\\', "/");
        let py = venv_python_in(Path::new("/games/bg2vg/engine-runtime"));
        let expected = if cfg!(windows) {
            "/games/bg2vg/engine-runtime/venv/Scripts/python.exe"
        } else {
            "/games/bg2vg/engine-runtime/venv/bin/python3"
        };
        assert_eq!(nrm(&py), expected);
    }

    #[test]
    fn installed_marker_derives_under_venv() {
        let nrm = |p: &Path| p.to_string_lossy().replace('\\', "/");
        let m = installed_marker_in(Path::new("/app/data"));
        assert_eq!(nrm(&m), "/app/data/venv/.installed");
    }

    #[test]
    fn layout_venv_helpers_match_runtime_root() {
        let l = build_layout(true, Some(PathBuf::from("/games/bg2vg")), Path::new("/app/data"));
        assert_eq!(l.venv_python(), venv_python_in(&l.runtime_root));
        assert_eq!(l.installed_marker(), installed_marker_in(&l.runtime_root));
        // No marker on disk under a test path => not installed.
        assert!(!l.engine_installed());
    }
}
