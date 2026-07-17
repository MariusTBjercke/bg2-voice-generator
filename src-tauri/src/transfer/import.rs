//! Transfer IMPORT (item-12): reconstruct a project from a transfer bundle on a second
//! machine. The bundle carries CONFIGURATION + generation STATE only - never audio - so
//! after import the target must re-scan its own install, re-harvest references, and
//! regenerate clips locally. Nothing audio-bearing is written here.
//!
//! The import is one transaction: either the whole project lands or none of it does.
//! Foreign keys are re-linked from the bundle's NATURAL keys (speaker `cre_resref`,
//! shared-group `strref`, archetype `name`) to freshly-inserted row ids on this machine.

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::AppError;

use super::export::TransferBundle;
use super::{zip_err, TransferManifest, MANIFEST_ENTRY, PROJECT_ENTRY, TRANSFER_KIND, TRANSFER_VERSION};

/// Outcome of a transfer import. Mirror of `TransferImportResult` in
/// `src/lib/types/index.ts`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TransferImportResult {
    pub project_id: i64,
    pub speakers: i64,
    pub lines: i64,
    pub decisions: i64,
    pub clones: i64,
    /// Always true: a fresh import has no local audio, so every re-imported project must
    /// re-scan + re-harvest + regenerate on this machine. Surfaced so the UI can prompt.
    pub needs_local_rescan: bool,
}

/// Read + validate the bundle, then reconstruct the project bound to `game_root` (THIS
/// machine's install path). Refuses if a project already exists for that install (import
/// is create-only; the caller deletes first to re-import).
pub(crate) fn import_bundle(
    conn: &mut Connection,
    bundle_path: &str,
    game_root: &str,
) -> Result<TransferImportResult, AppError> {
    let file = File::open(bundle_path)
        .map_err(|e| AppError::Other(format!("could not open bundle: {e}")))?;
    let mut archive = zip::ZipArchive::new(file).map_err(zip_err)?;

    validate_manifest(&mut archive)?;

    let bundle: TransferBundle = {
        let raw = read_zip_text(&mut archive, PROJECT_ENTRY)?;
        serde_json::from_str(&raw)
            .map_err(|e| AppError::Other(format!("invalid transfer payload: {e}")))?
    };

    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM project WHERE game_root = ?1",
            [game_root],
            |r| r.get(0),
        )
        .optional()?;
    if existing.is_some() {
        return Err(AppError::Other(format!(
            "a project already exists for {game_root}; delete it before importing"
        )));
    }

    let tx = conn.transaction()?;
    let result = insert_bundle(&tx, &bundle, game_root)?;
    tx.commit()?;
    Ok(result)
}

fn validate_manifest(archive: &mut zip::ZipArchive<File>) -> Result<TransferManifest, AppError> {
    let raw = read_zip_text(archive, MANIFEST_ENTRY)?;
    let manifest: TransferManifest = serde_json::from_str(&raw)
        .map_err(|e| AppError::Other(format!("invalid transfer manifest: {e}")))?;
    if manifest.kind != TRANSFER_KIND {
        return Err(AppError::Other(format!(
            "expected bundle kind {TRANSFER_KIND:?}, got {:?}",
            manifest.kind
        )));
    }
    if manifest.version > TRANSFER_VERSION {
        return Err(AppError::Other(format!(
            "bundle version {} is newer than this app supports ({TRANSFER_VERSION})",
            manifest.version
        )));
    }
    Ok(manifest)
}

fn read_zip_text(archive: &mut zip::ZipArchive<File>, name: &str) -> Result<String, AppError> {
    let mut entry = archive
        .by_name(name)
        .map_err(|_| AppError::Other(format!("bundle is missing {name}")))?;
    let mut buf = String::new();
    entry.read_to_string(&mut buf)?;
    Ok(buf)
}

/// Reconstruct the whole bundle inside the caller's transaction. Returns the counts the
/// command reports. All foreign keys are re-linked from natural keys to new local ids.
fn insert_bundle(
    tx: &rusqlite::Transaction,
    bundle: &TransferBundle,
    game_root: &str,
) -> Result<TransferImportResult, AppError> {
    let p = &bundle.project;
    tx.execute(
        "INSERT INTO project (game_root, edition, active_language, generator_version, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![game_root, p.edition, p.active_language, p.generator_version, p.created_at],
    )?;
    let project_id = tx.last_insert_rowid();

    // The fingerprint travels as the SOURCE GUARD: a pack built from this imported project
    // is portable only to an install whose fingerprint matches. Stored verbatim; the
    // export path re-captures the LOCAL install's fingerprint at pack-build time.
    if let Some(fp) = &bundle.fingerprint {
        tx.execute(
            "INSERT INTO install_fingerprint \
                (project_id, edition_version, language, mod_state_hash, source_hashes_json, \
                 export_version, captured_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                project_id,
                fp.edition_version,
                fp.language,
                fp.mod_state_hash,
                fp.source_hashes_json,
                fp.export_version,
                fp.captured_at,
            ],
        )?;
    }

    // speaker cre_resref -> new local speaker id (for re-linking lines/samples/clones).
    let mut speaker_ids: HashMap<String, i64> = HashMap::new();
    {
        let mut stmt = tx.prepare(
            "INSERT INTO speaker \
                (project_id, cre_resref, display_name, long_name_strref, sex, race, class, kit, alignment, \
                 creature_category, dialogue_resref, provenance_json, confidence, excluded) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        )?;
        for s in &bundle.speakers {
            stmt.execute(params![
                project_id,
                s.cre_resref,
                s.display_name,
                s.long_name_strref,
                s.sex,
                s.race,
                s.class,
                s.kit,
                s.alignment,
                s.creature_category,
                s.dialogue_resref,
                s.provenance_json,
                s.confidence,
                if s.excluded { 1i64 } else { 0 },
            ])?;
            speaker_ids.insert(s.cre_resref.clone(), tx.last_insert_rowid());
        }
    }

    // archetype name -> local id. Archetypes are GLOBAL + name-unique, so an existing
    // same-named archetype is reused (not duplicated); the bundle's tags_json does not
    // clobber a local archetype the user already curated.
    let mut archetype_ids: HashMap<String, i64> = HashMap::new();
    for a in &bundle.archetypes {
        let existing: Option<i64> = tx
            .query_row("SELECT id FROM archetype WHERE name = ?1", [&a.name], |r| r.get(0))
            .optional()?;
        let id = match existing {
            Some(id) => id,
            None => {
                tx.execute(
                    "INSERT INTO archetype (name, tags_json) VALUES (?1, ?2)",
                    params![a.name, a.tags_json],
                )?;
                tx.last_insert_rowid()
            }
        };
        archetype_ids.insert(a.name.clone(), id);
    }
    {
        let mut stmt = tx.prepare(
            "INSERT OR IGNORE INTO speaker_archetype (speaker_id, archetype_id) VALUES (?1, ?2)",
        )?;
        for t in &bundle.speaker_tags {
            if let (Some(&sid), Some(&aid)) = (
                speaker_ids.get(&t.cre_resref),
                archetype_ids.get(&t.archetype_name),
            ) {
                stmt.execute(params![sid, aid])?;
            }
        }
    }

    // shared-group strref -> local id (re-linked from lines below).
    let mut group_ids: HashMap<i64, i64> = HashMap::new();
    {
        let mut stmt = tx.prepare(
            "INSERT INTO shared_strref_group (strref, resolution) VALUES (?1, ?2)",
        )?;
        for g in &bundle.shared_groups {
            stmt.execute(params![g.strref, g.resolution])?;
            group_ids.insert(g.strref, tx.last_insert_rowid());
        }
    }

    let lines = insert_lines(tx, project_id, bundle, &speaker_ids, &group_ids)?;
    let decisions = insert_sample_decisions(tx, bundle, &speaker_ids)?;
    let profile_ids = insert_voice_profiles(tx, project_id, bundle, &speaker_ids)?;
    let clones = insert_clones(tx, bundle, &speaker_ids, &profile_ids)?;
    let _metadata_bindings = insert_metadata_bindings(tx, project_id, bundle, &speaker_ids, &profile_ids)?;

    Ok(TransferImportResult {
        project_id,
        speakers: bundle.speakers.len() as i64,
        lines,
        decisions,
        clones,
        needs_local_rescan: true,
    })
}

fn insert_voice_profiles(
    tx: &rusqlite::Transaction, project_id: i64, bundle: &TransferBundle,
    speaker_ids: &HashMap<String,i64>,
) -> Result<HashMap<String,i64>, AppError> {
    let mut ids = HashMap::new();
    for profile in &bundle.voice_profiles {
        let harvested_speaker_id = profile.harvested_speaker_cre_resref.as_ref().and_then(|key| speaker_ids.get(key)).copied();
        // Audio-free imports are deliberately unavailable. Designed profiles must be
        // auditioned again; imported profiles must be re-supplied locally.
        tx.execute(
            "INSERT INTO voice_profile(project_id,display_name,origin,harvested_speaker_id,design_spec_json,availability,created_at,updated_at) \
             VALUES(?1,?2,?3,?4,?5,'missing_local_audio',?6,?6)",
            params![project_id,profile.display_name,profile.origin,harvested_speaker_id,profile.design_spec_json,chrono::Utc::now().to_rfc3339()],
        )?;
        let profile_id = tx.last_insert_rowid();
        ids.insert(profile.key.clone(), profile_id);
        for reference in &profile.references {
            let sample_id = if let Some(owner_key) = &reference.sample_speaker_cre_resref {
                if let Some(owner) = speaker_ids.get(owner_key) {
                    tx.query_row(
                        "SELECT id FROM reference_sample WHERE speaker_id=?1 AND source_strref IS ?2 AND source_sound_resref IS ?3 ORDER BY id LIMIT 1",
                        params![owner,reference.source_strref,reference.source_sound_resref], |r| r.get::<_,i64>(0),
                    ).optional()?
                } else { None }
            } else { None };
            tx.execute(
                "INSERT INTO voice_profile_reference(voice_profile_id,reference_sample_id,managed_path,transcript,sort_order) \
                 VALUES(?1,?2,NULL,?3,?4)",
                params![profile_id,sample_id,reference.transcript,reference.sort_order],
            )?;
        }
    }
    Ok(ids)
}

fn insert_lines(
    tx: &rusqlite::Transaction,
    project_id: i64,
    bundle: &TransferBundle,
    speaker_ids: &HashMap<String, i64>,
    group_ids: &HashMap<i64, i64>,
) -> Result<i64, AppError> {
    let mut stmt = tx.prepare(
        "INSERT INTO line \
            (project_id, strref, dlg_resref, state_index, text, original_text, flags, \
             existing_sound_resref, kind, is_voiced, has_tokens, token_mask, shared_group_id, \
             speaker_id, attribution_confidence, status) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
    )?;
    let mut count = 0i64;
    for l in &bundle.lines {
        let shared_group_id = l.shared_group_strref.and_then(|s| group_ids.get(&s).copied());
        let speaker_id = l
            .speaker_cre_resref
            .as_ref()
            .and_then(|c| speaker_ids.get(c).copied());
        // No audio was transferred, so a line the source had already EXPORTED can't be
        // exported here until it is regenerated locally - downgrade it to `ready` so it
        // re-enters the local generate/export flow instead of falsely reading as shipped.
        let status = if l.status == "exported" { "ready" } else { l.status.as_str() };
        stmt.execute(params![
            project_id,
            l.strref,
            l.dlg_resref,
            l.state_index,
            l.text,
            l.original_text,
            l.flags,
            l.existing_sound_resref,
            l.kind,
            l.is_voiced as i64,
            l.has_tokens as i64,
            l.token_mask,
            shared_group_id,
            speaker_id,
            l.attribution_confidence,
            status,
        ])?;
        count += 1;
    }
    Ok(count)
}

/// Import reference-sample REVIEW DECISIONS only. `local_derivative_path` is left NULL:
/// the clip itself is game-derived and was never transferred, so the target re-harvests
/// it locally; the preserved decision/provenance/scores let that re-harvest carry the
/// user's prior audition verdict forward.
fn insert_sample_decisions(
    tx: &rusqlite::Transaction,
    bundle: &TransferBundle,
    speaker_ids: &HashMap<String, i64>,
) -> Result<i64, AppError> {
    let mut stmt = tx.prepare(
        "INSERT INTO reference_sample \
            (speaker_id, source_strref, source_sound_resref, provenance_json, scores_json, \
             decision, local_derivative_path) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
    )?;
    let mut count = 0i64;
    for d in &bundle.sample_decisions {
        let Some(&sid) = speaker_ids.get(&d.speaker_cre_resref) else {
            continue;
        };
        stmt.execute(params![
            sid,
            d.source_strref,
            d.source_sound_resref,
            d.provenance_json,
            d.scores_json,
            d.decision,
        ])?;
        count += 1;
    }
    Ok(count)
}

/// Import clone BINDINGS (the tier the user chose). `primary_sample_id` stays NULL and
/// status is forced to `pending`: the referenced local sample doesn't exist here yet, so
/// the target must re-harvest and rebind before generating. The preserved binding tier
/// tells that rebind which precedence the user intended.
fn insert_clones(
    tx: &rusqlite::Transaction,
    bundle: &TransferBundle,
    speaker_ids: &HashMap<String, i64>,
    profile_ids: &HashMap<String, i64>,
) -> Result<i64, AppError> {
    let mut stmt = tx.prepare(
        "INSERT INTO clone (speaker_id, primary_sample_id,voice_profile_id, binding_source, status, \
             render_settings_json) VALUES (?1, NULL, ?2, ?3, 'pending', ?4)",
    )?;
    let mut count = 0i64;
    for c in &bundle.clones {
        let Some(&sid) = speaker_ids.get(&c.speaker_cre_resref) else {
            continue;
        };
        c.render_settings.validate().map_err(AppError::Other)?;
        let render_settings_json = serde_json::to_string(&c.render_settings)?;
        let profile_id = c.voice_profile_key.as_ref().and_then(|key| profile_ids.get(key)).copied();
        stmt.execute(params![sid,profile_id, c.binding_source, render_settings_json])?;
        let clone_id = tx.last_insert_rowid();
        let project_id: i64 = tx.query_row(
            "SELECT project_id FROM speaker WHERE id=?1",
            [sid],
            |row| row.get(0),
        )?;
        let identity_key = crate::db::speaker_groups::identity_key_for_speaker(tx, sid)?;
        let allowed_sample_speakers = crate::db::speaker_groups::speaker_ids_in_group(
            tx,
            project_id,
            &identity_key,
        )?
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
        for reference in &c.references {
            let Some(&sample_speaker_id) =
                speaker_ids.get(&reference.sample_speaker_cre_resref)
            else {
                continue;
            };
            if !allowed_sample_speakers.contains(&sample_speaker_id) {
                continue;
            }
            let sample_id: Option<i64> = tx
                .query_row(
                    "SELECT id FROM reference_sample WHERE speaker_id=?1 \
                     AND source_strref IS ?2 AND source_sound_resref IS ?3 ORDER BY id LIMIT 1",
                    params![
                        sample_speaker_id,
                        reference.source_strref,
                        reference.source_sound_resref
                    ],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(sample_id) = sample_id {
                tx.execute(
                    "INSERT INTO clone_reference(clone_id,sample_id,sort_order) \
                     VALUES(?1,?2,?3)",
                    params![clone_id, sample_id, reference.sort_order],
                )?;
            }
        }
        count += 1;
    }
    Ok(count)
}

fn insert_metadata_bindings(
    tx: &rusqlite::Transaction,
    project_id: i64,
    bundle: &TransferBundle,
    speaker_ids: &HashMap<String, i64>,
    profile_ids: &HashMap<String, i64>,
) -> Result<i64, AppError> {
    use crate::db::metadata_binding::import_binding;
    let mut count = 0i64;
    for b in &bundle.metadata_bindings {
        let donor_ids: Vec<i64> = b
            .donor_cre_resrefs
            .iter()
            .filter_map(|c| speaker_ids.get(c).copied())
            .collect();
        import_binding(
            tx,
            project_id,
            b.sex,
            b.race,
            b.creature_category,
            &donor_ids,
        )?;
        if let Some(binding_id) = crate::db::metadata_binding::binding_id_for_key(
            tx,project_id,b.sex,b.race,b.creature_category,
        )? {
            for key in &b.voice_profile_keys {
                if let Some(profile_id) = profile_ids.get(key) {
                    crate::db::metadata_binding::add_profile(tx,binding_id,*profile_id)?;
                }
            }
        }
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::candidates::CandidateOrigin;
    use crate::audio::scoring::{score, PcmMetrics};
    use crate::db::schema;
    use crate::transfer::export::export_bundle;
    use crate::voices::harvest::{HarvestedSample, SampleProvenance};

    fn mem_db() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        schema::run_migrations(&mut conn).unwrap();
        conn
    }

    // Build a small project with a speaker, a harvested (audio-bearing) sample, a clone,
    // and an already-exported line, then return its id.
    fn seed_project(conn: &Connection, game_root: &str) -> i64 {
        conn.execute(
            "INSERT INTO project (game_root, edition, active_language, generator_version, created_at) \
             VALUES (?1, 'bg2ee', 'en_US', '0.1.0', '2026-01-01T00:00:00Z')",
            params![game_root],
        )
        .unwrap();
        let pid = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO install_fingerprint \
                (project_id, edition_version, language, mod_state_hash, export_version, captured_at) \
             VALUES (?1, '0.1.0', 'en_US', 'deadbeef', '1', 'now')",
            params![pid],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO speaker (project_id, cre_resref, display_name, confidence) \
             VALUES (?1, 'IMOEN', 'Imoen', 1.0)",
            params![pid],
        )
        .unwrap();
        let sid = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO reference_sample \
                (speaker_id, source_strref, source_sound_resref, decision, local_derivative_path) \
             VALUES (?1, 1001, 'IMOEN01', 'approved', 'C:\\game\\workspace\\imoen01.wav')",
            params![sid],
        )
        .unwrap();
        let primary_sample_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO reference_sample \
                (speaker_id, source_strref, source_sound_resref, decision, local_derivative_path) \
             VALUES (?1, 1002, 'IMOEN02', 'approved', 'C:\\game\\workspace\\imoen02.wav')",
            params![sid],
        )
        .unwrap();
        let secondary_sample_id = conn.last_insert_rowid();
        let tuned = crate::models::OmniVoiceRenderSettings {
            speed: Some(1.15),
            num_steps: 48,
            ..Default::default()
        };
        conn.execute(
            "INSERT INTO clone (speaker_id, primary_sample_id, binding_source, status, render_settings_json) \
             VALUES (?1, ?2, 'default', 'ready', ?3)",
            params![sid, primary_sample_id, serde_json::to_string(&tuned).unwrap()],
        )
        .unwrap();
        let clone_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO clone_reference(clone_id,sample_id,sort_order) VALUES(?1,?2,0)",
            params![clone_id, primary_sample_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO clone_reference(clone_id,sample_id,sort_order) VALUES(?1,?2,1)",
            params![clone_id, secondary_sample_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO line (project_id, strref, text, speaker_id, status) \
             VALUES (?1, 42, 'Hello there.', ?2, 'exported')",
            params![pid, sid],
        )
        .unwrap();
        let line_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO generation(line_id,clone_id,status,output_path,render_settings_json, \
                 render_settings_hash) VALUES(?1,?2,'done','C:\\game\\generated\\42.ogg',?3,?4)",
            params![
                line_id,
                clone_id,
                serde_json::to_string(&tuned).unwrap(),
                tuned.fingerprint().unwrap()
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO voice_profile(project_id,display_name,origin,availability,reference_fingerprint,created_at,updated_at) \
             VALUES(?1,'Imported narrator','imported','available','profile-fingerprint','now','now')",
            [pid],
        ).unwrap();
        let imported_profile = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO voice_profile_reference(voice_profile_id,managed_path,transcript,sort_order,fingerprint) \
             VALUES(?1,'C:\\private\\voices\\narrator.wav','The road goes ever on.',0,'reference-fingerprint')",
            [imported_profile],
        ).unwrap();
        pid
    }

    #[test]
    fn bundle_carries_no_local_audio_paths() {
        let conn = mem_db();
        let pid = seed_project(&conn, "C:\\SRC\\BG2EE");
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("bundle.zip");
        export_bundle(&conn, pid, dest.to_str().unwrap(), "0.1.0").unwrap();

        // Read every byte of the archive and assert the game-derived clip path is absent.
        let mut archive = zip::ZipArchive::new(File::open(&dest).unwrap()).unwrap();
        let project_json = read_zip_text(&mut archive, PROJECT_ENTRY).unwrap();
        assert!(
            !project_json.contains("imoen01.wav")
                && !project_json.contains("imoen02.wav")
                && !project_json.contains("42.ogg")
                && !project_json.contains("local_derivative")
                && !project_json.contains("output_path")
                && !project_json.contains("narrator.wav")
                && !project_json.contains("C:\\\\private"),
            "bundle must not carry any game-derived audio path: {project_json}"
        );
        assert!(project_json.contains("\"decision\": \"approved\""));
        assert!(project_json.contains("\"render_settings\""));
        assert!(project_json.contains("\"source_sound_resref\": \"IMOEN02\""));
    }

    #[test]
    fn round_trip_reconstructs_state_without_audio() {
        let mut conn = mem_db();
        let src_pid = seed_project(&conn, "C:\\SRC\\BG2EE");
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("bundle.zip");
        export_bundle(&conn, src_pid, dest.to_str().unwrap(), "0.1.0").unwrap();

        let result = import_bundle(&mut conn, dest.to_str().unwrap(), "D:\\DST\\BG2EE").unwrap();
        assert_eq!(result.speakers, 1);
        assert_eq!(result.lines, 1);
        assert_eq!(result.decisions, 2);
        assert_eq!(result.clones, 1);
        assert!(result.needs_local_rescan);

        // Imported line downgraded from exported -> ready (no local audio yet).
        let status: String = conn
            .query_row(
                "SELECT status FROM line WHERE project_id = ?1",
                params![result.project_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "ready");

        // Imported sample decision preserved but its local path is NULL (re-harvest locally).
        let (decision, path): (String, Option<String>) = conn
            .query_row(
                "SELECT rs.decision, rs.local_derivative_path FROM reference_sample rs \
                 JOIN speaker s ON s.id = rs.speaker_id WHERE s.project_id = ?1",
                params![result.project_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(decision, "approved");
        assert_eq!(path, None);

        // Imported clone forced to pending (must rebind after local re-harvest).
        let clone_status: String = conn
            .query_row(
                "SELECT c.status FROM clone c JOIN speaker s ON s.id = c.speaker_id \
                 WHERE s.project_id = ?1",
                params![result.project_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(clone_status, "pending");
        let settings_json: String = conn
            .query_row(
                "SELECT c.render_settings_json FROM clone c JOIN speaker s ON s.id=c.speaker_id \
                 WHERE s.project_id=?1",
                [result.project_id],
                |r| r.get(0),
            )
            .unwrap();
        let settings: crate::models::OmniVoiceRenderSettings =
            serde_json::from_str(&settings_json).unwrap();
        assert_eq!(settings.speed, Some(1.15));
        assert_eq!(settings.num_steps, 48);
        let references: Vec<(Option<String>, i64)> = {
            let mut stmt = conn
                .prepare(
                    "SELECT rs.source_sound_resref,cr.sort_order FROM clone_reference cr \
                     JOIN clone c ON c.id=cr.clone_id \
                     JOIN speaker s ON s.id=c.speaker_id \
                     JOIN reference_sample rs ON rs.id=cr.sample_id \
                     WHERE s.project_id=?1 ORDER BY cr.sort_order",
                )
                .unwrap();
            stmt.query_map([result.project_id], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        assert_eq!(references, vec![(Some("IMOEN01".into()), 0), (Some("IMOEN02".into()), 1)]);
        let generations: i64 = conn
            .query_row(
                "SELECT count(*) FROM generation g JOIN line l ON l.id=g.line_id \
                 WHERE l.project_id=?1",
                [result.project_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(generations, 0, "generated state/audio must not transfer");
        let imported_profile: (String, String, Option<String>) = conn.query_row(
            "SELECT vp.origin,vp.availability,vpr.managed_path FROM voice_profile vp \
             JOIN voice_profile_reference vpr ON vpr.voice_profile_id=vp.id \
             WHERE vp.project_id=?1 AND vp.display_name='Imported narrator'",
            [result.project_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
        ).unwrap();
        assert_eq!(imported_profile, ("imported".into(),"missing_local_audio".into(),None));

        // A local re-harvest remaps the transferred natural-key membership onto
        // fresh local sample ids and paths; no source-machine path is needed.
        let metrics = PcmMetrics::measure(&vec![0.2f32; 22_050 * 3], 22_050);
        let harvested = [
            (1001u32, "IMOEN01", "D:\\local\\imoen01.wav"),
            (1002u32, "IMOEN02", "D:\\local\\imoen02.wav"),
        ]
        .map(|(strref, sound, path)| HarvestedSample {
            cre_resref: "IMOEN".into(),
            source_strref: strref,
            source_sound_resref: sound.into(),
            provenance: SampleProvenance {
                origin: "dialogue_state".into(),
                cre_resref: "IMOEN".into(),
                source_sound_resref: sound.into(),
                attribution_confidence: 1.0,
                source_text: format!("Locally aligned reference {strref}."),
                eligibility: "automatic".into(),
                shared_source_count: 1,
            },
            score: score(
                CandidateOrigin::DialogueState,
                1.0,
                "Locally aligned reference sentence.",
                &metrics,
            ),
            local_derivative_path: path.into(),
        });
        crate::db::harvest::persist(
            &mut conn,
            result.project_id,
            &harvested,
            false,
            true,
        )
        .unwrap();
        let rebuilt: Vec<(String, String, i64)> = {
            let mut stmt = conn
                .prepare(
                    "SELECT rs.source_sound_resref,rs.local_derivative_path,cr.sort_order \
                     FROM clone_reference cr JOIN clone c ON c.id=cr.clone_id \
                     JOIN speaker s ON s.id=c.speaker_id \
                     JOIN reference_sample rs ON rs.id=cr.sample_id \
                     WHERE s.project_id=?1 ORDER BY cr.sort_order",
                )
                .unwrap();
            stmt.query_map([result.project_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
        };
        assert_eq!(
            rebuilt,
            vec![
                ("IMOEN01".into(), "D:\\local\\imoen01.wav".into(), 0),
                ("IMOEN02".into(), "D:\\local\\imoen02.wav".into(), 1),
            ]
        );
    }

    #[test]
    fn older_bundle_clone_without_settings_uses_current_defaults() {
        let clone: crate::transfer::export::BundleClone = serde_json::from_str(
            r#"{"speaker_cre_resref":"IMOEN","binding_source":"default","status":"ready"}"#,
        )
        .unwrap();
        assert_eq!(
            clone.render_settings,
            crate::models::OmniVoiceRenderSettings::default()
        );
    }

    #[test]
    fn refuses_import_over_existing_project() {
        let mut conn = mem_db();
        let src_pid = seed_project(&conn, "C:\\SRC\\BG2EE");
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("bundle.zip");
        export_bundle(&conn, src_pid, dest.to_str().unwrap(), "0.1.0").unwrap();
        // Import onto the SAME game_root that already has a project -> refuse.
        let err = import_bundle(&mut conn, dest.to_str().unwrap(), "C:\\SRC\\BG2EE").unwrap_err();
        assert!(err.to_string().contains("already exists"), "{err}");
    }

    #[test]
    fn rejects_wrong_kind_archive() {
        let mut conn = mem_db();
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("bad.zip");
        {
            use std::io::Write;
            let mut zip = zip::ZipWriter::new(File::create(&dest).unwrap());
            let opts = zip::write::SimpleFileOptions::default();
            zip.start_file(MANIFEST_ENTRY, opts).unwrap();
            zip.write_all(b"{\"kind\":\"something-else\",\"version\":1,\"created_at\":\"\",\"app_version\":\"\",\"edition\":\"\",\"language\":\"\",\"mod_state_hash\":\"\"}").unwrap();
            zip.finish().unwrap();
        }
        let err = import_bundle(&mut conn, dest.to_str().unwrap(), "D:\\DST\\BG2EE").unwrap_err();
        assert!(err.to_string().contains("expected bundle kind"), "{err}");
    }
}
