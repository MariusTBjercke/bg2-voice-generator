//! Progress events + cooperative cancellation for long-running commands (item-06b).
//!
//! Long commands (harvest, attribution scan, generation, export, transfer) stay
//! opaque blocking calls otherwise. This module adds:
//!   * [`OperationProgress`] - the typed payload emitted on `operation://progress`.
//!   * [`ProgressEmitter`] - a throttled adapter from a pure loop's counters to the
//!     Tauri event bus (the pure modules take plain callbacks; this bridges them so
//!     `voices::harvest` etc. never depend on Tauri types - see ADR 0003).
//!   * [`CancelRegistry`] - per-operation cancel flags flipped by [`cancel_operation`].
//!
//! Cancellation is COOPERATIVE: a loop checks its token between items and returns a
//! clean partial result; nothing is thread-killed and persisted work is kept.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

use crate::error::AppError;
use crate::AppState;

/// The event name every long-running command emits progress on.
pub const PROGRESS_EVENT: &str = "operation://progress";

/// Stable operation identifiers (also the frontend `progress` store keys and the
/// argument `cancel_operation` takes). Kept in sync with `src/lib/stores/progress.ts`.
pub const OP_HARVEST: &str = "harvest";
pub const OP_SPEECH_VERIFY: &str = "speech_verify";
pub const OP_ATTRIBUTION: &str = "attribution";
pub const OP_GENERATION: &str = "generation";
pub const OP_EXPORT: &str = "export";
pub const OP_TRANSFER: &str = "transfer";
pub const OP_ENGINE_INSTALL: &str = "engine_install";

/// A progress update for one operation. Mirror of `OperationProgress` in
/// `src/lib/types/index.ts`. `total: None` means an indeterminate bar; a terminal
/// `phase` (`done` / `cancelled` / `error`) tells the frontend to clear the entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationProgress {
    /// Which operation this belongs to (one of the `OP_*` constants).
    pub op: String,
    /// A coarse phase (`running`, `done`, `cancelled`, `error`, ...).
    pub phase: String,
    /// Items processed so far.
    pub done: u64,
    /// Total items when known; `None` for an indeterminate operation.
    pub total: Option<u64>,
    /// Optional human-readable detail (e.g. the current speaker/line).
    pub message: Option<String>,
}

impl OperationProgress {
    /// True for a terminal phase (the frontend clears the bar on these).
    pub fn is_terminal(&self) -> bool {
        matches!(self.phase.as_str(), "done" | "cancelled" | "error")
    }
}

/// Throttled adapter that turns a pure loop's counters into emitted events. Emits
/// immediately on the first call and on terminal phases; otherwise rate-limits to
/// ~10/s so a 20k-item loop cannot flood the IPC bridge (see item-06b Risks).
pub struct ProgressEmitter {
    app: AppHandle,
    op: &'static str,
    last: Option<Instant>,
    min_interval: Duration,
}

impl ProgressEmitter {
    pub fn new(app: AppHandle, op: &'static str) -> Self {
        Self { app, op, last: None, min_interval: Duration::from_millis(100) }
    }

    /// Emit a running-phase update, throttled by time.
    pub fn tick(&mut self, done: u64, total: Option<u64>, message: Option<String>) {
        let now = Instant::now();
        if self.last.map(|t| now.duration_since(t) < self.min_interval).unwrap_or(false) {
            return;
        }
        self.last = Some(now);
        self.emit("running", done, total, message);
    }

    /// Emit a terminal phase (always sent, bypassing the throttle).
    pub fn finish(&mut self, phase: &str, done: u64, total: Option<u64>, message: Option<String>) {
        self.emit(phase, done, total, message);
    }

    fn emit(&self, phase: &str, done: u64, total: Option<u64>, message: Option<String>) {
        let payload = OperationProgress {
            op: self.op.to_string(),
            phase: phase.to_string(),
            done,
            total,
            message,
        };
        // A failed emit (no window) must never abort the operation itself.
        let _ = self.app.emit(PROGRESS_EVENT, payload);
    }
}

/// Per-operation cancel flags. A running command registers a token under its `op`
/// name; `cancel_operation` flips it; the loop reads it via [`CancelToken`].
#[derive(Default)]
pub struct CancelRegistry {
    flags: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

/// A shared cancel flag handed to a running loop. Cloneable and cheap to poll.
#[derive(Clone)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// True once `cancel_operation` has flipped this operation's flag.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

impl CancelRegistry {
    /// Register (replacing any stale flag) a fresh, un-cancelled token for `op`.
    pub async fn begin(&self, op: &str) -> CancelToken {
        let flag = Arc::new(AtomicBool::new(false));
        self.flags.lock().await.insert(op.to_string(), flag.clone());
        CancelToken(flag)
    }

    /// Drop `op`'s token once the operation has finished (idempotent).
    pub async fn end(&self, op: &str) {
        self.flags.lock().await.remove(op);
    }

    /// Flip `op`'s flag if it is running. Returns whether a token was found.
    pub async fn cancel(&self, op: &str) -> bool {
        match self.flags.lock().await.get(op) {
            Some(flag) => {
                flag.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }
}

/// Request cooperative cancellation of a running operation by its `op` id. Returns
/// `true` if an operation was running (and has now been asked to stop), `false`
/// otherwise. The loop stops between items and returns a partial/cancelled result.
#[tauri::command]
pub async fn cancel_operation(state: tauri::State<'_, AppState>, op: String) -> Result<bool, AppError> {
    Ok(state.cancels.cancel(&op).await)
}
