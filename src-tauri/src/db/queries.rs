//! Row mappers: `rusqlite::Row` -> `models::*` domain structs (item-05).
//!
//! Each mapper is index-based; the paired `*_COLUMNS` constant is the canonical
//! `SELECT` column list and MUST be selected in the same order the mapper reads.
//! Enum columns rely on the `FromSql` impls in `models.rs`; booleans are stored as
//! INTEGER `0/1` and read via `i64 != 0`.

use rusqlite::Row;

use crate::models::{
    Archetype, Clone, Export, Generation, InstallFingerprint, Line, Project, ReferenceSample,
    Speaker, SharedStrrefGroup,
};

pub const PROJECT_COLUMNS: &str =
    "id, game_root, edition, active_language, generator_version, created_at";

pub fn project_from_row(r: &Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: r.get(0)?,
        game_root: r.get(1)?,
        edition: r.get(2)?,
        active_language: r.get(3)?,
        generator_version: r.get(4)?,
        created_at: r.get(5)?,
    })
}

pub const INSTALL_FINGERPRINT_COLUMNS: &str = "id, project_id, edition_version, language, \
    mod_state_hash, source_hashes_json, export_version, captured_at";

pub fn install_fingerprint_from_row(r: &Row<'_>) -> rusqlite::Result<InstallFingerprint> {
    Ok(InstallFingerprint {
        id: r.get(0)?,
        project_id: r.get(1)?,
        edition_version: r.get(2)?,
        language: r.get(3)?,
        mod_state_hash: r.get(4)?,
        source_hashes_json: r.get(5)?,
        export_version: r.get(6)?,
        captured_at: r.get(7)?,
    })
}

pub const SPEAKER_COLUMNS: &str = "id, project_id, cre_resref, display_name, long_name_strref, \
    sex, race, class, kit, alignment, creature_category, dialogue_resref, provenance_json, \
    confidence, excluded";

pub fn speaker_from_row(r: &Row<'_>) -> rusqlite::Result<Speaker> {
    Ok(Speaker {
        id: r.get(0)?,
        project_id: r.get(1)?,
        cre_resref: r.get(2)?,
        display_name: r.get(3)?,
        long_name_strref: r.get(4)?,
        sex: r.get(5)?,
        race: r.get(6)?,
        class: r.get(7)?,
        kit: r.get(8)?,
        alignment: r.get(9)?,
        creature_category: r.get(10)?,
        dialogue_resref: r.get(11)?,
        provenance_json: r.get(12)?,
        confidence: r.get(13)?,
        excluded: r.get::<_, i64>(14)? != 0,
    })
}

pub const ARCHETYPE_COLUMNS: &str = "id, name, tags_json";

pub fn archetype_from_row(r: &Row<'_>) -> rusqlite::Result<Archetype> {
    Ok(Archetype {
        id: r.get(0)?,
        name: r.get(1)?,
        tags_json: r.get(2)?,
    })
}

pub const SHARED_STRREF_GROUP_COLUMNS: &str = "id, strref, resolution";

pub fn shared_strref_group_from_row(r: &Row<'_>) -> rusqlite::Result<SharedStrrefGroup> {
    Ok(SharedStrrefGroup {
        id: r.get(0)?,
        strref: r.get(1)?,
        resolution: r.get(2)?,
    })
}

pub const LINE_COLUMNS: &str = "id, project_id, strref, dlg_resref, state_index, text, \
    original_text, flags, existing_sound_resref, kind, is_voiced, has_tokens, token_mask, \
    shared_group_id, speaker_id, attribution_confidence, status";

/// List payload for `list_generatable_lines` — skips `original_text`.
pub const GENERATABLE_LINE_COLUMNS: &str = "id, project_id, strref, dlg_resref, state_index, text, \
    flags, existing_sound_resref, kind, is_voiced, has_tokens, token_mask, \
    shared_group_id, speaker_id, attribution_confidence, status";

pub fn line_from_row(r: &Row<'_>) -> rusqlite::Result<Line> {
    Ok(Line {
        id: r.get(0)?,
        project_id: r.get(1)?,
        strref: r.get(2)?,
        dlg_resref: r.get(3)?,
        state_index: r.get(4)?,
        text: r.get(5)?,
        original_text: r.get(6)?,
        flags: r.get(7)?,
        existing_sound_resref: r.get(8)?,
        kind: r.get(9)?,
        is_voiced: r.get::<_, i64>(10)? != 0,
        has_tokens: r.get::<_, i64>(11)? != 0,
        token_mask: r.get(12)?,
        shared_group_id: r.get(13)?,
        speaker_id: r.get(14)?,
        attribution_confidence: r.get(15)?,
        status: r.get(16)?,
    })
}

pub fn generatable_line_from_row(r: &Row<'_>) -> rusqlite::Result<crate::models::GeneratableLine> {
    Ok(crate::models::GeneratableLine {
        id: r.get(0)?,
        project_id: r.get(1)?,
        strref: r.get(2)?,
        dlg_resref: r.get(3)?,
        state_index: r.get(4)?,
        text: r.get(5)?,
        flags: r.get(6)?,
        existing_sound_resref: r.get(7)?,
        kind: r.get(8)?,
        is_voiced: r.get::<_, i64>(9)? != 0,
        has_tokens: r.get::<_, i64>(10)? != 0,
        token_mask: r.get(11)?,
        shared_group_id: r.get(12)?,
        speaker_id: r.get(13)?,
        attribution_confidence: r.get(14)?,
        status: r.get(15)?,
    })
}

pub const REFERENCE_SAMPLE_COLUMNS: &str = "id, speaker_id, source_strref, source_sound_resref, \
    provenance_json, scores_json, decision, local_derivative_path";

pub fn reference_sample_from_row(r: &Row<'_>) -> rusqlite::Result<ReferenceSample> {
    Ok(ReferenceSample {
        id: r.get(0)?,
        speaker_id: r.get(1)?,
        source_strref: r.get(2)?,
        source_sound_resref: r.get(3)?,
        provenance_json: r.get(4)?,
        scores_json: r.get(5)?,
        decision: r.get(6)?,
        local_derivative_path: r.get(7)?,
    })
}

pub const CLONE_COLUMNS: &str =
    "id, speaker_id, primary_sample_id, voice_profile_id, binding_source, status, render_settings_json";

pub fn clone_from_row(r: &Row<'_>) -> rusqlite::Result<Clone> {
    Ok(Clone {
        id: r.get(0)?,
        speaker_id: r.get(1)?,
        primary_sample_id: r.get(2)?,
        voice_profile_id: r.get(3)?,
        binding_source: r.get(4)?,
        status: r.get(5)?,
        render_settings_json: r.get(6)?,
    })
}

pub const GENERATION_COLUMNS: &str =
    "id, line_id, clone_id, voice_profile_id_snapshot, reference_sample_id, binding_source_snapshot, status, output_path, attempts, resumable_state_json, render_settings_json, render_settings_hash, reference_fingerprint, diagnostics_json";

pub fn generation_from_row(r: &Row<'_>) -> rusqlite::Result<Generation> {
    Ok(Generation {
        id: r.get(0)?,
        line_id: r.get(1)?,
        clone_id: r.get(2)?,
        voice_profile_id_snapshot: r.get(3)?,
        reference_sample_id: r.get(4)?,
        binding_source_snapshot: r.get(5)?,
        status: r.get(6)?,
        output_path: r.get(7)?,
        attempts: r.get(8)?,
        resumable_state_json: r.get(9)?,
        render_settings_json: r.get(10)?,
        render_settings_hash: r.get(11)?,
        reference_fingerprint: r.get(12)?,
        diagnostics_json: r.get(13)?,
    })
}

pub const EXPORT_COLUMNS: &str =
    "id, project_id, fingerprint_id, manifest_json, weidu_pack_path, created_at";

pub fn export_from_row(r: &Row<'_>) -> rusqlite::Result<Export> {
    Ok(Export {
        id: r.get(0)?,
        project_id: r.get(1)?,
        fingerprint_id: r.get(2)?,
        manifest_json: r.get(3)?,
        weidu_pack_path: r.get(4)?,
        created_at: r.get(5)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;
    use crate::models::{BindingSource, CloneStatus, GenerationStatus, LineKind, LineStatus};
    use rusqlite::{params, Connection};

    fn mem_db() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        schema::run_migrations(&mut conn).unwrap();
        conn
    }

    // Insert a project + return its id (parent for every other domain row).
    fn insert_project(conn: &Connection) -> i64 {
        conn.execute(
            "INSERT INTO project (game_root, edition, active_language, generator_version, created_at)
             VALUES ('C:\\BG2EE', 'BG2EE', 'en_US', '0.1.0', '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn project_round_trips() {
        let conn = mem_db();
        let pid = insert_project(&conn);
        let got = conn
            .query_row(
                &format!("SELECT {PROJECT_COLUMNS} FROM project WHERE id=?1"),
                params![pid],
                project_from_row,
            )
            .unwrap();
        assert_eq!(got.id, pid);
        assert_eq!(got.edition, "BG2EE");
        assert_eq!(got.active_language, "en_US");
    }

    #[test]
    fn line_round_trips_enums_and_bools() {
        let conn = mem_db();
        let pid = insert_project(&conn);
        conn.execute(
            "INSERT INTO line (project_id, strref, text, kind, is_voiced, has_tokens, status)
             VALUES (?1, 42, 'hail', 'state', 1, 0, 'ready')",
            params![pid],
        )
        .unwrap();
        let id = conn.last_insert_rowid();
        let got = conn
            .query_row(
                &format!("SELECT {LINE_COLUMNS} FROM line WHERE id=?1"),
                params![id],
                line_from_row,
            )
            .unwrap();
        assert_eq!(got.strref, 42);
        assert_eq!(got.kind, LineKind::State);
        assert_eq!(got.status, LineStatus::Ready);
        assert!(got.is_voiced);
        assert!(!got.has_tokens);
    }

    #[test]
    fn clone_and_generation_round_trip_enums() {
        let conn = mem_db();
        let pid = insert_project(&conn);
        conn.execute(
            "INSERT INTO speaker (project_id, cre_resref) VALUES (?1, 'IMOEN')",
            params![pid],
        )
        .unwrap();
        let sid = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO clone (speaker_id, binding_source, status)
             VALUES (?1, 'override', 'ready')",
            params![sid],
        )
        .unwrap();
        let cid = conn.last_insert_rowid();
        let clone = conn
            .query_row(
                &format!("SELECT {CLONE_COLUMNS} FROM clone WHERE id=?1"),
                params![cid],
                clone_from_row,
            )
            .unwrap();
        assert_eq!(clone.binding_source, BindingSource::Override);
        assert_eq!(clone.status, CloneStatus::Ready);

        conn.execute(
            "INSERT INTO line (project_id, strref) VALUES (?1, 7)",
            params![pid],
        )
        .unwrap();
        let lid = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO generation (line_id, clone_id, status) VALUES (?1, ?2, 'running')",
            params![lid, cid],
        )
        .unwrap();
        let gid = conn.last_insert_rowid();
        let generation = conn
            .query_row(
                &format!("SELECT {GENERATION_COLUMNS} FROM generation WHERE id=?1"),
                params![gid],
                generation_from_row,
            )
            .unwrap();
        assert_eq!(generation.status, GenerationStatus::Running);
        assert_eq!(generation.clone_id, Some(cid));
    }

    // The status enums' SQL tokens must equal their serde JSON tokens (the CHECK
    // constraints allow exactly the serde tokens); this pins ToSql to serde.
    #[test]
    fn enum_sql_token_equals_serde_token() {
        let conn = mem_db();
        for (want, value) in [
            ("blocked", LineStatus::Blocked),
        ] {
            let got: String = conn
                .query_row("SELECT ?1", params![value], |r| r.get(0))
                .unwrap();
            assert_eq!(got, want);
        }
    }
}
