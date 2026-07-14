//! The managed OmniVoice subprocess supervisor (item-08).
//!
//! [`OmniVoiceEngine`] owns a lazily-booted local Python server (`engine/
//! omnivoice_server.py`) and talks to it over loopback HTTP. Boot is "adopt or
//! spawn": if a healthy server already answers on the port (a prior run, or a dev
//! server started by hand) we reuse it and DO NOT own its lifecycle; otherwise we
//! spawn one and kill it on shutdown. This keeps the engine a generation-time
//! dependency only (the WeiDU packs are native - see
//! `docs/adr/0001-native-weidu-export.md`).

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex as StdMutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use crate::error::AppError;
use crate::paths::{env_is_set, ToolLayout};
use crate::tts::install::{omnivoice_device_string, GpuChoice};

use super::omnivoice::HealthResp;

/// The default loopback port the server binds. Overridable via `OMNIVOICE_TTS_PORT`
/// (must match the value the Python server reads).
pub const DEFAULT_PORT: u16 = 8140;
/// How long a `/health` probe may take before we treat the server as down.
const HEALTH_TIMEOUT: Duration = Duration::from_millis(1500);
/// Overall budget for a freshly-spawned server to answer `/health` once.
const BOOT_TIMEOUT: Duration = Duration::from_secs(60);
/// Poll interval while waiting for a spawned server to come up.
const BOOT_POLL: Duration = Duration::from_millis(500);

/// Resolved launch config for the engine subprocess. Env overrides win over the
/// portable-layout defaults so a dev run can point at a hand-managed interpreter.
///
/// The interpreter is deliberately NOT cached here: it is re-resolved at spawn time
/// from the [`ToolLayout`] (see [`OmniVoiceEngine::spawn`]) so a just-finished in-app
/// install is picked up without restarting the app.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// The server script (`OMNIVOICE_SCRIPT`, else the portable/dev script path).
    pub script: PathBuf,
    /// The loopback port (`OMNIVOICE_TTS_PORT`, else [`DEFAULT_PORT`]).
    pub port: u16,
}

impl EngineConfig {
    /// Resolve the launch config from the tool layout + environment.
    pub fn resolve(tools: &ToolLayout) -> Self {
        EngineConfig {
            script: resolve_script(tools),
            port: resolve_port(std::env::var("OMNIVOICE_TTS_PORT").ok().as_deref()),
        }
    }

    /// The loopback base URL the HTTP client speaks to.
    pub fn base_url(&self) -> String {
        base_url_for(self.port)
    }
}

/// Parse the port from an optional env string, falling back to [`DEFAULT_PORT`] on
/// absent/blank/invalid input. Pure for unit testing.
pub fn resolve_port(env: Option<&str>) -> u16 {
    env.and_then(|s| s.trim().parse::<u16>().ok())
        .filter(|p| *p != 0)
        .unwrap_or(DEFAULT_PORT)
}

/// Build the loopback base URL for a port. Pure for unit testing.
pub fn base_url_for(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

/// The interpreter: `OMNIVOICE_PYTHON` override, else the installed venv python, else
/// the portable base python, else bare `python` (resolved on `PATH` at spawn time).
/// Re-evaluated on every [`OmniVoiceEngine::spawn`] so a fresh install is honored.
fn resolve_python(tools: &ToolLayout) -> PathBuf {
    let env_override = env_is_set("OMNIVOICE_PYTHON")
        .then(|| PathBuf::from(std::env::var("OMNIVOICE_PYTHON").unwrap()));
    let base = tools.base_python.as_ref().filter(|p| p.exists()).cloned();
    pick_python(env_override, tools.venv_python(), tools.engine_installed(), base)
}

/// Pure interpreter precedence (no IO): env override > installed venv > base python >
/// bare `python`. Extracted so the ladder is unit-testable without touching disk.
fn pick_python(
    env_override: Option<PathBuf>,
    venv_python: PathBuf,
    engine_installed: bool,
    base_python: Option<PathBuf>,
) -> PathBuf {
    if let Some(p) = env_override {
        return p;
    }
    if engine_installed {
        return venv_python;
    }
    if let Some(p) = base_python {
        return p;
    }
    PathBuf::from("python")
}

/// The server script: `OMNIVOICE_SCRIPT` override, else the portable script, else the
/// dev repo path (`engine/omnivoice_server.py` relative to the working dir).
fn resolve_script(tools: &ToolLayout) -> PathBuf {
    if env_is_set("OMNIVOICE_SCRIPT") {
        return PathBuf::from(std::env::var("OMNIVOICE_SCRIPT").unwrap());
    }
    if let Some(p) = tools.omnivoice_script.as_ref() {
        return p.clone();
    }
    PathBuf::from("engine").join("omnivoice_server.py")
}

/// A snapshot of engine state for the UI (mirror of `EngineStatus` in
/// `src/lib/types/index.ts`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EngineStatus {
    /// The server answered `/health` (process is up).
    pub running: bool,
    /// The model is loaded and `/synthesize` can run.
    pub ready: bool,
    /// The loopback base URL.
    pub base_url: String,
    /// The loaded model id, when the server reports one.
    pub model_id: Option<String>,
    /// The server's last load error, when the model failed to import/load.
    pub load_error: Option<String>,
    /// Whether THIS process spawned + owns the server (vs. an adopted one).
    pub owned: bool,
    /// Whether the per-machine venv carries the `.installed` marker (the in-app
    /// installer has completed). Lets the UI pick Install-vs-Start from a boolean
    /// instead of parsing `load_error`, and reflects a just-finished install without a
    /// restart. Independent of `running`/`ready` (a live adopted server may report
    /// `false` if it wasn't provisioned by this app's installer).
    pub installed: bool,
    /// The device the engine reports (e.g. `cuda:0`, `cpu`), when known.
    pub device: Option<String>,
    /// The CUDA device name, when the engine reports one.
    pub cuda_name: Option<String>,
    /// Whether the performance fork is active in the engine process.
    pub fork: Option<bool>,
}

/// Decide whether a `/health` probe means the server is already up (adopt) or we
/// must spawn one. Pure for unit testing: `Some(resp)` = a live server answered.
pub fn should_adopt(probe: &Option<HealthResp>) -> bool {
    matches!(probe, Some(h) if h.status == "ok")
}

/// The managed engine handle stored in `AppState`. Cheap to construct (no IO); the
/// server boots lazily on the first [`OmniVoiceEngine::ensure_ready`].
pub struct OmniVoiceEngine {
    config: EngineConfig,
    /// The resolved tool layout, kept so the interpreter can be re-resolved at spawn
    /// time (a just-finished install is honored without a restart) and so `status()`
    /// can probe the `.installed` marker afresh.
    tools: ToolLayout,
    http: reqwest::Client,
    /// The spawned child, when WE own it. `None` when never booted or adopted.
    child: Mutex<Option<Child>>,
    /// True once we have confirmed a healthy server (adopted or spawned).
    up: AtomicBool,
    /// Device string (`cuda:0` / `cpu`) passed to the next owned spawn via
    /// `OMNIVOICE_DEVICE`. Updated before `ensure_ready` from install settings.
    spawn_device: StdMutex<String>,
}

impl OmniVoiceEngine {
    /// Construct the handle from the resolved layout + shared HTTP client. No IO.
    pub fn new(tools: &ToolLayout, http: reqwest::Client) -> Self {
        OmniVoiceEngine {
            config: EngineConfig::resolve(tools),
            tools: tools.clone(),
            http,
            child: Mutex::new(None),
            up: AtomicBool::new(false),
            spawn_device: StdMutex::new(omnivoice_device_string(GpuChoice::Cuda).to_string()),
        }
    }

    /// Set the `OMNIVOICE_DEVICE` env for the next owned spawn. Call before
    /// `ensure_ready` when the install GPU choice is known.
    pub fn set_spawn_device(&self, gpu: GpuChoice) {
        if let Ok(mut guard) = self.spawn_device.lock() {
            *guard = omnivoice_device_string(gpu).to_string();
        }
    }

    /// The loopback base URL callers POST `/synthesize` to.
    pub fn base_url(&self) -> String {
        self.config.base_url()
    }

    /// Probe `/health` once. `Ok(None)` means nothing answered (server down);
    /// `Ok(Some(h))` is the parsed body. A malformed body surfaces as an error.
    async fn probe(&self) -> Result<Option<HealthResp>, AppError> {
        let url = format!("{}/health", self.config.base_url());
        match self.http.get(&url).timeout(HEALTH_TIMEOUT).send().await {
            Ok(resp) if resp.status().is_success() => Ok(Some(resp.json::<HealthResp>().await?)),
            Ok(_) => Ok(None),
            // A connection refused / timeout is the "server down" case, not an error.
            Err(_) => Ok(None),
        }
    }

    /// Ensure a healthy server is reachable, spawning + adopting as needed. Idempotent:
    /// a no-op once up. Holds the child lock across boot so two callers can't race two
    /// servers onto the same port. Returns the confirmed [`HealthResp`].
    pub async fn ensure_ready(&self) -> Result<HealthResp, AppError> {
        let mut child = self.child.lock().await;
        // Adopt an already-live server (ours from a prior call, or an external one).
        let probe = self.probe().await?;
        if should_adopt(&probe) {
            self.up.store(true, Ordering::SeqCst);
            return Ok(probe.unwrap());
        }
        // Nothing healthy: spawn one we own and wait for it to answer.
        if child.is_none() {
            *child = Some(self.spawn()?);
        }
        let deadline = std::time::Instant::now() + BOOT_TIMEOUT;
        loop {
            if let Some(h) = self.probe().await? {
                if h.status == "ok" {
                    self.up.store(true, Ordering::SeqCst);
                    return Ok(h);
                }
            }
            if std::time::Instant::now() >= deadline {
                return Err(AppError::Other(format!(
                    "OmniVoice server did not become healthy within {}s",
                    BOOT_TIMEOUT.as_secs()
                )));
            }
            tokio::time::sleep(BOOT_POLL).await;
        }
    }

    /// Spawn the Python server, passing the port via env (matching the script's
    /// `OMNIVOICE_TTS_PORT` read) and suppressing a console window on Windows.
    fn spawn(&self) -> Result<Child, AppError> {
        // Re-resolve the interpreter now so a just-finished in-app install (which wrote
        // the venv + `.installed` marker after startup) is honored without a restart.
        let python = resolve_python(&self.tools);
        let device = self
            .spawn_device
            .lock()
            .map(|g| g.clone())
            .unwrap_or_else(|_| omnivoice_device_string(GpuChoice::Cuda).to_string());
        let mut cmd = Command::new(&python);
        cmd.arg(&self.config.script)
            .arg("--port")
            .arg(self.config.port.to_string())
            .env("OMNIVOICE_TTS_PORT", self.config.port.to_string())
            .env("OMNIVOICE_DEVICE", device)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        no_window(&mut cmd);
        cmd.spawn().map_err(|e| {
            AppError::Other(format!(
                "failed to spawn OmniVoice server ({} {}): {e}",
                python.display(),
                self.config.script.display()
            ))
        })
    }

    /// Current engine status (a fresh probe). Never spawns.
    pub async fn status(&self) -> EngineStatus {
        let owned = self.child.lock().await.is_some();
        let installed = self.tools.engine_installed();
        let probe = self.probe().await.ok().flatten();
        match probe {
            Some(h) => EngineStatus {
                running: h.status == "ok",
                ready: h.ready,
                base_url: self.config.base_url(),
                model_id: h.model_id,
                load_error: h.load_error,
                owned,
                installed,
                device: h.device,
                cuda_name: h.cuda_name,
                fork: h.fork,
            },
            None => EngineStatus {
                running: false,
                ready: false,
                base_url: self.config.base_url(),
                model_id: None,
                load_error: None,
                owned,
                installed,
                device: None,
                cuda_name: None,
                fork: None,
            },
        }
    }

    /// Stop the server IF we own it (adopted servers are left alone). Safe to call
    /// when never started. Called on app exit and by the stop command.
    pub async fn shutdown(&self) {
        self.up.store(false, Ordering::SeqCst);
        let mut guard = self.child.lock().await;
        if let Some(mut child) = guard.take() {
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
    }
}

#[cfg(windows)]
fn no_window(cmd: &mut Command) {
    // `tokio::process::Command` exposes `creation_flags` directly on Windows.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn no_window(_cmd: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_port_defaults_on_absent_blank_or_invalid() {
        assert_eq!(resolve_port(None), DEFAULT_PORT);
        assert_eq!(resolve_port(Some("  ")), DEFAULT_PORT);
        assert_eq!(resolve_port(Some("not-a-port")), DEFAULT_PORT);
        assert_eq!(resolve_port(Some("0")), DEFAULT_PORT);
    }

    #[test]
    fn resolve_port_parses_a_valid_override() {
        assert_eq!(resolve_port(Some("8199")), 8199);
        assert_eq!(resolve_port(Some(" 8200 ")), 8200);
    }

    #[test]
    fn base_url_is_loopback_only() {
        assert_eq!(base_url_for(8140), "http://127.0.0.1:8140");
    }

    #[test]
    fn pick_python_prefers_env_override() {
        let got = pick_python(
            Some(PathBuf::from("/custom/python")),
            PathBuf::from("/rt/venv/bin/python3"),
            true,
            Some(PathBuf::from("/tools/python")),
        );
        assert_eq!(got, PathBuf::from("/custom/python"));
    }

    #[test]
    fn pick_python_uses_venv_only_when_installed() {
        let venv = PathBuf::from("/rt/venv/bin/python3");
        let base = PathBuf::from("/tools/python");
        // Marker present: venv wins over base.
        assert_eq!(
            pick_python(None, venv.clone(), true, Some(base.clone())),
            venv
        );
        // No marker: fall back to base python.
        assert_eq!(pick_python(None, venv, false, Some(base.clone())), base);
    }

    #[test]
    fn pick_python_falls_back_to_bare_python() {
        let got = pick_python(None, PathBuf::from("/rt/venv/bin/python3"), false, None);
        assert_eq!(got, PathBuf::from("python"));
    }

    #[test]
    fn should_adopt_only_a_healthy_server() {
        let ok = HealthResp {
            status: "ok".into(),
            ready: false,
            model_id: None,
            device: None,
            cuda_name: None,
            fork: None,
            load_error: None,
        };
        assert!(should_adopt(&Some(ok)));
        assert!(!should_adopt(&None));
        let bad = HealthResp {
            status: "starting".into(),
            ready: false,
            model_id: None,
            device: None,
            cuda_name: None,
            fork: None,
            load_error: None,
        };
        assert!(!should_adopt(&Some(bad)));
    }
}
