//! Local OmniVoice engine client (item-08).
//!
//! [`engine`] owns the managed subprocess (start/adopt/health/stop); [`omnivoice`] holds the HTTP wire
//! types + the single-line synthesis call. The engine plays no role at export/play
//! time - the WeiDU packs are native (see `docs/adr/0001-native-weidu-export.md`);
//! this is a generation-time dependency only.

pub mod engine;
pub mod install;
pub mod omnivoice;

pub use engine::{EngineConfig, EngineStatus, OmniVoiceEngine};
pub use install::{
    detect_gpu, resolve_gpu_choice, run_install, GpuChoice, InstallReport, InstallStep,
};
