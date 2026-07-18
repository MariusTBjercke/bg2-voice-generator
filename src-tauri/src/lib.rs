//! BG2 Voice Generator - Tauri backend entry point.
//!
//! Owns `AppState`, plugin registration, the SQLite bootstrap, and the full
//! `invoke_handler`. The domain modules (extractor, generator, export, ...) are
//! reserved here as stubs and filled in by later items; every capability the
//! frontend gains registers a command in the handler below - the UI never performs
//! IO directly (see `docs/adr/0003-repo-module-layout.md`).

pub mod error;
pub mod models;
pub mod paths;
pub mod profile;

pub mod audio;
pub mod agent_templates;
pub mod backup;
pub mod cli;
pub mod commands;
pub mod db;
pub mod dictionary;
pub mod dictionary_defaults;
pub mod export;
pub mod extractor;
pub mod fingerprint;
pub mod generator;
pub mod omnivoice_tags;
pub mod synthesis;
pub mod synthesis_corpus_audit;
pub mod synthesis_validation;
pub mod tag_rule_defaults;
pub mod tag_rules;
pub mod tts_spelling;
pub mod transfer;
pub mod profile_transfer;
pub mod tts;
pub mod voices;

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use tauri::{LogicalSize, Manager};
use tokio::sync::Mutex;

pub use error::AppError;

use commands::progress::CancelRegistry;
use tts::OmniVoiceEngine;

/// Shared application state, managed by Tauri and injected into every command.
pub struct AppState {
    /// Tauri app-data root (holds `profiles.json`, shared engine runtime, `profiles/`).
    pub app_data_dir: PathBuf,
    /// Active profile id (folder name under `profiles/`).
    pub profile_id: RwLock<String>,
    /// Active profile display name.
    pub profile_name: RwLock<String>,
    /// Absolute path to the active profile directory.
    pub profile_dir: RwLock<PathBuf>,
    /// The single writer connection, guarded for async command access.
    pub db: Arc<Mutex<rusqlite::Connection>>,
    /// Absolute path to the active profile's SQLite file (surfaced by `health_check`).
    pub(crate) db_path_slot: RwLock<PathBuf>,
    /// Shared HTTP client (reused by the OmniVoice subprocess client).
    pub http: reqwest::Client,
    /// Resolved portable vs. dev tool/engine layout.
    pub tools: Arc<paths::ToolLayout>,
    /// The managed local OmniVoice engine (item-08). Boots lazily on first use and
    /// is stopped on app exit if this process owns it.
    pub omnivoice: Arc<OmniVoiceEngine>,
    /// Per-operation cooperative-cancel flags (item-06b), flipped by
    /// `cancel_operation` and polled by the long-running loops.
    pub cancels: Arc<CancelRegistry>,
}

impl AppState {
    /// Absolute path to the active profile database.
    pub fn db_path(&self) -> PathBuf {
        self.db_path_slot
            .read()
            .expect("db_path lock")
            .clone()
    }

    pub fn active_profile_id(&self) -> String {
        self.profile_id.read().expect("profile_id lock").clone()
    }

    pub fn active_profile_name(&self) -> String {
        self.profile_name.read().expect("profile_name lock").clone()
    }

    pub fn active_profile_dir(&self) -> PathBuf {
        self.profile_dir.read().expect("profile_dir lock").clone()
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            log::info!("BG2 Voice Generator v{} starting up", env!("CARGO_PKG_VERSION"));

            let data_dir = app
                .handle()
                .path()
                .app_data_dir()
                .expect("failed to resolve app data dir");
            std::fs::create_dir_all(&data_dir).ok();
            log::info!("Data directory: {}", data_dir.display());

            let tools = Arc::new(paths::ToolLayout::resolve(&data_dir));
            log::info!(
                "Tool layout: portable={}, runtime_root={}",
                tools.portable,
                tools.runtime_root.display()
            );

            let resolved = profile::ensure_profile_layout(&data_dir)
                .expect("failed to resolve profile layout");
            log::info!(
                "Active profile: {} ({}) at {}",
                resolved.info.name,
                resolved.info.id,
                resolved.profile_dir.display()
            );

            // Open + migrate the active profile DB. Engine runtime stays at data_dir.
            let conn = db::open_db(&resolved.profile_dir).expect("failed to open database");
            let db_path = resolved.profile_dir.join(db::DB_FILE_NAME);
            log::info!("Database ready at {}", db_path.display());
            commands::agent::refresh_all_agent_workspaces(&conn, &db_path);
            commands::voice_profiles::cleanup_abandoned_design_previews(&conn, &db_path);

            let http = reqwest::Client::new();
            let omnivoice = Arc::new(OmniVoiceEngine::new(&tools, http.clone()));

            app.manage(AppState {
                app_data_dir: data_dir,
                profile_id: RwLock::new(resolved.info.id),
                profile_name: RwLock::new(resolved.info.name),
                profile_dir: RwLock::new(resolved.profile_dir),
                db: Arc::new(Mutex::new(conn)),
                db_path_slot: RwLock::new(db_path),
                http,
                tools,
                omnivoice,
                cancels: Arc::new(CancelRegistry::default()),
            });

            // Size the shell from the current monitor work area so the pipeline nav +
            // profile controls usually fit on one row without overflowing small screens.
            if let Some(window) = app.get_webview_window("main") {
                size_main_window_for_monitor(&window);
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::startup::health_check,
            commands::profile::list_profiles,
            commands::profile::get_active_profile,
            commands::profile::create_profile,
            commands::profile::rename_profile,
            commands::profile::switch_profile,
            commands::profile::duplicate_profile,
            commands::profile::delete_profile,
            commands::profile::export_profile,
            commands::profile::import_profile,
            commands::progress::cancel_operation,
            commands::settings::get_setting,
            commands::settings::set_setting,
            commands::settings::get_peak_normalize_default,
            commands::settings::set_peak_normalize_default,
            commands::dictionary::list_dictionary_rules,
            commands::dictionary::preview_dictionary_text,
            commands::dictionary::upsert_dictionary_rule,
            commands::dictionary::set_dictionary_rule_enabled,
            commands::dictionary::delete_dictionary_rule,
            commands::dictionary::reset_dictionary_defaults,
            commands::tag_rules::list_tag_rules,
            commands::tag_rules::list_supported_inline_tags,
            commands::tag_rules::preview_tag_rules_text,
            commands::tag_rules::upsert_tag_rule,
            commands::tag_rules::set_tag_rule_enabled,
            commands::tag_rules::delete_tag_rule,
            commands::tag_rules::reset_tag_rule_defaults,
            commands::agent::prepare_agent_workspace,
            commands::agent::reveal_agent_workspace,
            commands::agent::launch_agent,
            commands::synthesis::get_line_synthesis_preview,
            commands::synthesis::set_line_synthesis_override,
            commands::synthesis::clear_line_synthesis_override,
            commands::synthesis::mark_synthesis_reviewed,
            commands::synthesis::unmark_synthesis_reviewed,
            commands::synthesis::synthesis_tagging_summary,
            commands::synthesis::list_synthesis_decisions,
            commands::synthesis::reset_synthesis_agent_state,
            commands::synthesis::synthesis_corpus_audit_summary,
            commands::synthesis::list_synthesis_flagged,
            commands::synthesis::list_synthesis_remaining,
            commands::synthesis::auto_review_synthesis_plain,
            commands::extractor::get_game_languages,
            commands::extractor::get_tlk_summary,
            commands::extractor::get_tlk_entry,
            commands::extractor::resolve_dialog,
            commands::extractor::resolve_creature,
            commands::attribution::scan_attribution,
            commands::attribution::get_attribution_counts,
            commands::attribution::list_blocked_lines,
            commands::attribution::list_blocked_lines_page,
            commands::attribution::reapply_token_standins,
            commands::harvest::harvest_references,
            commands::harvest::list_speakers,
            commands::harvest::list_speaker_groups,
            commands::harvest::count_speaker_group_generations,
            commands::harvest::set_speaker_group_excluded,
            commands::harvest::list_group_reference_samples,
            commands::harvest::list_reference_samples,
            commands::harvest::set_sample_decision,
            commands::harvest::auto_approve_best_samples,
            commands::harvest::auto_approve_manual_gaps_samples,
            commands::harvest::reset_decisions,
            commands::harvest::verify_speech,
            commands::generate::engine_status,
            commands::generate::start_engine,
            commands::generate::stop_engine,
            commands::generate::install_engine,
            commands::generate::bind_clone,
            commands::generate::auto_bind_all,
            commands::generate::reconcile_identity_group_bindings,
            commands::generate::list_clones,
            commands::generate::get_clone_render_settings,
            commands::generate::set_clone_render_settings,
            commands::generate::set_clone_references,
            commands::generate::preview_clone_voice,
            commands::generate::generate_line,
            commands::generate::get_line_render_override,
            commands::generate::set_line_render_override,
            commands::generate::clear_line_render_override,
            commands::generate::list_render_candidates,
            commands::generate::generate_render_candidate,
            commands::generate::accept_render_candidate,
            commands::generate::discard_render_candidate,
            commands::generate::generate_lines_batched,
            commands::generate::list_generatable_lines,
            commands::generate::list_completed_generations,
            commands::generate::list_generation_diagnostics,
            commands::generate::remove_generations,
            commands::generate::assign_fallback_voices,
            commands::metadata_binding::list_demographic_groups,
            commands::metadata_binding::list_metadata_bindings,
            commands::metadata_binding::list_effective_speaker_bindings,
            commands::binding_audit::binding_audit_progress,
            commands::binding_audit::list_personal_bindings,
            commands::binding_audit::list_suspicious_bindings,
            commands::binding_audit::list_marked_bindings,
            commands::binding_audit::list_binding_groups,
            commands::binding_audit::show_binding_detail,
            commands::binding_audit::flag_binding_review,
            commands::binding_audit::mark_binding_reviewed,
            commands::binding_audit::clear_binding_review_marker,
            commands::binding_audit::clear_personal_binding,
            commands::binding_audit::reject_binding_sample,
            commands::metadata_binding::use_demographic_default,
            commands::metadata_binding::follow_speaker_voice,
            commands::metadata_binding::add_metadata_donor,
            commands::metadata_binding::remove_metadata_donor,
            commands::metadata_binding::suggest_metadata_donors,
            commands::metadata_binding::list_eligible_metadata_donors,
            commands::metadata_binding::auto_configure_metadata_pools,
            commands::metadata_binding::clear_metadata_binding,
            commands::metadata_binding::clear_all_metadata_pools,
            commands::metadata_binding::clear_speaker_clones,
            commands::metadata_binding::apply_metadata_bindings,
            commands::voice_profiles::list_voice_profiles,
            commands::voice_profiles::select_voice_reference_files,
            commands::voice_profiles::create_imported_voice_profile,
            commands::voice_profiles::generate_designed_voice_candidates,
            commands::voice_profiles::save_designed_voice_profile,
            commands::voice_profiles::rename_voice_profile,
            commands::voice_profiles::delete_voice_profile,
            commands::voice_profiles::bind_speaker_voice_profile,
            commands::metadata_binding::add_metadata_profile,
            commands::metadata_binding::remove_metadata_profile,
            commands::export::build_export,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Stop the OmniVoice subprocess we own on exit so it never orphans.
            if let tauri::RunEvent::ExitRequested { .. } = event {
                let engine = app.state::<AppState>().omnivoice.clone();
                tauri::async_runtime::block_on(engine.shutdown());
            }
        });
}

/// Pick a comfortable startup size from the monitor work area (taskbar excluded).
/// Uses ~82% of available space, clamped so small laptops stay usable and large
/// displays do not open a near-fullscreen window.
fn size_main_window_for_monitor(window: &tauri::WebviewWindow) {
    const MIN_W: f64 = 960.0;
    const MIN_H: f64 = 640.0;
    const MAX_W: f64 = 1680.0;
    const MAX_H: f64 = 1050.0;
    const FRACTION: f64 = 0.82;
    const MARGIN: f64 = 48.0;

    let Ok(Some(monitor)) = window.current_monitor() else {
        log::warn!("could not resolve current monitor; keeping configured window size");
        return;
    };
    let scale = monitor.scale_factor();
    let work = monitor.work_area().size;
    let work_w = (work.width as f64 / scale) - MARGIN;
    let work_h = (work.height as f64 / scale) - MARGIN;
    if work_w <= 0.0 || work_h <= 0.0 {
        return;
    }

    let width = (work_w * FRACTION).clamp(MIN_W.min(work_w), MAX_W.min(work_w));
    let height = (work_h * FRACTION).clamp(MIN_H.min(work_h), MAX_H.min(work_h));

    if let Err(e) = window.set_size(LogicalSize::new(width, height)) {
        log::warn!("failed to set startup window size: {e}");
        return;
    }
    if let Err(e) = window.center() {
        log::warn!("failed to center startup window: {e}");
    }
    log::info!(
        "Startup window sized to {width:.0}x{height:.0} (work area {:.0}x{:.0} @ scale {scale})",
        work_w + MARGIN,
        work_h + MARGIN
    );
}
