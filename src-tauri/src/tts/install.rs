//! OmniVoice engine provisioner (in-app installer core).
//!
//! Builds the per-machine venv under `runtime_root/venv`, installs the pinned deps
//! (torch/torchaudio + omnivoice), warms the HuggingFace model cache, and writes the
//! `.installed` marker resolve_python (item-01) then keys off. This module is
//! deliberately Tauri-free (ADR 0003): [`plan_install`] is a pure argv planner and
//! [`run_install`] takes a plain progress callback + a [`CancelToken`], so the Tauri
//! command in `commands/generate.rs` (item-04) can bridge it to the event bus without
//! any engine logic leaking into the frontend.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::commands::progress::CancelToken;
use crate::error::AppError;
use crate::paths::ToolLayout;

/// The provisioning steps, in execution order. The first five map to spawned
/// subprocesses via [`plan_install`]; `Finalize` is done in Rust (write the marker).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallStep {
    CreateVenv,
    UpgradePip,
    InstallTorch,
    InstallOmnivoice,
    DownloadModel,
    Finalize,
}

impl InstallStep {
    /// Every step in order (the fixed count item-04 renders a determinate bar over).
    pub const ALL: [InstallStep; 6] = [
        InstallStep::CreateVenv,
        InstallStep::UpgradePip,
        InstallStep::InstallTorch,
        InstallStep::InstallOmnivoice,
        InstallStep::DownloadModel,
        InstallStep::Finalize,
    ];
}

/// Which torch build to install. `Auto` is resolved to `Cpu`/`Cuda` by
/// [`resolve_gpu_choice`] (via [`detect_gpu`]); `Cpu`/`Cuda` are explicit overrides
/// (the `omnivoice_install_gpu` setting, read by the item-04 command).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuChoice {
    Auto,
    Cpu,
    Cuda,
}

impl GpuChoice {
    /// Parse the `omnivoice_install_gpu` setting token (`auto|cpu|cuda`, any case);
    /// anything else (including unset) falls back to `Auto`.
    pub fn from_setting(value: Option<&str>) -> GpuChoice {
        match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            Some("cpu") => GpuChoice::Cpu,
            Some("cuda") => GpuChoice::Cuda,
            _ => GpuChoice::Auto,
        }
    }
}

/// One planned subprocess: the step it belongs to, the program, and its argv.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepSpec {
    pub step: InstallStep,
    pub program: PathBuf,
    pub args: Vec<String>,
}

/// Outcome of a provisioning run (mapped to the IPC `InstallResult` in item-04).
#[derive(Debug, Clone)]
pub struct InstallReport {
    pub installed_python: PathBuf,
    pub steps_run: Vec<InstallStep>,
    pub skipped: bool,
}

/// torch/torchaudio version, shared by the CPU and CUDA install paths. Verify on release.
const TORCH_VER: &str = "2.8.0";
/// CUDA wheel tag for the GPU torch build. cu128 / torch 2.8 includes Blackwell
/// kernels (RTX 50-series) while still running on CUDA-12.8-capable RTX 40-series
/// drivers. The CUDA plan pins the LOCAL version (`+cu128`) on purpose so pip can't
/// treat a system CPU build as "already satisfied" and skip the GPU wheel.
const TORCH_CUDA: &str = "cu128";
/// The PyTorch CUDA wheel index used for the `Cuda` plan.
const CUDA_INDEX_URL: &str = "https://download.pytorch.org/whl/cu128";
/// The warmup that populates the HF cache: importing + `from_pretrained` is exactly
/// what the server's `_load_model` does, so this pre-downloads the model weights.
const MODEL_WARMUP_PY: &str = "import os; from omnivoice import OmniVoice; \
OmniVoice.from_pretrained(os.environ.get('OMNIVOICE_MODEL_ID', 'k2-fsa/omnivoice')); \
print('omnivoice model ready', flush=True)";
/// How often the per-step reader wakes to poll the cancel flag while a child runs.
const CANCEL_POLL: Duration = Duration::from_millis(250);
/// How many trailing output lines to keep for a failed step's error message.
const TAIL_LINES: usize = 20;

/// The `pip install` argv (after the interpreter) for the torch step, per GPU choice.
/// `Auto` is treated as `Cpu` here — callers resolve `Auto` via [`resolve_gpu_choice`]
/// first; the CPU-safe fallback keeps a bare `Auto` from ever planning a CUDA wheel.
///
/// - `Cuda`: pin the LOCAL `+cu128` version from the PyTorch CUDA index, so pip fetches
///   the GPU wheel (a plain `torch==2.8.0` can resolve to a CPU build).
/// - `Cpu`: plain version pins from the default PyPI index (CPU wheels).
fn torch_install_args(gpu: GpuChoice) -> Vec<String> {
    let s = |v: &str| v.to_string();
    match gpu {
        GpuChoice::Cuda => vec![
            s("-m"),
            s("pip"),
            s("install"),
            format!("torch=={TORCH_VER}+{TORCH_CUDA}"),
            format!("torchaudio=={TORCH_VER}+{TORCH_CUDA}"),
            s("--extra-index-url"),
            s(CUDA_INDEX_URL),
        ],
        GpuChoice::Cpu | GpuChoice::Auto => vec![
            s("-m"),
            s("pip"),
            s("install"),
            format!("torch=={TORCH_VER}"),
            format!("torchaudio=={TORCH_VER}"),
        ],
    }
}

/// Probe for an NVIDIA GPU by running `nvidia-smi` (on PATH). Exit 0 -> [`GpuChoice::Cuda`],
/// otherwise (missing binary, non-zero exit, spawn error) -> [`GpuChoice::Cpu`]. This is
/// the same heuristic the reference installers use; it never errors (a probe failure just
/// means "no CUDA"), so the install always has a working CPU fallback.
pub fn detect_gpu() -> GpuChoice {
    let mut cmd = std::process::Command::new("nvidia-smi");
    cmd.arg("-L").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    match cmd.status() {
        Ok(st) if st.success() => GpuChoice::Cuda,
        _ => GpuChoice::Cpu,
    }
}

/// Resolve a requested [`GpuChoice`] to a concrete `Cpu`/`Cuda`: `Auto` defers to
/// `detected` (from [`detect_gpu`]); explicit `Cpu`/`Cuda` are honored as overrides.
/// Split from `detect_gpu` (no IO) so the resolution is deterministically unit-testable.
/// The `OMNIVOICE_DEVICE` string passed to the Python engine subprocess.
pub fn omnivoice_device_string(gpu: GpuChoice) -> &'static str {
    match gpu {
        GpuChoice::Cuda => "cuda:0",
        GpuChoice::Cpu | GpuChoice::Auto => "cpu",
    }
}

pub fn resolve_gpu_choice(requested: GpuChoice, detected: GpuChoice) -> GpuChoice {
    match requested {
        GpuChoice::Auto => detected,
        explicit => explicit,
    }
}

/// Pure argv planner for the subprocess steps (no spawning, no IO). Unit-tested.
pub fn plan_install(
    base_python: &Path,
    venv_dir: &Path,
    venv_python: &Path,
    requirements: &Path,
    gpu: GpuChoice,
) -> Vec<StepSpec> {
    let s = |v: &str| v.to_string();
    let spec = |step, program: &Path, args: Vec<String>| StepSpec {
        step,
        program: program.to_path_buf(),
        args,
    };

    let torch = torch_install_args(gpu);

    vec![
        spec(
            InstallStep::CreateVenv,
            base_python,
            vec![s("-m"), s("venv"), venv_dir.to_string_lossy().into_owned()],
        ),
        spec(
            InstallStep::UpgradePip,
            venv_python,
            vec![s("-m"), s("pip"), s("install"), s("--upgrade"), s("pip")],
        ),
        spec(InstallStep::InstallTorch, venv_python, torch),
        spec(
            InstallStep::InstallOmnivoice,
            venv_python,
            vec![
                s("-m"),
                s("pip"),
                s("install"),
                s("-r"),
                requirements.to_string_lossy().into_owned(),
            ],
        ),
        spec(
            InstallStep::DownloadModel,
            venv_python,
            vec![s("-c"), s(MODEL_WARMUP_PY)],
        ),
    ]
}

/// The shipped requirements file: alongside the portable engine script, else the dev
/// repo path (mirrors `engine::resolve_script`'s portable-vs-dev fallback).
fn resolve_requirements(layout: &ToolLayout) -> PathBuf {
    match layout.omnivoice_script.as_ref().and_then(|p| p.parent()) {
        Some(dir) => dir.join("requirements-omnivoice.txt"),
        None => PathBuf::from("engine").join("requirements-omnivoice.txt"),
    }
}

/// Provision the engine: create the venv, install deps, warm the model cache, write
/// the marker. Streams each child's stdout+stderr line-by-line to `on_progress` and
/// polls `cancel` between and during steps (best-effort killing the running child).
/// Idempotent: returns early (`skipped`) if the `.installed` marker already exists.
pub async fn run_install(
    layout: &ToolLayout,
    gpu: GpuChoice,
    mut on_progress: impl FnMut(InstallStep, &str),
    cancel: &CancelToken,
) -> Result<InstallReport, AppError> {
    let venv_python = layout.venv_python();
    if layout.engine_installed() {
        return Ok(InstallReport { installed_python: venv_python, steps_run: vec![], skipped: true });
    }

    let base_python = layout
        .base_python
        .clone()
        .unwrap_or_else(|| PathBuf::from("python"));
    let venv_dir = layout.runtime_root.join("venv");
    let requirements = resolve_requirements(layout);
    let hf_cache = layout.runtime_root.join("hf-cache");
    std::fs::create_dir_all(&layout.runtime_root)?;
    std::fs::create_dir_all(&hf_cache)?;
    let env: Vec<(String, String)> = vec![("HF_HOME".into(), hf_cache.to_string_lossy().into_owned())];

    let specs = plan_install(&base_python, &venv_dir, &venv_python, &requirements, gpu);
    let mut steps_run = Vec::new();
    for spec in &specs {
        if cancel.is_cancelled() {
            return Err(AppError::Other("engine install cancelled".into()));
        }
        on_progress(spec.step, &format!("$ {}", describe(spec)));
        run_step(spec, &env, &mut on_progress, cancel).await?;
        steps_run.push(spec.step);
    }

    if cancel.is_cancelled() {
        return Err(AppError::Other("engine install cancelled".into()));
    }
    std::fs::write(layout.installed_marker(), b"ok\n")?;
    on_progress(InstallStep::Finalize, "wrote .installed marker");
    steps_run.push(InstallStep::Finalize);
    Ok(InstallReport { installed_python: venv_python, steps_run, skipped: false })
}

/// A short human-readable description of a step's command for the progress log.
fn describe(spec: &StepSpec) -> String {
    format!("{} {}", spec.program.display(), spec.args.join(" "))
}

/// Spawn one planned step, streaming both output streams to `on_progress` and
/// polling `cancel` on a short timer. Non-zero exit -> an error carrying the output
/// tail; a cancel kills the child and returns a cancelled error.
async fn run_step(
    spec: &StepSpec,
    env: &[(String, String)],
    on_progress: &mut impl FnMut(InstallStep, &str),
    cancel: &CancelToken,
) -> Result<(), AppError> {
    let mut cmd = Command::new(&spec.program);
    cmd.args(&spec.args)
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    no_window(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| {
        AppError::Other(format!("failed to spawn {}: {e}", spec.program.display()))
    })?;

    let mut out = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut err = BufReader::new(child.stderr.take().unwrap()).lines();
    let mut tail: VecDeque<String> = VecDeque::new();
    let (mut out_done, mut err_done) = (false, false);
    while !(out_done && err_done) {
        tokio::select! {
            r = out.next_line(), if !out_done => match r {
                Ok(Some(l)) => { on_progress(spec.step, &l); push_tail(&mut tail, l); }
                _ => out_done = true,
            },
            r = err.next_line(), if !err_done => match r {
                Ok(Some(l)) => { on_progress(spec.step, &l); push_tail(&mut tail, l); }
                _ => err_done = true,
            },
            _ = tokio::time::sleep(CANCEL_POLL) => {
                if cancel.is_cancelled() {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    return Err(AppError::Other("engine install cancelled".into()));
                }
            }
        }
    }

    let status = child.wait().await?;
    if !status.success() {
        return Err(AppError::Other(format!(
            "install step {:?} failed ({status}). Last output:\n{}",
            spec.step,
            Vec::from(tail).join("\n")
        )));
    }
    Ok(())
}

/// Keep only the last [`TAIL_LINES`] output lines for a failed-step error message.
fn push_tail(tail: &mut VecDeque<String>, line: String) {
    if tail.len() == TAIL_LINES {
        tail.pop_front();
    }
    tail.push_back(line);
}

#[cfg(windows)]
fn no_window(cmd: &mut Command) {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn no_window(_cmd: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(gpu: GpuChoice) -> Vec<StepSpec> {
        plan_install(
            Path::new("/tools/python"),
            Path::new("/rt/venv"),
            Path::new("/rt/venv/bin/python3"),
            Path::new("/engine/requirements-omnivoice.txt"),
            gpu,
        )
    }

    #[test]
    fn plan_orders_the_five_subprocess_steps() {
        let steps: Vec<InstallStep> = plan(GpuChoice::Cpu).iter().map(|s| s.step).collect();
        assert_eq!(
            steps,
            vec![
                InstallStep::CreateVenv,
                InstallStep::UpgradePip,
                InstallStep::InstallTorch,
                InstallStep::InstallOmnivoice,
                InstallStep::DownloadModel,
            ]
        );
    }

    #[test]
    fn create_venv_uses_base_python_and_venv_dir() {
        let p = &plan(GpuChoice::Cpu)[0];
        assert_eq!(p.program, PathBuf::from("/tools/python"));
        assert_eq!(p.args, vec!["-m", "venv", "/rt/venv"]);
    }

    #[test]
    fn cpu_torch_plan_pins_versions_from_the_default_index() {
        let torch = plan(GpuChoice::Cpu)
            .into_iter()
            .find(|s| s.step == InstallStep::InstallTorch)
            .unwrap();
        assert_eq!(torch.program, PathBuf::from("/rt/venv/bin/python3"));
        // Plain version pins, no CUDA local version and no extra index (PyPI = CPU).
        assert!(torch.args.contains(&"torch==2.8.0".to_string()));
        assert!(torch.args.contains(&"torchaudio==2.8.0".to_string()));
        assert!(!torch.args.iter().any(|a| a == "--extra-index-url"));
        assert!(!torch.args.iter().any(|a| a.contains("cu128")));
    }

    #[test]
    fn cuda_torch_plan_pins_the_cuda_local_version_and_index() {
        let torch = plan(GpuChoice::Cuda)
            .into_iter()
            .find(|s| s.step == InstallStep::InstallTorch)
            .unwrap();
        // The CUDA local version is pinned so pip can't skip the GPU wheel.
        assert!(torch.args.contains(&"torch==2.8.0+cu128".to_string()));
        assert!(torch.args.contains(&"torchaudio==2.8.0+cu128".to_string()));
        assert!(torch.args.contains(&"--extra-index-url".to_string()));
        assert!(torch.args.contains(&CUDA_INDEX_URL.to_string()));
    }

    #[test]
    fn auto_plans_the_cpu_default() {
        assert_eq!(plan(GpuChoice::Auto), plan(GpuChoice::Cpu));
    }

    #[test]
    fn resolve_gpu_choice_defers_auto_to_detection_and_honors_overrides() {
        // Auto follows whatever detect_gpu returned.
        assert_eq!(resolve_gpu_choice(GpuChoice::Auto, GpuChoice::Cuda), GpuChoice::Cuda);
        assert_eq!(resolve_gpu_choice(GpuChoice::Auto, GpuChoice::Cpu), GpuChoice::Cpu);
        // Explicit overrides win regardless of what was detected.
        assert_eq!(resolve_gpu_choice(GpuChoice::Cpu, GpuChoice::Cuda), GpuChoice::Cpu);
        assert_eq!(resolve_gpu_choice(GpuChoice::Cuda, GpuChoice::Cpu), GpuChoice::Cuda);
    }

    #[test]
    fn gpu_choice_parses_the_setting_token() {
        assert_eq!(GpuChoice::from_setting(Some("cpu")), GpuChoice::Cpu);
        assert_eq!(GpuChoice::from_setting(Some(" CUDA ")), GpuChoice::Cuda);
        assert_eq!(GpuChoice::from_setting(Some("auto")), GpuChoice::Auto);
        // Unset or unknown -> Auto (the safe, detect-then-fallback default).
        assert_eq!(GpuChoice::from_setting(None), GpuChoice::Auto);
        assert_eq!(GpuChoice::from_setting(Some("gpu")), GpuChoice::Auto);
    }

    #[test]
    fn omnivoice_step_installs_from_requirements_and_model_uses_dash_c() {
        let steps = plan(GpuChoice::Cpu);
        let omni = steps.iter().find(|s| s.step == InstallStep::InstallOmnivoice).unwrap();
        assert_eq!(
            omni.args,
            vec!["-m", "pip", "install", "-r", "/engine/requirements-omnivoice.txt"]
        );
        let model = steps.iter().find(|s| s.step == InstallStep::DownloadModel).unwrap();
        assert_eq!(model.args[0], "-c");
        assert!(model.args[1].contains("from_pretrained"));
    }

    #[test]
    fn all_lists_every_step_including_finalize() {
        assert_eq!(InstallStep::ALL.len(), 6);
        assert_eq!(InstallStep::ALL[5], InstallStep::Finalize);
    }
}
