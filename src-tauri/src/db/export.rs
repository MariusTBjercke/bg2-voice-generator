//! DB helpers for the WeiDU pack export (item-09).
//!
//! `list_export_candidates` joins each `done` generation to its `line`, the line's
//! `speaker` (for the clone-source resref), and the speaker's `clone` (binding tier),
//! plus the line's shared-strref group resolution. The PURE `export::plan` layer
//! makes the final keep/defer decision from these rows - this query is deliberately
//! permissive (it returns `done` generations regardless of line status) so the plan
//! can record a reason for every excluded line. `insert_fingerprint` +
//! `record_export` persist the item-05 `install_fingerprint` and `export` rows.

use rusqlite::{params, Connection};

use crate::error::AppError;
use crate::export::Candidate;
use crate::fingerprint::PackFingerprintInputs;
use crate::models::{LineKind, SharedResolution};

/// Every `done` generation in a project, joined to the facts the exporter's plan
/// needs. `clip_on_disk` is filled here from the stored `output_path` so the plan
/// stays pure. Lines missing a speaker/clone still come back (with empty resref /
/// `default` tier) so the plan can defer them explicitly.
///
/// `voice_changed` uses the same currency rules as
/// [`crate::db::generation::completed_generations_for_project`]: profile id when
/// the clone is profile-bound, otherwise primary sample id.
pub fn list_export_candidates(
    conn: &Connection,
    project_id: i64,
) -> Result<Vec<Candidate>, AppError> {
    let sql = format!(
        "{cte} \
         SELECT l.id, l.strref, l.text, l.original_text, l.kind, l.is_voiced, l.has_tokens, l.speaker_id, \
                l.shared_group_id, l.existing_sound_resref, g.output_path, \
                COALESCE(s.cre_resref, ''), \
                COALESCE(g.binding_source_snapshot, c.binding_source, 'default'), \
                COALESCE(grp.resolution, 'reuse_same_voice'), \
                {voice_changed}, \
                COALESCE(s.excluded, 0) \
         FROM generation g \
         JOIN line l ON l.id = g.line_id \
         LEFT JOIN speaker s ON s.id = l.speaker_id \
         LEFT JOIN clone c ON c.speaker_id = l.speaker_id \
         LEFT JOIN resolved_voice rv ON rv.origin_speaker_id = l.speaker_id \
         LEFT JOIN shared_strref_group grp ON grp.id = l.shared_group_id \
         WHERE l.project_id = ?1 AND g.status = 'done' \
         ORDER BY l.strref",
        cte = crate::db::follow_binding::RESOLVED_VOICE_CTE,
        voice_changed = crate::db::follow_binding::VOICE_CHANGED_CASE,
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![project_id], |r| {
        let kind: LineKind = r.get(4)?;
        let output_path: Option<String> = r.get(10)?;
        let resolution: SharedResolution = r.get(13)?;
        Ok(Candidate {
            line_id: r.get(0)?,
            strref: r.get(1)?,
            text: r.get(2)?,
            original_text: r.get(3)?,
            kind,
            is_voiced: r.get::<_, i64>(5)? != 0,
            has_tokens: r.get::<_, i64>(6)? != 0,
            speaker_id: r.get(7)?,
            shared_group_id: r.get(8)?,
            existing_sound_resref: r.get(9)?,
            shared_deferred: resolution == SharedResolution::DeferDiffVoice,
            speaker_resref: r.get::<_, String>(11)?.to_ascii_uppercase(),
            binding_source: r.get(12)?,
            voice_changed: r.get::<_, i64>(14)? != 0,
            speaker_excluded: r.get::<_, i64>(15)? != 0,
            clip_on_disk: output_path
                .as_deref()
                .map(|p| std::path::Path::new(p).exists())
                .unwrap_or(false),
            audio_source_path: output_path.unwrap_or_default(),
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Persist a captured fingerprint; returns its row id (referenced by the export).
pub fn insert_fingerprint(
    conn: &Connection,
    project_id: i64,
    fp: &PackFingerprintInputs,
    export_version: &str,
) -> Result<i64, AppError> {
    let now = chrono::Utc::now().to_rfc3339();
    let source_hashes = serde_json::json!({
        "tlk_entry_count": fp.tlk_entry_count,
    })
    .to_string();
    conn.execute(
        "INSERT INTO install_fingerprint \
            (project_id, edition_version, language, mod_state_hash, source_hashes_json, \
             export_version, captured_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            project_id,
            fp.edition_version,
            fp.language,
            fp.mod_state_hash,
            source_hashes,
            export_version,
            now
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Record an export row (the manifest JSON + the on-disk pack path). Returns its id.
pub fn record_export(
    conn: &Connection,
    project_id: i64,
    fingerprint_id: i64,
    manifest_json: &str,
    pack_path: &str,
) -> Result<i64, AppError> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO export (project_id, fingerprint_id, manifest_json, weidu_pack_path, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![project_id, fingerprint_id, manifest_json, pack_path, now],
    )?;
    Ok(conn.last_insert_rowid())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;

    fn db() -> Connection {
        let mut c = Connection::open_in_memory().unwrap();
        schema::run_migrations(&mut c).unwrap();
        c.execute(
            "INSERT INTO project (game_root, edition, active_language, generator_version, created_at) \
             VALUES ('r','BG2EE','en_US','0.1.0','now')",
            [],
        )
        .unwrap();
        c
    }

    fn done_line(c: &Connection, strref: i64, out: &str) -> i64 {
        c.execute(
            "INSERT INTO speaker (project_id, cre_resref) VALUES (1,'XZAR')",
            [],
        )
        .ok();
        let sid: i64 = c
            .query_row("SELECT id FROM speaker LIMIT 1", [], |r| r.get(0))
            .unwrap();
        c.execute(
            "INSERT INTO line (project_id, strref, text, speaker_id, kind) VALUES (1,?1,'Hi',?2,'state')",
            params![strref, sid],
        )
        .unwrap();
        let lid = c.last_insert_rowid();
        c.execute(
            "INSERT INTO reference_sample (speaker_id, decision, local_derivative_path) \
             VALUES (?1,'approved','/ws/ref.wav')",
            params![sid],
        )
        .unwrap();
        let sample_id = c.last_insert_rowid();
        c.execute(
            "INSERT INTO clone (speaker_id, primary_sample_id, binding_source, status) \
             VALUES (?1,?2,'default','ready')",
            params![sid, sample_id],
        )
        .unwrap();
        let cid = c.last_insert_rowid();
        c.execute(
            "INSERT INTO generation (line_id, clone_id, reference_sample_id, binding_source_snapshot, \
             status, output_path, render_settings_hash) \
             VALUES (?1,?2,?3,'default','done',?4,'fixture-settings-hash')",
            params![lid, cid, sample_id, out],
        )
        .unwrap();
        lid
    }

    #[test]
    fn lists_only_done_generations_with_joined_facts() {
        let c = db();
        let lid = done_line(&c, 22570, "/ws/x.wav");
        let cands = list_export_candidates(&c, 1).unwrap();
        assert_eq!(cands.len(), 1);
        let cand = &cands[0];
        assert_eq!(cand.line_id, lid);
        assert_eq!(cand.strref, 22570);
        assert_eq!(cand.speaker_resref, "XZAR");
        assert_eq!(cand.binding_source, "default");
        assert!(!cand.voice_changed);
        assert!(!cand.shared_deferred);
        assert!(!cand.clip_on_disk, "the missing path is reported as not-on-disk");
    }

    #[test]
    fn voice_changed_export_keeps_render_time_binding_metadata() {
        let c = db();
        done_line(&c, 22570, "/ws/x.wav");
        let sid: i64 = c.query_row("SELECT id FROM speaker LIMIT 1", [], |r| r.get(0)).unwrap();
        c.execute(
            "INSERT INTO reference_sample (speaker_id, decision, local_derivative_path) \
             VALUES (?1,'approved','/ws/new.wav')",
            params![sid],
        )
        .unwrap();
        let new_sample = c.last_insert_rowid();
        c.execute(
            "UPDATE clone SET primary_sample_id=?1, binding_source='override', status='ready' \
             WHERE speaker_id=?2",
            params![new_sample, sid],
        )
        .unwrap();

        let cand = list_export_candidates(&c, 1).unwrap().pop().unwrap();
        assert!(cand.voice_changed);
        assert_eq!(cand.binding_source, "default");
        assert_eq!(cand.audio_source_path, "/ws/x.wav");
    }

    #[test]
    fn null_render_settings_hash_marks_export_candidate_voice_changed() {
        let c = db();
        done_line(&c, 22570, "/ws/x.wav");
        c.execute(
            "UPDATE generation SET render_settings_hash=NULL WHERE output_path='/ws/x.wav'",
            [],
        )
        .unwrap();

        let cand = list_export_candidates(&c, 1).unwrap().pop().unwrap();
        assert!(cand.voice_changed);
        assert_eq!(cand.audio_source_path, "/ws/x.wav");
        assert!(!cand.clip_on_disk);
    }

    #[test]
    fn profile_bound_clone_with_null_primary_is_not_voice_changed_when_snapshot_matches() {
        // Matches Generation: profile id is the currency check; primary_sample_id may be NULL.
        let c = db();
        done_line(&c, 22570, "/ws/x.wav");
        let sid: i64 = c.query_row("SELECT id FROM speaker LIMIT 1", [], |r| r.get(0)).unwrap();
        c.execute(
            "INSERT INTO voice_profile(project_id,display_name,origin,availability,created_at,updated_at) \
             VALUES(1,'Pool','designed','available','now','now')",
            [],
        )
        .unwrap();
        let profile_id = c.last_insert_rowid();
        c.execute(
            "UPDATE clone SET primary_sample_id=NULL, voice_profile_id=?1, status='ready' \
             WHERE speaker_id=?2",
            params![profile_id, sid],
        )
        .unwrap();
        c.execute(
            "UPDATE generation SET voice_profile_id_snapshot=?1, reference_sample_id=NULL \
             WHERE output_path='/ws/x.wav'",
            params![profile_id],
        )
        .unwrap();

        let cand = list_export_candidates(&c, 1).unwrap().pop().unwrap();
        assert!(
            !cand.voice_changed,
            "current profile snapshot must not report voice_changed"
        );
    }

    #[test]
    fn profile_rebind_marks_export_candidate_voice_changed() {
        let c = db();
        done_line(&c, 22570, "/ws/x.wav");
        let sid: i64 = c.query_row("SELECT id FROM speaker LIMIT 1", [], |r| r.get(0)).unwrap();
        c.execute(
            "INSERT INTO voice_profile(project_id,display_name,origin,availability,created_at,updated_at) \
             VALUES(1,'Old','designed','available','now','now'), \
                   (1,'New','designed','available','now','now')",
            [],
        )
        .unwrap();
        let old_profile: i64 = c
            .query_row(
                "SELECT id FROM voice_profile WHERE display_name='Old'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let new_profile: i64 = c
            .query_row(
                "SELECT id FROM voice_profile WHERE display_name='New'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        c.execute(
            "UPDATE clone SET primary_sample_id=NULL, voice_profile_id=?1, status='ready' \
             WHERE speaker_id=?2",
            params![new_profile, sid],
        )
        .unwrap();
        c.execute(
            "UPDATE generation SET voice_profile_id_snapshot=?1, reference_sample_id=NULL \
             WHERE output_path='/ws/x.wav'",
            params![old_profile],
        )
        .unwrap();

        let cand = list_export_candidates(&c, 1).unwrap().pop().unwrap();
        assert!(cand.voice_changed);
    }

    #[test]
    fn includes_original_text_for_token_resolved_lines() {
        let c = db();
        let line_id = done_line(&c, 22570, "/ws/x.wav");
        c.execute(
            "UPDATE line SET text='Hello Hero.', original_text='Hello <CHARNAME>.' WHERE id=?1",
            params![line_id],
        )
        .unwrap();

        let cand = list_export_candidates(&c, 1).unwrap().pop().unwrap();
        assert_eq!(cand.text, "Hello Hero.");
        assert_eq!(cand.original_text, "Hello <CHARNAME>.");
    }

    #[test]
    fn fingerprint_and_export_rows_round_trip() {
        let c = db();
        let fp = PackFingerprintInputs {
            edition: "bg2ee".into(),
            edition_version: "2.6".into(),
            language: "en_US".into(),
            mod_state_hash: "h".into(),
            tlk_entry_count: 103_778,
        };
        let fid = insert_fingerprint(&c, 1, &fp, "1").unwrap();
        let eid = record_export(&c, 1, fid, "{}", "/ws/exports/BG2VG").unwrap();
        assert!(fid > 0 && eid > 0);
        let (mj, pp): (String, String) = c
            .query_row(
                "SELECT manifest_json, weidu_pack_path FROM export WHERE id=?1",
                params![eid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(mj, "{}");
        assert_eq!(pp, "/ws/exports/BG2VG");
    }
}
