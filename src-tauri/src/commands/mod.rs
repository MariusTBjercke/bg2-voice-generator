//! The Tauri command boundary - the ONLY surface the frontend reaches the backend
//! through (see `docs/adr/0003-repo-module-layout.md`). Every capability added later
//! registers a command here; the frontend never touches the filesystem, DB, game
//! resources, generation, or export directly.
//!
//! The scaffold ships the two boundary commands needed to prove the wiring end to
//! end: `settings` (get/set persisted key/value) and `startup` (health check).

pub mod agent;
pub mod attribution;
pub mod binding_audit;
pub mod dictionary;
pub mod export;
pub mod extractor;
pub mod generate;
pub mod harvest;
pub mod metadata_binding;
pub mod progress;
pub mod settings;
pub mod startup;
pub mod synthesis;
pub mod tag_rules;
pub mod transfer;
pub mod voice_profiles;
