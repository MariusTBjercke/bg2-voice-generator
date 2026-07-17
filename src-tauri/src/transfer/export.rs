//! Transfer EXPORT (item-12): serialize a project's transferable state into a ZIP.
//!
//! The bundle is machine-portable CONFIGURATION + generation STATE only. It carries
//! NO audio: `reference_sample.local_derivative_path` and `generation.output_path` are
//! dropped here (they name copyrighted game-derived / regenerable local files). The
//! target re-scans, re-harvests, and regenerates locally on import.

use std::fs::File;
use std::io::Write;

use rusqlite::Connection;

use crate::error::AppError;
use crate::models::OmniVoiceRenderSettings;

use super::{zip_err, TransferManifest, MANIFEST_ENTRY, PROJECT_ENTRY, TRANSFER_KIND, TRANSFER_VERSION};

/// Outcome of a transfer export. Mirror of `TransferExportResult` in
/// `src/lib/types/index.ts`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TransferExportResult {
    pub path: String,
    pub speakers: i64,
    pub lines: i64,
    pub decisions: i64,
}

/// The full transferable payload written as `project.json`. Every child struct mirrors
/// its DB row MINUS the local-audio path columns. `serde(default)` on the deser side
/// (import) keeps older bundles loadable; here we always populate every field.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct TransferBundle {
    pub project: BundleProject,
    #[serde(default)]
    pub fingerprint: Option<BundleFingerprint>,
    #[serde(default)]
    pub speakers: Vec<BundleSpeaker>,
    #[serde(default)]
    pub archetypes: Vec<BundleArchetype>,
    /// (speaker cre_resref, archetype name) tag links - keyed by natural keys, not the
    /// source machine's row ids, so they re-link correctly on import.
    #[serde(default)]
    pub speaker_tags: Vec<BundleSpeakerTag>,
    #[serde(default)]
    pub shared_groups: Vec<BundleSharedGroup>,
    #[serde(default)]
    pub lines: Vec<BundleLine>,
    /// Reference-sample REVIEW DECISIONS + provenance/scores metadata. NO audio path.
    #[serde(default)]
    pub sample_decisions: Vec<BundleSampleDecision>,
    /// Reusable voice metadata only. Managed WAV paths and bytes are never serialized.
    #[serde(default)]
    pub voice_profiles: Vec<BundleVoiceProfile>,
    /// Clone bindings (tier + which sample was primary), keyed by speaker cre_resref.
    #[serde(default)]
    pub clones: Vec<BundleClone>,
    /// Demographic voice pools (sex + race + creature_category -> donor cre_resrefs).
    #[serde(default)]
    pub metadata_bindings: Vec<BundleMetadataBinding>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct BundleProject {
    pub edition: String,
    pub active_language: String,
    pub generator_version: String,
    pub created_at: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct BundleFingerprint {
    pub edition_version: String,
    pub language: String,
    pub mod_state_hash: String,
    pub source_hashes_json: String,
    pub export_version: String,
    pub captured_at: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct BundleSpeaker {
    pub cre_resref: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub long_name_strref: Option<i64>,
    pub sex: i64,
    pub race: i64,
    pub class: i64,
    pub kit: i64,
    pub alignment: i64,
    pub creature_category: i64,
    #[serde(default)]
    pub dialogue_resref: Option<String>,
    pub provenance_json: String,
    pub confidence: f64,
    /// Omitted in older bundles; defaults to not excluded.
    #[serde(default)]
    pub excluded: bool,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct BundleArchetype {
    pub name: String,
    pub tags_json: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct BundleSpeakerTag {
    pub cre_resref: String,
    pub archetype_name: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct BundleSharedGroup {
    pub strref: i64,
    pub resolution: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct BundleLine {
    pub strref: i64,
    #[serde(default)]
    pub dlg_resref: Option<String>,
    #[serde(default)]
    pub state_index: Option<i64>,
    pub text: String,
    #[serde(default)]
    pub original_text: String,
    pub flags: i64,
    #[serde(default)]
    pub existing_sound_resref: Option<String>,
    pub kind: String,
    pub is_voiced: bool,
    pub has_tokens: bool,
    #[serde(default)]
    pub token_mask: i64,
    /// The shared group's `strref` (natural key), re-linked on import; null if none.
    #[serde(default)]
    pub shared_group_strref: Option<i64>,
    /// The owning speaker's `cre_resref` (natural key), re-linked on import; null if none.
    #[serde(default)]
    pub speaker_cre_resref: Option<String>,
    pub attribution_confidence: f64,
    pub status: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct BundleSampleDecision {
    pub speaker_cre_resref: String,
    #[serde(default)]
    pub source_strref: Option<i64>,
    #[serde(default)]
    pub source_sound_resref: Option<String>,
    pub provenance_json: String,
    pub scores_json: String,
    pub decision: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct BundleClone {
    pub speaker_cre_resref: String,
    pub binding_source: String,
    pub status: String,
    /// Transferable configuration only; contains no reference or output paths.
    #[serde(default)]
    pub render_settings: OmniVoiceRenderSettings,
    /// Ordered natural-key metadata only. No sample ids or local derivative paths.
    #[serde(default)]
    pub references: Vec<BundleCloneReference>,
    #[serde(default)]
    pub voice_profile_key: Option<String>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct BundleVoiceProfile {
    pub key: String,
    pub display_name: String,
    pub origin: String,
    #[serde(default)]
    pub harvested_speaker_cre_resref: Option<String>,
    #[serde(default)]
    pub design_spec_json: Option<String>,
    #[serde(default)]
    pub references: Vec<BundleVoiceProfileReference>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct BundleVoiceProfileReference {
    #[serde(default)]
    pub sample_speaker_cre_resref: Option<String>,
    #[serde(default)]
    pub source_strref: Option<i64>,
    #[serde(default)]
    pub source_sound_resref: Option<String>,
    pub transcript: String,
    pub sort_order: i64,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct BundleCloneReference {
    pub sample_speaker_cre_resref: String,
    #[serde(default)]
    pub source_strref: Option<i64>,
    #[serde(default)]
    pub source_sound_resref: Option<String>,
    pub sort_order: i64,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct BundleMetadataBinding {
    pub sex: i64,
    pub race: i64,
    pub creature_category: i64,
    #[serde(default)]
    pub donor_cre_resrefs: Vec<String>,
    #[serde(default)]
    pub voice_profile_keys: Vec<String>,
}

/// Gather the project's transferable state from the DB. PURE-ish (read-only): no audio
/// paths are read, so the returned bundle can never carry a game-derived file location.
pub(crate) fn gather_bundle(
    conn: &Connection,
    project_id: i64,
) -> Result<TransferBundle, AppError> {
    let project = conn.query_row(
        "SELECT edition, active_language, generator_version, created_at \
         FROM project WHERE id = ?1",
        [project_id],
        |r| {
            Ok(BundleProject {
                edition: r.get(0)?,
                active_language: r.get(1)?,
                generator_version: r.get(2)?,
                created_at: r.get(3)?,
            })
        },
    )?;

    // The most recent fingerprint is the source guard a matching target compares against.
    let fingerprint = conn
        .query_row(
            "SELECT edition_version, language, mod_state_hash, source_hashes_json, \
                    export_version, captured_at \
             FROM install_fingerprint WHERE project_id = ?1 ORDER BY id DESC LIMIT 1",
            [project_id],
            |r| {
                Ok(BundleFingerprint {
                    edition_version: r.get(0)?,
                    language: r.get(1)?,
                    mod_state_hash: r.get(2)?,
                    source_hashes_json: r.get(3)?,
                    export_version: r.get(4)?,
                    captured_at: r.get(5)?,
                })
            },
        )
        .ok();

    let speakers = gather_speakers(conn, project_id)?;
    let (archetypes, speaker_tags) = gather_archetypes(conn, project_id)?;
    let shared_groups = gather_shared_groups(conn, project_id)?;
    let lines = gather_lines(conn, project_id)?;
    let sample_decisions = gather_sample_decisions(conn, project_id)?;
    let voice_profiles = gather_voice_profiles(conn, project_id)?;
    let clones = gather_clones(conn, project_id)?;
    let metadata_bindings = gather_metadata_bindings(conn, project_id)?;

    Ok(TransferBundle {
        project,
        fingerprint,
        speakers,
        archetypes,
        speaker_tags,
        shared_groups,
        lines,
        sample_decisions,
        voice_profiles,
        clones,
        metadata_bindings,
    })
}

fn gather_voice_profiles(conn: &Connection, project_id: i64) -> Result<Vec<BundleVoiceProfile>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT vp.id,vp.display_name,vp.origin,s.cre_resref,vp.design_spec_json \
         FROM voice_profile vp LEFT JOIN speaker s ON s.id=vp.harvested_speaker_id \
         WHERE vp.project_id=?1 ORDER BY vp.id",
    )?;
    let rows = stmt.query_map([project_id], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,Option<String>>(4)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out = Vec::new();
    for (id,display_name,origin,harvested_speaker_cre_resref,design_spec_json) in rows {
        let mut refs = conn.prepare(
            "SELECT owner.cre_resref,rs.source_strref,rs.source_sound_resref,vpr.transcript,vpr.sort_order \
             FROM voice_profile_reference vpr \
             LEFT JOIN reference_sample rs ON rs.id=vpr.reference_sample_id \
             LEFT JOIN speaker owner ON owner.id=rs.speaker_id \
             WHERE vpr.voice_profile_id=?1 ORDER BY vpr.sort_order,vpr.id",
        )?;
        let references = refs.query_map([id], |r| Ok(BundleVoiceProfileReference {
            sample_speaker_cre_resref:r.get(0)?,source_strref:r.get(1)?,source_sound_resref:r.get(2)?,
            transcript:r.get(3)?,sort_order:r.get(4)?,
        }))?.collect::<rusqlite::Result<Vec<_>>>()?;
        out.push(BundleVoiceProfile { key:format!("profile-{id}"),display_name,origin,harvested_speaker_cre_resref,design_spec_json,references });
    }
    Ok(out)
}

fn gather_speakers(conn: &Connection, project_id: i64) -> Result<Vec<BundleSpeaker>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT cre_resref, display_name, long_name_strref, sex, race, class, kit, alignment, \
                creature_category, dialogue_resref, provenance_json, confidence, excluded \
         FROM speaker WHERE project_id = ?1 ORDER BY cre_resref",
    )?;
    let rows = stmt
        .query_map([project_id], |r| {
            Ok(BundleSpeaker {
                cre_resref: r.get(0)?,
                display_name: r.get(1)?,
                long_name_strref: r.get(2)?,
                sex: r.get(3)?,
                race: r.get(4)?,
                class: r.get(5)?,
                kit: r.get(6)?,
                alignment: r.get(7)?,
                creature_category: r.get(8)?,
                dialogue_resref: r.get(9)?,
                provenance_json: r.get(10)?,
                confidence: r.get(11)?,
                excluded: r.get::<_, i64>(12)? != 0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// The archetype layer is GLOBAL (not project-scoped), but only archetypes actually
/// linked to one of THIS project's speakers travel with it - so an import doesn't graft
/// the source machine's whole unrelated tag library onto the target.
fn gather_archetypes(
    conn: &Connection,
    project_id: i64,
) -> Result<(Vec<BundleArchetype>, Vec<BundleSpeakerTag>), AppError> {
    let mut tag_stmt = conn.prepare(
        "SELECT s.cre_resref, a.name \
         FROM speaker_archetype sa \
         JOIN speaker s ON s.id = sa.speaker_id \
         JOIN archetype a ON a.id = sa.archetype_id \
         WHERE s.project_id = ?1 ORDER BY s.cre_resref, a.name",
    )?;
    let speaker_tags: Vec<BundleSpeakerTag> = tag_stmt
        .query_map([project_id], |r| {
            Ok(BundleSpeakerTag {
                cre_resref: r.get(0)?,
                archetype_name: r.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(tag_stmt);

    let mut arch_stmt = conn.prepare(
        "SELECT DISTINCT a.name, a.tags_json \
         FROM archetype a \
         JOIN speaker_archetype sa ON sa.archetype_id = a.id \
         JOIN speaker s ON s.id = sa.speaker_id \
         WHERE s.project_id = ?1 ORDER BY a.name",
    )?;
    let archetypes: Vec<BundleArchetype> = arch_stmt
        .query_map([project_id], |r| {
            Ok(BundleArchetype {
                name: r.get(0)?,
                tags_json: r.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok((archetypes, speaker_tags))
}

/// `shared_strref_group` is not project-scoped; travel only the groups referenced by one
/// of this project's lines, keyed by `strref` (their natural key) for re-linking.
fn gather_shared_groups(
    conn: &Connection,
    project_id: i64,
) -> Result<Vec<BundleSharedGroup>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT g.strref, g.resolution \
         FROM shared_strref_group g \
         JOIN line l ON l.shared_group_id = g.id \
         WHERE l.project_id = ?1 ORDER BY g.strref",
    )?;
    let rows = stmt
        .query_map([project_id], |r| {
            Ok(BundleSharedGroup {
                strref: r.get(0)?,
                resolution: r.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn gather_lines(conn: &Connection, project_id: i64) -> Result<Vec<BundleLine>, AppError> {
    // The shared-group and speaker foreign keys are resolved to their NATURAL keys via
    // LEFT JOINs so the bundle never carries the source machine's row ids.
    let mut stmt = conn.prepare(
        "SELECT l.strref, l.dlg_resref, l.state_index, l.text, l.original_text, l.flags, \
                l.existing_sound_resref, l.kind, l.is_voiced, l.has_tokens, l.token_mask, \
                g.strref, s.cre_resref, l.attribution_confidence, l.status \
         FROM line l \
         LEFT JOIN shared_strref_group g ON g.id = l.shared_group_id \
         LEFT JOIN speaker s ON s.id = l.speaker_id \
         WHERE l.project_id = ?1 ORDER BY l.strref, l.id",
    )?;
    let rows = stmt
        .query_map([project_id], |r| {
            Ok(BundleLine {
                strref: r.get(0)?,
                dlg_resref: r.get(1)?,
                state_index: r.get(2)?,
                text: r.get(3)?,
                original_text: r.get(4)?,
                flags: r.get(5)?,
                existing_sound_resref: r.get(6)?,
                kind: r.get(7)?,
                is_voiced: r.get::<_, i64>(8)? != 0,
                has_tokens: r.get::<_, i64>(9)? != 0,
                token_mask: r.get(10)?,
                shared_group_strref: r.get(11)?,
                speaker_cre_resref: r.get(12)?,
                attribution_confidence: r.get(13)?,
                status: r.get(14)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Reference-sample REVIEW DECISIONS + provenance/scores metadata. The
/// `local_derivative_path` column is DELIBERATELY not selected: it names a copyrighted
/// game-derived clip on this machine and must never travel (item-12 copyright rule).
fn gather_sample_decisions(
    conn: &Connection,
    project_id: i64,
) -> Result<Vec<BundleSampleDecision>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT s.cre_resref, rs.source_strref, rs.source_sound_resref, \
                rs.provenance_json, rs.scores_json, rs.decision \
         FROM reference_sample rs \
         JOIN speaker s ON s.id = rs.speaker_id \
         WHERE s.project_id = ?1 ORDER BY s.cre_resref, rs.id",
    )?;
    let rows = stmt
        .query_map([project_id], |r| {
            Ok(BundleSampleDecision {
                speaker_cre_resref: r.get(0)?,
                source_strref: r.get(1)?,
                source_sound_resref: r.get(2)?,
                provenance_json: r.get(3)?,
                scores_json: r.get(4)?,
                decision: r.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Clone BINDINGS (which tier + status), keyed by speaker. `primary_sample_id` is a local
/// row id tied to a local (game-derived) sample, so it is NOT transferred - the target
/// re-harvests and rebinds locally.
fn gather_clones(conn: &Connection, project_id: i64) -> Result<Vec<BundleClone>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT s.cre_resref, c.binding_source, c.status, c.render_settings_json, c.voice_profile_id \
         FROM clone c \
         JOIN speaker s ON s.id = c.speaker_id \
         WHERE s.project_id = ?1 ORDER BY s.cre_resref",
    )?;
    let rows = stmt
        .query_map([project_id], |r| {
            let raw: String = r.get(3)?;
            let render_settings = serde_json::from_str::<OmniVoiceRenderSettings>(&raw)
                .map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        3,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;
            Ok(BundleClone {
                speaker_cre_resref: r.get(0)?,
                binding_source: r.get(1)?,
                status: r.get(2)?,
                render_settings,
                references: Vec::new(),
                voice_profile_key: r.get::<_,Option<i64>>(4)?.map(|id| format!("profile-{id}")),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut rows = rows;
    for clone in &mut rows {
        clone.render_settings.validate().map_err(AppError::Other)?;
        let mut ref_stmt = conn.prepare(
            "SELECT sample_s.cre_resref, rs.source_strref, rs.source_sound_resref, cr.sort_order \
             FROM clone_reference cr \
             JOIN clone c ON c.id=cr.clone_id \
             JOIN speaker owner_s ON owner_s.id=c.speaker_id \
             JOIN reference_sample rs ON rs.id=cr.sample_id \
             JOIN speaker sample_s ON sample_s.id=rs.speaker_id \
             WHERE owner_s.cre_resref=?1 AND owner_s.project_id=?2 \
             ORDER BY cr.sort_order",
        )?;
        clone.references = ref_stmt
            .query_map(rusqlite::params![clone.speaker_cre_resref, project_id], |row| {
                Ok(BundleCloneReference {
                    sample_speaker_cre_resref: row.get(0)?,
                    source_strref: row.get(1)?,
                    source_sound_resref: row.get(2)?,
                    sort_order: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
    }
    Ok(rows)
}

fn gather_metadata_bindings(
    conn: &Connection,
    project_id: i64,
) -> Result<Vec<BundleMetadataBinding>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT mb.sex, mb.race, mb.creature_category, s.cre_resref \
         FROM metadata_binding mb \
         LEFT JOIN metadata_binding_donor mbd ON mbd.binding_id = mb.id \
         LEFT JOIN speaker s ON s.id = mbd.donor_speaker_id \
         WHERE mb.project_id = ?1 \
         ORDER BY mb.sex, mb.race, mb.creature_category, s.cre_resref",
    )?;
    let rows: Vec<(i64, i64, i64, Option<String>)> = stmt
        .query_map([project_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut out: Vec<BundleMetadataBinding> = Vec::new();
    for (sex, race, creature_category, cre) in rows {
        if let Some(last) = out.last_mut() {
            if last.sex == sex && last.race == race && last.creature_category == creature_category {
                if let Some(c) = cre {
                    last.donor_cre_resrefs.push(c);
                }
                continue;
            }
        }
        out.push(BundleMetadataBinding {
            sex,
            race,
            creature_category,
            donor_cre_resrefs: cre.into_iter().collect(),
            voice_profile_keys: Vec::new(),
        });
    }
    for binding in &mut out {
        let mut profiles = conn.prepare(
            "SELECT mbp.voice_profile_id FROM metadata_binding mb \
             JOIN metadata_binding_profile mbp ON mbp.binding_id=mb.id \
             WHERE mb.project_id=?1 AND mb.sex=?2 AND mb.race=?3 AND mb.creature_category=?4 \
             ORDER BY mbp.sort_order,mbp.voice_profile_id",
        )?;
        binding.voice_profile_keys = profiles.query_map(
            rusqlite::params![project_id,binding.sex,binding.race,binding.creature_category],
            |r| r.get::<_,i64>(0),
        )?.map(|row| row.map(|id| format!("profile-{id}"))).collect::<rusqlite::Result<_>>()?;
    }
    Ok(out)
}

/// Write the transfer bundle ZIP to `dest_path`: `manifest.json` (format guard) +
/// `project.json` (the payload). Deflated JSON only - the archive holds no binary audio.
pub(crate) fn export_bundle(
    conn: &Connection,
    project_id: i64,
    dest_path: &str,
    app_version: &str,
) -> Result<TransferExportResult, AppError> {
    let bundle = gather_bundle(conn, project_id)?;

    let manifest = TransferManifest {
        kind: TRANSFER_KIND.into(),
        version: TRANSFER_VERSION,
        created_at: chrono::Utc::now().to_rfc3339(),
        app_version: app_version.into(),
        edition: bundle.project.edition.clone(),
        language: bundle.project.active_language.clone(),
        mod_state_hash: bundle
            .fingerprint
            .as_ref()
            .map(|f| f.mod_state_hash.clone())
            .unwrap_or_default(),
    };

    let result = TransferExportResult {
        path: dest_path.into(),
        speakers: bundle.speakers.len() as i64,
        lines: bundle.lines.len() as i64,
        decisions: bundle.sample_decisions.len() as i64,
    };

    let manifest_json = serde_json::to_string_pretty(&manifest)?;
    let project_json = serde_json::to_string_pretty(&bundle)?;

    let file = File::create(dest_path)?;
    let mut zip = zip::ZipWriter::new(file);
    let deflated = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    zip.start_file(MANIFEST_ENTRY, deflated).map_err(zip_err)?;
    zip.write_all(manifest_json.as_bytes())?;
    zip.start_file(PROJECT_ENTRY, deflated).map_err(zip_err)?;
    zip.write_all(project_json.as_bytes())?;
    zip.finish().map_err(zip_err)?;

    Ok(result)
}
