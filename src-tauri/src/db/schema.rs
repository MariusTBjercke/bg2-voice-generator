//! Versioned schema migrations, tracked by SQLite's `PRAGMA user_version`.
//!
//! v1 is the initial release schema: settings, the full BG2 domain tables, metadata
//! binding pools, synthesis-text overrides/review, speaker-identity grouping, clone
//! render settings, ordered clone references, line render experiments, generation
//! diagnostics, and machine-wide dictionary rules.
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

/// The ordered migration list. Append-only.
const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: V1_INITIAL_SCHEMA,
    },
    Migration {
        version: 2,
        sql: V2_GENERATION_SYNTHESIS_STALE,
    },
    Migration {
        version: 3,
        sql: V3_TAG_RULE,
    },
    Migration {
        version: 4,
        sql: V4_QUERY_PERFORMANCE,
    },
    Migration {
        version: 5,
        sql: V5_VOICE_PROFILES,
    },
    Migration {
        version: 6,
        sql: V6_SPEAKER_EXCLUDED,
    },
    Migration {
        version: 7,
        sql: V7_SAME_SOUND_DECISION_REPAIR_GATE,
    },
    Migration {
        version: 8,
        sql: V8_BINDING_REVIEW,
    },
    Migration {
        version: 9,
        sql: V9_FOLLOW_BINDING_GATE,
    },
];

/// v9 — live “follow character” bindings. DDL runs in a Rust hook so foreign keys
/// can be disabled for the clone/generation table rebuilds (CHECK cannot be widened
/// with ALTER TABLE).
const V9_FOLLOW_BINDING_GATE: &str = r#"
SELECT 1;
"#;

/// v6 — per-speaker exclude-from-generate/export flag (Binding identity-group toggle).
const V6_SPEAKER_EXCLUDED: &str = r#"
ALTER TABLE speaker ADD COLUMN excluded INTEGER NOT NULL DEFAULT 0
    CHECK (excluded IN (0, 1));
"#;

/// v7 — gate for the same-sound sample-decision repair (Rust hook in `run_migrations`).
/// No DDL; the data repair syncs pending siblings of a shared sound resref within a
/// display identity group after partial legacy approvals.
const V7_SAME_SOUND_DECISION_REPAIR_GATE: &str = r#"
SELECT 1;
"#;

/// v8 — local agent/human markers for personal voice-binding audit (CRE-stable keys).
const V8_BINDING_REVIEW: &str = r#"
CREATE TABLE binding_review (
    project_id  INTEGER NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    cre_resref  TEXT NOT NULL,
    status      TEXT NOT NULL CHECK (status IN ('flagged', 'reviewed')),
    reason      TEXT NOT NULL DEFAULT '',
    updated_at  TEXT NOT NULL,
    PRIMARY KEY (project_id, cre_resref)
);
CREATE INDEX idx_binding_review_project_status
    ON binding_review(project_id, status);
"#;

/// v5 — reusable project voice profiles. Legacy sample columns and donor rows stay
/// readable for older bundles and compatibility commands, but every existing clone
/// and demographic donor is backfilled to an equivalent harvested profile.
const V5_VOICE_PROFILES: &str = r#"
CREATE TABLE voice_profile (
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
CREATE INDEX ix_voice_profile_project ON voice_profile(project_id, id);
CREATE INDEX ix_voice_profile_harvested_speaker ON voice_profile(harvested_speaker_id);

CREATE TABLE voice_profile_reference (
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
CREATE INDEX ix_voice_profile_reference_profile
    ON voice_profile_reference(voice_profile_id, sort_order);
CREATE INDEX ix_voice_profile_reference_sample
    ON voice_profile_reference(reference_sample_id);

ALTER TABLE clone ADD COLUMN voice_profile_id INTEGER REFERENCES voice_profile(id) ON DELETE SET NULL;
CREATE INDEX ix_clone_voice_profile ON clone(voice_profile_id);
ALTER TABLE generation ADD COLUMN voice_profile_id_snapshot INTEGER;

CREATE TABLE metadata_binding_profile (
    binding_id      INTEGER NOT NULL REFERENCES metadata_binding(id) ON DELETE CASCADE,
    voice_profile_id INTEGER NOT NULL REFERENCES voice_profile(id) ON DELETE CASCADE,
    sort_order      INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(binding_id, voice_profile_id),
    UNIQUE(binding_id, sort_order)
);
CREATE INDEX ix_metadata_profile_binding ON metadata_binding_profile(binding_id, sort_order);

-- One harvested profile per distinct ordered clone reference set.
WITH clone_sets AS (
    SELECT c.id AS clone_id, s.project_id, c.speaker_id,
           COALESCE(
             (SELECT group_concat(sample_id, ',') FROM
                (SELECT sample_id FROM clone_reference WHERE clone_id=c.id ORDER BY sort_order)),
             CAST(c.primary_sample_id AS TEXT), 'none:' || c.id
           ) AS signature
    FROM clone c JOIN speaker s ON s.id=c.speaker_id
    WHERE c.primary_sample_id IS NOT NULL
       OR EXISTS (SELECT 1 FROM clone_reference cr WHERE cr.clone_id=c.id)
), distinct_sets AS (
    SELECT project_id, signature, MIN(clone_id) AS clone_id, MIN(speaker_id) AS speaker_id
    FROM clone_sets GROUP BY project_id, signature
)
INSERT INTO voice_profile(project_id,display_name,origin,harvested_speaker_id,availability,
                          reference_fingerprint,created_at,updated_at)
SELECT ds.project_id,
       'Harvested — ' || COALESCE(s.display_name,s.cre_resref),
       'harvested', ds.speaker_id, 'available', 'legacy:' || ds.signature,
       datetime('now'), datetime('now')
FROM distinct_sets ds JOIN speaker s ON s.id=ds.speaker_id;

WITH clone_sets AS (
    SELECT c.id AS clone_id, s.project_id,
           COALESCE(
             (SELECT group_concat(sample_id, ',') FROM
                (SELECT sample_id FROM clone_reference WHERE clone_id=c.id ORDER BY sort_order)),
             CAST(c.primary_sample_id AS TEXT), 'none:' || c.id
           ) AS signature
    FROM clone c JOIN speaker s ON s.id=c.speaker_id
    WHERE c.primary_sample_id IS NOT NULL
       OR EXISTS (SELECT 1 FROM clone_reference cr WHERE cr.clone_id=c.id)
)
UPDATE clone SET voice_profile_id=(
    SELECT vp.id FROM clone_sets cs JOIN voice_profile vp
      ON vp.project_id=cs.project_id AND vp.reference_fingerprint='legacy:' || cs.signature
    WHERE cs.clone_id=clone.id LIMIT 1
);

INSERT INTO voice_profile_reference(voice_profile_id,reference_sample_id,transcript,sort_order)
SELECT c.voice_profile_id, cr.sample_id,
       COALESCE(json_extract(rs.provenance_json,'$.source_text'),''), cr.sort_order
FROM clone c JOIN clone_reference cr ON cr.clone_id=c.id
JOIN reference_sample rs ON rs.id=cr.sample_id
WHERE c.id=(SELECT MIN(c2.id) FROM clone c2 WHERE c2.voice_profile_id=c.voice_profile_id);

INSERT INTO voice_profile_reference(voice_profile_id,reference_sample_id,transcript,sort_order)
SELECT c.voice_profile_id, c.primary_sample_id,
       COALESCE(json_extract(rs.provenance_json,'$.source_text'),''), 0
FROM clone c JOIN reference_sample rs ON rs.id=c.primary_sample_id
WHERE c.voice_profile_id IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM voice_profile_reference vpr WHERE vpr.voice_profile_id=c.voice_profile_id)
  AND c.id=(SELECT MIN(c2.id) FROM clone c2 WHERE c2.voice_profile_id=c.voice_profile_id);

-- Donors without a clone still become reusable harvested profiles.
INSERT INTO voice_profile(project_id,display_name,origin,harvested_speaker_id,availability,
                          reference_fingerprint,created_at,updated_at)
SELECT DISTINCT s.project_id, 'Harvested — ' || COALESCE(s.display_name,s.cre_resref),
       'harvested', s.id, 'available', 'legacy-donor:' || s.id,
       datetime('now'), datetime('now')
FROM metadata_binding_donor mbd JOIN speaker s ON s.id=mbd.donor_speaker_id
WHERE NOT EXISTS (
    SELECT 1 FROM voice_profile vp
    WHERE vp.project_id=s.project_id AND vp.harvested_speaker_id=s.id
);

INSERT INTO voice_profile_reference(voice_profile_id,reference_sample_id,transcript,sort_order)
SELECT vp.id, rs.id, COALESCE(json_extract(rs.provenance_json,'$.source_text'),''), 0
FROM voice_profile vp JOIN reference_sample rs ON rs.id=(
    SELECT rs2.id FROM reference_sample rs2
    WHERE rs2.speaker_id=vp.harvested_speaker_id AND rs2.decision='approved'
      AND rs2.local_derivative_path IS NOT NULL ORDER BY rs2.id DESC LIMIT 1
)
WHERE vp.reference_fingerprint LIKE 'legacy-donor:%'
  AND NOT EXISTS (SELECT 1 FROM voice_profile_reference x WHERE x.voice_profile_id=vp.id);

INSERT OR IGNORE INTO metadata_binding_profile(binding_id,voice_profile_id,sort_order)
SELECT mbd.binding_id, vp.id, mbd.sort_order
FROM metadata_binding_donor mbd JOIN speaker s ON s.id=mbd.donor_speaker_id
JOIN voice_profile vp ON vp.project_id=s.project_id AND vp.harvested_speaker_id=s.id
WHERE vp.id=(SELECT MIN(vp2.id) FROM voice_profile vp2
             WHERE vp2.project_id=s.project_id AND vp2.harvested_speaker_id=s.id);
"#;

/// v4 — indexes for the large list screens plus a disposable, local synthesis
/// corpus cache. The cache is derived entirely from `line` + machine-local rules;
/// transfer deliberately ignores it and callers may rebuild it at any time.
const V4_QUERY_PERFORMANCE: &str = r#"
CREATE INDEX IF NOT EXISTS ix_line_project_status_order
    ON line(project_id, status, dlg_resref, state_index);
CREATE INDEX IF NOT EXISTS ix_line_project_speaker
    ON line(project_id, speaker_id);
CREATE INDEX IF NOT EXISTS ix_generation_done_line
    ON generation(status, line_id) WHERE output_path IS NOT NULL;
CREATE INDEX IF NOT EXISTS ix_clone_status_speaker
    ON clone(status, speaker_id);
CREATE INDEX IF NOT EXISTS ix_sample_approved_speaker
    ON reference_sample(speaker_id) WHERE decision='approved';

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
    rebuilt_at   TEXT NOT NULL
);
"#;

/// v2 — keep completed clips playable when Dictionary / synthesis text drifts.
/// `synthesis_stale=1` means preview stays available but Generation can filter
/// "text changed" lines for selective re-render.
const V2_GENERATION_SYNTHESIS_STALE: &str = r#"
ALTER TABLE generation ADD COLUMN synthesis_stale INTEGER NOT NULL DEFAULT 0
    CHECK (synthesis_stale IN (0, 1));
"#;

/// v3 — machine-wide OmniVoice tag rules (stage cues + optional spoken words).
const V3_TAG_RULE: &str = r#"
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
"#;

/// v1 - the initial release schema.
///
/// Conventions (BG2 domain schema):
///   * Booleans are stored as INTEGER `0/1`; JSON blobs as TEXT; timestamps as
///     RFC3339 TEXT (chrono default).
///   * Enum-like status columns are free-form TEXT constrained by CHECK to the exact
///     serde tokens the Rust/TS contracts serialize (see `models.rs`).
///   * Large audio is referenced by filesystem PATH, never stored as a BLOB
///     (resolves the item-05 open question; matches item-00 "local derivatives" rule).
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

CREATE TABLE IF NOT EXISTS clone (
    id                INTEGER PRIMARY KEY,
    speaker_id        INTEGER NOT NULL REFERENCES speaker(id) ON DELETE CASCADE,
    primary_sample_id INTEGER REFERENCES reference_sample(id) ON DELETE SET NULL,
    binding_source    TEXT NOT NULL DEFAULT 'default'
        CHECK (binding_source IN ('override', 'default', 'generic')),
    status            TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'ready', 'failed')),
    render_settings_json TEXT NOT NULL DEFAULT
'{"speed":null,"num_steps":32,"guidance_scale":2.0,"t_shift":0.1,"layer_penalty_factor":5.0,"position_temperature":5.0,"class_temperature":0.0,"prompt_denoise":true,"preprocess_prompt":true,"postprocess_output":true,"audio_chunk_duration":10.0,"audio_chunk_threshold":30.0,"seed":42,"peak_normalize_dbfs":-1.0,"peak_normalize_inherit":true}'
);
CREATE INDEX IF NOT EXISTS ix_clone_speaker ON clone(speaker_id);

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
        CHECK (binding_source_snapshot IS NULL OR binding_source_snapshot IN ('override','default','generic')),
    render_settings_json TEXT,
    render_settings_hash TEXT,
    reference_fingerprint TEXT,
    diagnostics_json     TEXT
);
CREATE INDEX IF NOT EXISTS ix_generation_line ON generation(line_id);

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

CREATE TABLE synthesis_text_string (
    text_hash   TEXT PRIMARY KEY,
    source_text TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_synthesis_text_string_source_text
    ON synthesis_text_string(source_text);
CREATE TABLE synthesis_text_override (
    text_hash       TEXT PRIMARY KEY REFERENCES synthesis_text_string(text_hash) ON DELETE CASCADE,
    synthesis_text  TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);
CREATE TABLE synthesis_text_reviewed (
    text_hash TEXT PRIMARY KEY REFERENCES synthesis_text_string(text_hash) ON DELETE CASCADE
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

/// Apply every migration whose `version` exceeds the DB's current `user_version`, in
/// order, each in its own transaction (a failed step rolls back cleanly), bumping
/// `user_version` after each. No-op on an already-current DB.
///
/// Some versions run a Rust data hook after the SQL batch (still inside the same
/// transaction) so schema history and one-shot repairs stay coupled.
pub fn run_migrations(conn: &mut Connection) -> Result<(), AppError> {
    let current = current_schema_version(conn)?;
    for m in MIGRATIONS.iter().filter(|m| m.version > current) {
        let tx = conn.transaction()?;
        tx.execute_batch(m.sql)?;
        match m.version {
            7 => {
                crate::db::harvest::repair_same_sound_sample_decisions(&tx)?;
            }
            9 => {
                migrate_v9_follow_binding(&tx)?;
            }
            _ => {}
        }
        // PRAGMA user_version doesn't accept bound params - the value is our own
        // trusted constant, so formatting it in is safe.
        tx.execute_batch(&format!("PRAGMA user_version = {}", m.version))?;
        tx.commit()?;
        log::info!("applied schema migration v{}", m.version);
    }
    Ok(())
}

/// Rebuild `clone` / `generation` so `binding_source` may be `'follow'` and clones
/// may store `follow_speaker_id`. Existing rows keep their data; follow column is NULL.
fn migrate_v9_follow_binding(tx: &rusqlite::Transaction<'_>) -> Result<(), AppError> {
    // FK pragma is a no-op inside a transaction on some SQLite builds; recreate via
    // backup tables so CASCADE drops do not wipe clone_reference prematurely.
    tx.execute_batch(
        r#"
CREATE TABLE clone_v9 (
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
INSERT INTO clone_v9 (
    id, speaker_id, primary_sample_id, voice_profile_id, follow_speaker_id,
    binding_source, status, render_settings_json
)
SELECT id, speaker_id, primary_sample_id, voice_profile_id, NULL,
       binding_source, status, render_settings_json
FROM clone;

CREATE TABLE clone_reference_v9 AS SELECT * FROM clone_reference;

CREATE TABLE generation_v9 (
    id                   INTEGER PRIMARY KEY,
    line_id              INTEGER NOT NULL REFERENCES line(id) ON DELETE CASCADE,
    clone_id             INTEGER,
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
);
"#,
    )?;

    // Copy generation columns that exist (synthesis_stale / voice_profile_id_snapshot
    // were added in later migrations; probe via pragma).
    let gen_cols: Vec<String> = {
        let mut stmt = tx.prepare("PRAGMA table_info(generation)")?;
        let cols = stmt
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        cols
    };
    let has = |name: &str| gen_cols.iter().any(|c| c == name);
    let select_vp = if has("voice_profile_id_snapshot") {
        "voice_profile_id_snapshot"
    } else {
        "NULL"
    };
    let select_stale = if has("synthesis_stale") {
        "synthesis_stale"
    } else {
        "0"
    };
    tx.execute(
        &format!(
            "INSERT INTO generation_v9 (\
                id, line_id, clone_id, status, output_path, attempts, resumable_state_json, \
                reference_sample_id, binding_source_snapshot, render_settings_json, \
                render_settings_hash, reference_fingerprint, diagnostics_json, \
                voice_profile_id_snapshot, synthesis_stale) \
             SELECT id, line_id, clone_id, status, output_path, attempts, resumable_state_json, \
                reference_sample_id, binding_source_snapshot, render_settings_json, \
                render_settings_hash, reference_fingerprint, diagnostics_json, \
                {select_vp}, {select_stale} \
             FROM generation"
        ),
        [],
    )?;

    tx.execute_batch(
        r#"
DROP TABLE generation;
DROP TABLE clone_reference;
DROP TABLE clone;

ALTER TABLE clone_v9 RENAME TO clone;
CREATE INDEX IF NOT EXISTS ix_clone_speaker ON clone(speaker_id);
CREATE INDEX IF NOT EXISTS ix_clone_voice_profile ON clone(voice_profile_id);
CREATE INDEX IF NOT EXISTS ix_clone_follow_speaker ON clone(follow_speaker_id);
CREATE INDEX IF NOT EXISTS ix_clone_status_speaker ON clone(status, speaker_id);

CREATE TABLE clone_reference (
    clone_id   INTEGER NOT NULL REFERENCES clone(id) ON DELETE CASCADE,
    sample_id  INTEGER NOT NULL REFERENCES reference_sample(id) ON DELETE CASCADE,
    sort_order INTEGER NOT NULL CHECK (sort_order >= 0),
    PRIMARY KEY (clone_id, sample_id),
    UNIQUE (clone_id, sort_order)
);
INSERT INTO clone_reference SELECT * FROM clone_reference_v9;
DROP TABLE clone_reference_v9;
CREATE INDEX IF NOT EXISTS ix_clone_reference_sample ON clone_reference(sample_id);

ALTER TABLE generation_v9 RENAME TO generation;
-- Re-attach clone FK now that `clone` has its final name (SQLite cannot ALTER FK).
CREATE TABLE generation_v9b (
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
);
INSERT INTO generation_v9b SELECT * FROM generation;
DROP TABLE generation;
ALTER TABLE generation_v9b RENAME TO generation;
CREATE INDEX IF NOT EXISTS ix_generation_line ON generation(line_id);
CREATE INDEX IF NOT EXISTS ix_generation_done_line
    ON generation(status, line_id) WHERE output_path IS NOT NULL;
"#,
    )?;
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
        assert_eq!(current_schema_version(&conn).unwrap(), latest_migration_version());
        // The settings table exists.
        conn.execute("INSERT INTO settings (key, value) VALUES ('k', 'v')", [])
            .unwrap();
        // Re-running is a no-op (no error, version unchanged).
        run_migrations(&mut conn).unwrap();
        assert_eq!(current_schema_version(&conn).unwrap(), latest_migration_version());
    }

    #[test]
    fn initial_schema_creates_every_domain_table() {
        let mut conn = Connection::open_in_memory().unwrap();
        run_migrations(&mut conn).unwrap();
        for table in [
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
    fn v5_backfills_shared_composites_and_demographic_donors_idempotently() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(V1_INITIAL_SCHEMA).unwrap();
        conn.execute_batch(V2_GENERATION_SYNTHESIS_STALE).unwrap();
        conn.execute_batch(V3_TAG_RULE).unwrap();
        conn.execute_batch(V4_QUERY_PERFORMANCE).unwrap();
        conn.execute_batch("PRAGMA user_version=4").unwrap();
        conn.execute("INSERT INTO project(game_root,edition,active_language,generator_version,created_at) VALUES('r','BG2EE','en_US','0.1.0','now')", []).unwrap();
        conn.execute("INSERT INTO speaker(project_id,cre_resref,display_name) VALUES(1,'A','Shared'),(1,'B','Shared'),(1,'C','Unbound')", []).unwrap();
        conn.execute("INSERT INTO reference_sample(speaker_id,decision,local_derivative_path,provenance_json) VALUES(1,'approved','a.wav','{\"source_text\":\"One\"}'),(1,'approved','b.wav','{\"source_text\":\"Two\"}')", []).unwrap();
        conn.execute("INSERT INTO clone(speaker_id,primary_sample_id,status) VALUES(1,1,'ready'),(2,1,'ready'),(3,NULL,'pending')", []).unwrap();
        conn.execute("INSERT INTO clone_reference(clone_id,sample_id,sort_order) VALUES(1,1,0),(1,2,1),(2,1,0),(2,2,1)", []).unwrap();
        conn.execute("INSERT INTO metadata_binding(project_id,sex,race,creature_category) VALUES(1,1,1,1)", []).unwrap();
        conn.execute("INSERT INTO metadata_binding_donor(binding_id,donor_speaker_id,sort_order) VALUES(1,1,0)", []).unwrap();

        run_migrations(&mut conn).unwrap();
        let profile_count: i64 = conn.query_row("SELECT COUNT(*) FROM voice_profile WHERE reference_fingerprint='legacy:1,2'", [], |r| r.get(0)).unwrap();
        assert_eq!(profile_count, 1);
        let linked: i64 = conn.query_row("SELECT COUNT(DISTINCT voice_profile_id) FROM clone", [], |r| r.get(0)).unwrap();
        assert_eq!(linked, 1, "identical ordered legacy composites share one profile");
        let unbound_profile: Option<i64> = conn.query_row("SELECT voice_profile_id FROM clone WHERE speaker_id=3", [], |r| r.get(0)).unwrap();
        assert_eq!(unbound_profile, None, "an unbound legacy clone must not create an empty profile");
        let pool: i64 = conn.query_row("SELECT COUNT(*) FROM metadata_binding_profile", [], |r| r.get(0)).unwrap();
        assert_eq!(pool, 1);
        run_migrations(&mut conn).unwrap();
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM voice_profile", [], |r| r.get::<_,i64>(0)).unwrap(), 1);
    }
}
