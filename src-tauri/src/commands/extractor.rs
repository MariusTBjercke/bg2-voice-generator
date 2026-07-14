//! Game-resource resolution commands (item-04).
//!
//! Thin wrappers over the `extractor` module: they take a game directory (and
//! optional locale/resref) and return the minimal serde views. All filesystem +
//! binary parsing stays behind these commands so the frontend remains UI-only
//! (see `docs/adr/0003-repo-module-layout.md`). `game_dir` is passed explicitly
//! for now; a later item defaults it from the persisted `game_dir` setting.

use std::path::Path;

use crate::error::AppError;
use crate::extractor;
use crate::extractor::views::{CreView, DlgView, GameLanguages, TlkEntryView, TlkSummary};

/// List installed locales and the resolved active one.
#[tauri::command]
pub async fn get_game_languages(game_dir: String) -> Result<GameLanguages, AppError> {
    extractor::game_languages(Path::new(&game_dir))
}

/// Header facts for the active-language `dialog.tlk`.
#[tauri::command]
pub async fn get_tlk_summary(
    game_dir: String,
    locale: Option<String>,
) -> Result<TlkSummary, AppError> {
    extractor::tlk_summary(Path::new(&game_dir), locale.as_deref())
}

/// Resolve a single TLK strref (text, flags, attached sound resref).
#[tauri::command]
pub async fn get_tlk_entry(
    game_dir: String,
    locale: Option<String>,
    strref: u32,
) -> Result<TlkEntryView, AppError> {
    extractor::tlk_entry(Path::new(&game_dir), locale.as_deref(), strref)
}

/// Resolve and parse a DLG (actor states kept distinct from player transitions).
#[tauri::command]
pub async fn resolve_dialog(game_dir: String, resref: String) -> Result<DlgView, AppError> {
    extractor::resolve_dialog(Path::new(&game_dir), &resref)
}

/// Resolve and parse a CRE (factual creature metadata).
#[tauri::command]
pub async fn resolve_creature(game_dir: String, resref: String) -> Result<CreView, AppError> {
    extractor::resolve_creature(Path::new(&game_dir), &resref)
}
