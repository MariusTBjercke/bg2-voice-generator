//! Versioned schema migrations, tracked by SQLite's `PRAGMA user_version`.
//!
//! v1 is the public-release baseline: settings, the full BG2 domain tables (including
//! voice profiles, follow bindings, binding review, tag rules, and list-screen indexes),
//! metadata binding pools, synthesis-text overrides/review, clone render settings,
//! ordered clone references, line render experiments, generation diagnostics, and
//! machine-wide dictionary rules.
//!
//! Add later schema as new [`MIGRATIONS`] entries with the next sequential `version`;
//! NEVER edit a shipped migration in place.

use rusqlite::Connection;

use crate::error::AppError;

/// One forward-only migration step. `version` is the `user_version` the DB is at
/// AFTER `sql` runs, and MUST be `1..` and strictly increasing across the slice.
struct Migration {
    version: i32,
    sql: &'static str,
}

/// The ordered migration list. Append-only after the public v1 baseline.
const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    sql: V1_INITIAL_SCHEMA,
}];

/// v1 — public-release schema (squashed pre-release history into one baseline).
///
/// Conventions (BG2 domain schema):
///   * Booleans are stored as INTEGER `0/1`; JSON blobs as TEXT; timestamps as
///     RFC3339 TEXT (chrono default).
///   * Enum-like status columns are free-form TEXT constrained by CHECK to the exact
///     serde tokens the Rust/TS contracts serialize (see `models.rs`).
///   * Large audio is referenced by filesystem PATH, never stored as a BLOB.
///   * Foreign keys use ON DELETE CASCADE so deleting a project tears down its rows.
const V1_INITIAL_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS project (
    id                INTEGER PRIMARY KEY,
    game_root         TEXT NOT NULL,
    edition           TEXT NOT NULL,
    active_language   TEXT NOT NULL,
    generator_version TEXT NOT NULL,
    created_at        TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS install_fingerprint (
    id              INTEGER PRIMARY KEY,
    project_id      INTEGER NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    edition_version TEXT NOT NULL,
    language        TEXT NOT NULL,
    mod_state_hash  TEXT NOT NULL,
    source_hashes_json TEXT NOT NULL DEFAULT '{}',
    export_version  TEXT NOT NULL,
    captured_at     TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_fingerprint_project ON install_fingerprint(project_id);

CREATE TABLE IF NOT EXISTS speaker (
    id                 INTEGER PRIMARY KEY,
    project_id         INTEGER NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    cre_resref         TEXT NOT NULL,
    display_name       TEXT,
    sex                INTEGER NOT NULL DEFAULT 0,
    race               INTEGER NOT NULL DEFAULT 0,
    class              INTEGER NOT NULL DEFAULT 0,
    kit                INTEGER NOT NULL DEFAULT 0,
    alignment          INTEGER NOT NULL DEFAULT 0,
    creature_category  INTEGER NOT NULL DEFAULT 0,
    dialogue_resref    TEXT,
    long_name_strref   INTEGER,
    provenance_json    TEXT NOT NULL DEFAULT '{}',
    confidence         REAL NOT NULL DEFAULT 0,
    excluded           INTEGER NOT NULL DEFAULT 0
        CHECK (excluded IN (0, 1)),
    UNIQUE(project_id, cre_resref)
);
CREATE INDEX IF NOT EXISTS ix_speaker_project ON speaker(project_id);
CREATE INDEX IF NOT EXISTS ix_speaker_identity ON speaker(project_id, long_name_strref)
  WHERE long_name_strref IS NOT NULL;

CREATE TABLE IF NOT EXISTS archetype (
    id        INTEGER PRIMARY KEY,
    name      TEXT NOT NULL UNIQUE,
    tags_json TEXT NOT NULL DEFAULT '[]'
);

CREATE TABLE IF NOT EXISTS speaker_archetype (
    speaker_id   INTEGER NOT NULL REFERENCES speaker(id) ON DELETE CASCADE,
    archetype_id INTEGER NOT NULL REFERENCES archetype(id) ON DELETE CASCADE,
    PRIMARY KEY (speaker_id, archetype_id)
);

CREATE TABLE IF NOT EXISTS shared_strref_group (
    id         INTEGER PRIMARY KEY,
    strref     INTEGER NOT NULL,
    resolution TEXT NOT NULL DEFAULT 'defer_diff_voice'
        CHECK (resolution IN ('reuse_same_voice', 'defer_diff_voice'))
);

CREATE TABLE IF NOT EXISTS line (
    id                     INTEGER PRIMARY KEY,
    project_id             INTEGER NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    strref                 INTEGER NOT NULL,
    dlg_resref             TEXT,
    state_index            INTEGER,
    text                   TEXT NOT NULL DEFAULT '',
    original_text          TEXT NOT NULL DEFAULT '',
    token_mask             INTEGER NOT NULL DEFAULT 0,
    flags                  INTEGER NOT NULL DEFAULT 0,
    existing_sound_resref  TEXT,
    kind                   TEXT NOT NULL DEFAULT 'state'
        CHECK (kind IN ('state', 'transition', 'script', 'token')),
    is_voiced              INTEGER NOT NULL DEFAULT 0,
    has_tokens             INTEGER NOT NULL DEFAULT 0,
    shared_group_id        INTEGER REFERENCES shared_strref_group(id) ON DELETE SET NULL,
    speaker_id             INTEGER REFERENCES speaker(id) ON DELETE SET NULL,
    attribution_confidence REAL NOT NULL DEFAULT 0,
    status                 TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'ready', 'blocked', 'exported', 'skipped'))
);
CREATE INDEX IF NOT EXISTS ix_line_project        ON line(project_id);
CREATE INDEX IF NOT EXISTS ix_line_project_strref ON line(project_id, strref);
CREATE INDEX IF NOT EXISTS ix_line_speaker        ON line(speaker_id);
CREATE UNIQUE INDEX IF NOT EXISTS ux_line_natural_key
  ON line(project_id, strref, dlg_resref, state_index);
CREATE INDEX IF NOT EXISTS ix_line_project_status_order
    ON line(project_id, status, dlg_resref, state_index);
CREATE INDEX IF NOT EXISTS ix_line_project_speaker
    ON line(project_id, speaker_id);

CREATE TABLE IF NOT EXISTS reference_sample (
    id                    INTEGER PRIMARY KEY,
    speaker_id            INTEGER NOT NULL REFERENCES speaker(id) ON DELETE CASCADE,
    source_strref         INTEGER,
    source_sound_resref   TEXT,
    provenance_json       TEXT NOT NULL DEFAULT '{}',
    scores_json           TEXT NOT NULL DEFAULT '{}',
    decision              TEXT NOT NULL DEFAULT 'pending'
        CHECK (decision IN ('pending', 'approved', 'rejected')),
    local_derivative_path TEXT
);
CREATE INDEX IF NOT EXISTS ix_sample_speaker ON reference_sample(speaker_id);
CREATE INDEX IF NOT EXISTS ix_sample_approved_speaker
    ON reference_sample(speaker_id) WHERE decision='approved';

CREATE TABLE IF NOT EXISTS voice_profile (
    id                    INTEGER PRIMARY KEY,
    project_id            INTEGER NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    display_name          TEXT NOT NULL,
    origin                TEXT NOT NULL CHECK (origin IN ('harvested','imported','designed')),
    harvested_speaker_id  INTEGER REFERENCES speaker(id) ON DELETE SET NULL,
    design_spec_json      TEXT,
    availability          TEXT NOT NULL DEFAULT 'available'
        CHECK (availability IN ('available','missing_local_audio')),
    reference_fingerprint TEXT,
    created_at            TEXT NOT NULL,
    updated_at            TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_voice_profile_project ON voice_profile(project_id, id);
CREATE INDEX IF NOT EXISTS ix_voice_profile_harvested_speaker ON voice_profile(harvested_speaker_id);

CREATE TABLE IF NOT EXISTS voice_profile_reference (
    id                  INTEGER PRIMARY KEY,
    voice_profile_id    INTEGER NOT NULL REFERENCES voice_profile(id) ON DELETE CASCADE,
    reference_sample_id INTEGER REFERENCES reference_sample(id) ON DELETE CASCADE,
    managed_path        TEXT,
    transcript          TEXT NOT NULL,
    sort_order          INTEGER NOT NULL CHECK (sort_order >= 0),
    fingerprint         TEXT,
    CHECK ((reference_sample_id IS NOT NULL AND managed_path IS NULL) OR
           (reference_sample_id IS NULL AND managed_path IS NOT NULL) OR
           (reference_sample_id IS NULL AND managed_path IS NULL)),
    UNIQUE(voice_profile_id, sort_order)
);
CREATE INDEX IF NOT EXISTS ix_voice_profile_reference_profile
    ON voice_profile_reference(voice_profile_id, sort_order);
CREATE INDEX IF NOT EXISTS ix_voice_profile_reference_sample
    ON voice_profile_reference(reference_sample_id);

CREATE TABLE IF NOT EXISTS clone (
    id                INTEGER PRIMARY KEY,
    speaker_id        INTEGER NOT NULL REFERENCES speaker(id) ON DELETE CASCADE,
    primary_sample_id INTEGER REFERENCES reference_sample(id) ON DELETE SET NULL,
    voice_profile_id  INTEGER REFERENCES voice_profile(id) ON DELETE SET NULL,
    follow_speaker_id INTEGER REFERENCES speaker(id) ON DELETE SET NULL,
    binding_source    TEXT NOT NULL DEFAULT 'default'
        CHECK (binding_source IN ('override', 'default', 'generic', 'follow')),
    status            TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'ready', 'failed')),
    render_settings_json TEXT NOT NULL DEFAULT
'{"speed":null,"num_steps":32,"guidance_scale":2.0,"t_shift":0.1,"layer_penalty_factor":5.0,"position_temperature":5.0,"class_temperature":0.0,"prompt_denoise":true,"preprocess_prompt":true,"postprocess_output":true,"audio_chunk_duration":10.0,"audio_chunk_threshold":30.0,"seed":42,"peak_normalize_dbfs":-1.0,"peak_normalize_inherit":true}',
    CHECK (
      (binding_source = 'follow' AND follow_speaker_id IS NOT NULL AND follow_speaker_id != speaker_id)
      OR (binding_source != 'follow' AND follow_speaker_id IS NULL)
    )
);
CREATE INDEX IF NOT EXISTS ix_clone_speaker ON clone(speaker_id);
CREATE INDEX IF NOT EXISTS ix_clone_voice_profile ON clone(voice_profile_id);
CREATE INDEX IF NOT EXISTS ix_clone_follow_speaker ON clone(follow_speaker_id);
CREATE INDEX IF NOT EXISTS ix_clone_status_speaker ON clone(status, speaker_id);

CREATE TABLE IF NOT EXISTS clone_reference (
    clone_id   INTEGER NOT NULL REFERENCES clone(id) ON DELETE CASCADE,
    sample_id  INTEGER NOT NULL REFERENCES reference_sample(id) ON DELETE CASCADE,
    sort_order INTEGER NOT NULL CHECK (sort_order >= 0),
    PRIMARY KEY (clone_id, sample_id),
    UNIQUE (clone_id, sort_order)
);
CREATE INDEX IF NOT EXISTS ix_clone_reference_sample ON clone_reference(sample_id);

CREATE TABLE IF NOT EXISTS generation (
    id                   INTEGER PRIMARY KEY,
    line_id              INTEGER NOT NULL REFERENCES line(id) ON DELETE CASCADE,
    clone_id             INTEGER REFERENCES clone(id) ON DELETE SET NULL,
    status               TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'running', 'done', 'failed')),
    output_path          TEXT,
    attempts             INTEGER NOT NULL DEFAULT 0,
    resumable_state_json TEXT NOT NULL DEFAULT '{}',
    reference_sample_id  INTEGER,
    binding_source_snapshot TEXT
        CHECK (binding_source_snapshot IS NULL OR binding_source_snapshot IN ('override','default','generic','follow')),
    render_settings_json TEXT,
    render_settings_hash TEXT,
    reference_fingerprint TEXT,
    diagnostics_json     TEXT,
    voice_profile_id_snapshot INTEGER,
    synthesis_stale      INTEGER NOT NULL DEFAULT 0
        CHECK (synthesis_stale IN (0, 1))
);
CREATE INDEX IF NOT EXISTS ix_generation_line ON generation(line_id);
CREATE INDEX IF NOT EXISTS ix_generation_done_line
    ON generation(status, line_id) WHERE output_path IS NOT NULL;

CREATE TABLE IF NOT EXISTS export (
    id             INTEGER PRIMARY KEY,
    project_id     INTEGER NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    fingerprint_id INTEGER REFERENCES install_fingerprint(id) ON DELETE SET NULL,
    manifest_json  TEXT NOT NULL DEFAULT '{}',
    weidu_pack_path TEXT,
    created_at     TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_export_project ON export(project_id);

CREATE TABLE IF NOT EXISTS metadata_binding (
    id                 INTEGER PRIMARY KEY,
    project_id         INTEGER NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    sex                INTEGER NOT NULL,
    race               INTEGER NOT NULL,
    creature_category  INTEGER NOT NULL,
    UNIQUE(project_id, sex, race, creature_category)
);
CREATE INDEX IF NOT EXISTS ix_metadata_binding_project ON metadata_binding(project_id);

CREATE TABLE IF NOT EXISTS metadata_binding_donor (
    binding_id         INTEGER NOT NULL REFERENCES metadata_binding(id) ON DELETE CASCADE,
    donor_speaker_id   INTEGER NOT NULL REFERENCES speaker(id),
    sort_order         INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (binding_id, donor_speaker_id)
);
CREATE INDEX IF NOT EXISTS ix_metadata_donor_binding ON metadata_binding_donor(binding_id);

CREATE TABLE IF NOT EXISTS metadata_binding_profile (
    binding_id       INTEGER NOT NULL REFERENCES metadata_binding(id) ON DELETE CASCADE,
    voice_profile_id INTEGER NOT NULL REFERENCES voice_profile(id) ON DELETE CASCADE,
    sort_order       INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(binding_id, voice_profile_id),
    UNIQUE(binding_id, sort_order)
);
CREATE INDEX IF NOT EXISTS ix_metadata_profile_binding ON metadata_binding_profile(binding_id, sort_order);

CREATE TABLE IF NOT EXISTS synthesis_text_string (
    text_hash   TEXT PRIMARY KEY,
    source_text TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_synthesis_text_string_source_text
    ON synthesis_text_string(source_text);
CREATE TABLE IF NOT EXISTS synthesis_text_override (
    text_hash       TEXT PRIMARY KEY REFERENCES synthesis_text_string(text_hash) ON DELETE CASCADE,
    synthesis_text  TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS synthesis_text_reviewed (
    text_hash TEXT PRIMARY KEY REFERENCES synthesis_text_string(text_hash) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS synthesis_corpus_cache (
    project_id      INTEGER NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    text_hash       TEXT NOT NULL,
    source_text     TEXT NOT NULL,
    mapped_text     TEXT NOT NULL,
    first_line_id   INTEGER NOT NULL REFERENCES line(id) ON DELETE CASCADE,
    first_strref    INTEGER NOT NULL,
    shared_count    INTEGER NOT NULL,
    audit_mask      INTEGER NOT NULL,
    PRIMARY KEY (project_id, text_hash)
);
CREATE INDEX IF NOT EXISTS ix_synthesis_corpus_page
    ON synthesis_corpus_cache(project_id, first_line_id);
CREATE INDEX IF NOT EXISTS ix_synthesis_corpus_hash
    ON synthesis_corpus_cache(text_hash);

CREATE TABLE IF NOT EXISTS synthesis_corpus_cache_state (
    project_id    INTEGER PRIMARY KEY REFERENCES project(id) ON DELETE CASCADE,
    cache_version INTEGER NOT NULL,
    rebuilt_at    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS line_render_override (
    line_id       INTEGER PRIMARY KEY REFERENCES line(id) ON DELETE CASCADE,
    settings_json TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS render_candidate (
    line_id              INTEGER PRIMARY KEY REFERENCES line(id) ON DELETE CASCADE,
    status               TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending','running','done','failed')),
    output_path          TEXT,
    text_snapshot        TEXT NOT NULL,
    clone_id             INTEGER NOT NULL,
    reference_sample_id  INTEGER NOT NULL,
    reference_fingerprint TEXT NOT NULL,
    render_settings_json TEXT NOT NULL,
    render_settings_hash TEXT NOT NULL,
    state_json           TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS ix_render_candidate_status ON render_candidate(status);

CREATE TABLE IF NOT EXISTS dictionary_rule (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    find_text   TEXT NOT NULL,
    speak_as    TEXT NOT NULL,
    match_kind  TEXT NOT NULL DEFAULT 'whole_word'
        CHECK (match_kind IN ('whole_word')),
    enabled     INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0,1)),
    is_default  INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0,1)),
    updated_at  TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS ix_dictionary_rule_find
    ON dictionary_rule(lower(find_text), match_kind, is_default);

CREATE TABLE IF NOT EXISTS tag_rule (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    find_text   TEXT NOT NULL,
    tag         TEXT NOT NULL,
    match_kind  TEXT NOT NULL DEFAULT 'stage_cue'
        CHECK (match_kind IN ('whole_word', 'stage_cue')),
    enabled     INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    is_default  INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0, 1)),
    updated_at  TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS ix_tag_rule_find
    ON tag_rule(lower(find_text), match_kind, is_default);

CREATE TABLE IF NOT EXISTS binding_review (
    project_id  INTEGER NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    cre_resref  TEXT NOT NULL,
    status      TEXT NOT NULL CHECK (status IN ('flagged', 'reviewed')),
    reason      TEXT NOT NULL DEFAULT '',
    updated_at  TEXT NOT NULL,
    PRIMARY KEY (project_id, cre_resref)
);
CREATE INDEX IF NOT EXISTS idx_binding_review_project_status
    ON binding_review(project_id, status);
"#;

/// The latest schema version (the highest migration `version`, `0` if none).
pub fn latest_migration_version() -> i32 {
    MIGRATIONS.last().map(|m| m.version).unwrap_or(0)
}

/// Read the DB's current `user_version` (`0` on a brand-new DB).
pub fn current_schema_version(conn: &Connection) -> Result<i32, AppError> {
    let v: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    Ok(v)
}

/// True when the DB already has the public-release final schema markers (used to
/// reconcile pre-squash `user_version` values that exceeded the new baseline).
fn has_final_schema_markers(conn: &Connection) -> Result<bool, AppError> {
    let has_binding_review: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='binding_review')",
        [],
        |r| r.get(0),
    )?;
    if !has_binding_review {
        return Ok(false);
    }
    let has_follow: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('clone') WHERE name='follow_speaker_id')",
        [],
        |r| r.get(0),
    )?;
    Ok(has_follow)
}

/// Apply every migration whose `version` exceeds the DB's current `user_version`, in
/// order, each in its own transaction (a failed step rolls back cleanly), bumping
/// `user_version` after each. No-op on an already-current DB.
///
/// Pre-release compatibility: DBs left at a historical `user_version` above the
/// squashed baseline are clamped to `latest` when final-schema markers are present;
/// otherwise opening fails with a recreate message (no upgrade path from mid-chain
/// historical versions).
pub fn run_migrations(conn: &mut Connection) -> Result<(), AppError> {
    let current = current_schema_version(conn)?;
    let latest = latest_migration_version();
    if current > latest {
        if has_final_schema_markers(conn)? {
            conn.execute_batch(&format!("PRAGMA user_version = {latest}"))?;
            log::info!(
                "clamped schema user_version from {current} to {latest} (pre-release squash)"
            );
            return Ok(());
        }
        return Err(AppError::Other(format!(
            "Profile database schema version {current} is from a pre-release build and cannot be \
             upgraded automatically. Delete this profile's bg2vg.db (under profiles/<id>/) or \
             recreate the profile, then re-run Attribution / Harvest / Binding as needed."
        )));
    }

    for m in MIGRATIONS.iter().filter(|m| m.version > current) {
        let tx = conn.transaction()?;
        tx.execute_batch(m.sql)?;
        // PRAGMA user_version doesn't accept bound params - the value is our own
        // trusted constant, so formatting it in is safe.
        tx.execute_batch(&format!("PRAGMA user_version = {}", m.version))?;
        tx.commit()?;
        log::info!("applied schema migration v{}", m.version);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_are_strictly_increasing_from_one() {
        let mut prev = 0;
        for m in MIGRATIONS {
            assert!(m.version == prev + 1, "migration versions must be 1,2,3,...");
            prev = m.version;
        }
    }

    #[test]
    fn fresh_db_migrates_to_latest_and_is_idempotent() {
        let mut conn = Connection::open_in_memory().unwrap();
        assert_eq!(current_schema_version(&conn).unwrap(), 0);
        run_migrations(&mut conn).unwrap();
        assert_eq!(
            current_schema_version(&conn).unwrap(),
            latest_migration_version()
        );
        // The settings table exists.
        conn.execute("INSERT INTO settings (key, value) VALUES ('k', 'v')", [])
            .unwrap();
        // Re-running is a no-op (no error, version unchanged).
        run_migrations(&mut conn).unwrap();
        assert_eq!(
            current_schema_version(&conn).unwrap(),
            latest_migration_version()
        );
    }

    #[test]
    fn initial_schema_creates_every_domain_table() {
        let mut conn = Connection::open_in_memory().unwrap();
        run_migrations(&mut conn).unwrap();
        for table in [
            "settings",
            "project",
            "install_fingerprint",
            "speaker",
            "archetype",
            "speaker_archetype",
            "shared_strref_group",
            "line",
            "reference_sample",
            "clone",
            "clone_reference",
            "generation",
            "export",
            "metadata_binding",
            "metadata_binding_donor",
            "synthesis_text_string",
            "synthesis_text_override",
            "synthesis_text_reviewed",
            "synthesis_corpus_cache",
            "synthesis_corpus_cache_state",
            "line_render_override",
            "render_candidate",
            "dictionary_rule",
            "tag_rule",
            "voice_profile",
            "voice_profile_reference",
            "metadata_binding_profile",
            "binding_review",
        ] {
            let n: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    rusqlite::params![table],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 1, "missing domain table {table:?}");
        }
    }

    #[test]
    fn clone_has_follow_binding_columns() {
        let mut conn = Connection::open_in_memory().unwrap();
        run_migrations(&mut conn).unwrap();
        let has_follow: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('clone') WHERE name='follow_speaker_id')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(has_follow, "clone.follow_speaker_id missing");
        conn.execute(
            "INSERT INTO project (game_root, edition, active_language, generator_version, created_at)
             VALUES ('r', 'BG2EE', 'en_US', '0.1.0', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO speaker(project_id,cre_resref) VALUES(1,'A'),(1,'B')",
            [],
        )
        .unwrap();
        // follow without follow_speaker_id must fail the CHECK.
        let bad = conn.execute(
            "INSERT INTO clone(speaker_id, binding_source, follow_speaker_id)
             VALUES(1, 'follow', NULL)",
            [],
        );
        assert!(bad.is_err(), "follow without follow_speaker_id should fail");
        conn.execute(
            "INSERT INTO clone(speaker_id, binding_source, follow_speaker_id)
             VALUES(1, 'follow', 2)",
            [],
        )
        .unwrap();
    }

    #[test]
    fn performance_indexes_are_present() {
        let mut conn = Connection::open_in_memory().unwrap();
        run_migrations(&mut conn).unwrap();
        for index in [
            "ix_line_project_status_order",
            "ix_line_project_speaker",
            "ix_generation_done_line",
            "ix_clone_status_speaker",
            "ix_sample_approved_speaker",
            "ix_synthesis_corpus_page",
        ] {
            let found: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name=?1)",
                    [index],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(found, "missing performance index {index}");
        }
    }

    #[test]
    fn line_status_check_rejects_unknown_token() {
        let mut conn = Connection::open_in_memory().unwrap();
        run_migrations(&mut conn).unwrap();
        conn.execute(
            "INSERT INTO project (game_root, edition, active_language, generator_version, created_at)
             VALUES ('r', 'BG2EE', 'en_US', '0.1.0', 'now')",
            [],
        )
        .unwrap();
        let pid = conn.last_insert_rowid();
        let bad = conn.execute(
            "INSERT INTO line (project_id, strref, status) VALUES (?1, 1, 'not_a_status')",
            rusqlite::params![pid],
        );
        assert!(bad.is_err(), "CHECK should reject unknown status token");
    }

    #[test]
    fn clamps_legacy_user_version_when_final_schema_present() {
        let mut conn = Connection::open_in_memory().unwrap();
        run_migrations(&mut conn).unwrap();
        assert_eq!(current_schema_version(&conn).unwrap(), 1);
        conn.execute_batch("PRAGMA user_version = 11").unwrap();
        assert_eq!(current_schema_version(&conn).unwrap(), 11);
        run_migrations(&mut conn).unwrap();
        assert_eq!(current_schema_version(&conn).unwrap(), 1);
    }

    #[test]
    fn refuses_legacy_user_version_without_final_schema_markers() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA user_version = 11").unwrap();
        let err = run_migrations(&mut conn).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("pre-release") || msg.contains("bg2vg.db"),
            "unexpected error: {msg}"
        );
    }
}
