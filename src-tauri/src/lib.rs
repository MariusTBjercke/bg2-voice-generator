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
pub mod tts;
pub mod voices;

use std::path::PathBuf;
use std::sync::Arc;

use tauri::Manager;
use tokio::sync::Mutex;

pub use error::AppError;

use commands::progress::CancelRegistry;
use tts::OmniVoiceEngine;

/// Shared application state, managed by Tauri and injected into every command.
pub struct AppState {
    /// The single writer connection, guarded for async command access.
    pub db: Arc<Mutex<rusqlite::Connection>>,
    /// Absolute path to the SQLite file (surfaced by `health_check`).
    pub db_path: PathBuf,
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

            // Open + migrate the DB synchronously. The scaffold's schema is tiny, so
            // (unlike the reference's minute-long migrations) this is fast; a later
            // item moves it to a background thread with a splash if it ever grows.
            let conn = db::open_db(&data_dir).expect("failed to open database");
            let db_path = data_dir.join(db::DB_FILE_NAME);
            log::info!("Database ready at {}", db_path.display());
            commands::agent::refresh_all_agent_workspaces(&conn, &db_path);

            let http = reqwest::Client::new();
            let omnivoice = Arc::new(OmniVoiceEngine::new(&tools, http.clone()));

            app.manage(AppState {
                db: Arc::new(Mutex::new(conn)),
                db_path,
                http,
                tools,
                omnivoice,
                cancels: Arc::new(CancelRegistry::default()),
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::startup::health_check,
            commands::progress::cancel_operation,
            commands::settings::get_setting,
            commands::settings::set_setting,
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
            commands::attribution::reapply_token_standins,
            commands::harvest::harvest_references,
            commands::harvest::list_speakers,
            commands::harvest::list_speaker_groups,
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
            commands::metadata_binding::use_demographic_default,
            commands::metadata_binding::add_metadata_donor,
            commands::metadata_binding::remove_metadata_donor,
            commands::metadata_binding::suggest_metadata_donors,
            commands::metadata_binding::list_eligible_metadata_donors,
            commands::metadata_binding::auto_configure_metadata_pools,
            commands::metadata_binding::clear_metadata_binding,
            commands::metadata_binding::clear_all_metadata_pools,
            commands::metadata_binding::clear_speaker_clones,
            commands::metadata_binding::apply_metadata_bindings,
            commands::export::build_export,
            commands::transfer::export_project,
            commands::transfer::import_project,
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
