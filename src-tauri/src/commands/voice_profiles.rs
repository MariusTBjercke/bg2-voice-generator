//! Binding-page voice library commands. All file, DB, ffmpeg, and engine work stays
//! behind this Tauri boundary.

use std::path::{Path, PathBuf};

use rusqlite::{params, OptionalExtension};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::audio::ffmpeg;
use crate::db::voice_profiles::{bind_profile_to_group, profile_by_id, profiles_for_project};
use crate::error::AppError;
use crate::export::manifest::sha256_hex;
use crate::generator::clone::validate_file;
use crate::models::{
    BindingSource, DeleteVoiceProfileResult, DesignVoiceAttributes, DesignedVoiceCandidate,
    DesignedVoiceCandidatesResult, ImportedVoiceClipInput, VoiceProfile,
};
use crate::AppState;

pub fn cleanup_abandoned_design_previews(conn: &rusqlite::Connection, db_path: &Path) {
    let Ok(mut stmt) = conn.prepare("SELECT id FROM project") else {
        return;
    };
    let Ok(ids) = stmt.query_map([], |r| r.get::<_, i64>(0)) else {
        return;
    };
    for id in ids.flatten() {
        let dir = workspace(db_path, id).join("voice-design-previews");
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.parent() == Some(dir.as_path())
                    && path.extension().and_then(|x| x.to_str()) == Some("wav")
                {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
    }
}

fn project_id(conn: &rusqlite::Connection, game_dir: &str) -> Result<i64, AppError> {
    conn.query_row(
        "SELECT id FROM project WHERE game_root=?1",
        [game_dir],
        |r| r.get(0),
    )
    .map_err(|_| AppError::Other("scan this game install before creating voice profiles".into()))
}

fn workspace(db_path: &Path, project_id: i64) -> PathBuf {
    db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("workspaces")
        .join(project_id.to_string())
}

fn voice_root(db_path: &Path, project_id: i64) -> PathBuf {
    workspace(db_path, project_id).join("voice-profiles")
}

fn validate_name(name: &str) -> Result<String, AppError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(AppError::Other(
            "voice profile name must contain 1–80 characters".into(),
        ));
    }
    Ok(name.to_string())
}

fn exact_transcript(text: &str, index: usize) -> Result<String, AppError> {
    if text.trim().is_empty() {
        return Err(AppError::Other(format!(
            "reference {} needs an exact transcript",
            index + 1
        )));
    }
    if text.chars().count() > 1_000 {
        return Err(AppError::Other(format!(
            "reference {} transcript is too long",
            index + 1
        )));
    }
    Ok(text.to_string())
}

fn validate_design(attributes: &DesignVoiceAttributes) -> Result<String, AppError> {
    const GENDER: &[&str] = &["male", "female"];
    const AGE: &[&str] = &["child", "teenager", "young adult", "middle-aged", "elderly"];
    const PITCH: &[&str] = &[
        "very low pitch",
        "low pitch",
        "moderate pitch",
        "high pitch",
        "very high pitch",
    ];
    const ACCENT: &[&str] = &[
        "american accent",
        "british accent",
        "australian accent",
        "canadian accent",
        "indian accent",
        "chinese accent",
        "korean accent",
        "japanese accent",
        "portuguese accent",
        "russian accent",
        "河南话",
        "陕西话",
        "四川话",
        "贵州话",
        "云南话",
        "桂林话",
        "济南话",
        "石家庄话",
        "甘肃话",
        "宁夏话",
        "青岛话",
        "东北话",
    ];
    if !GENDER.contains(&attributes.gender.as_str()) {
        return Err(AppError::Other("unsupported voice-design gender".into()));
    }
    if !AGE.contains(&attributes.age.as_str()) {
        return Err(AppError::Other("unsupported voice-design age".into()));
    }
    if !PITCH.contains(&attributes.pitch.as_str()) {
        return Err(AppError::Other("unsupported voice-design pitch".into()));
    }
    if let Some(accent) = attributes.accent.as_deref() {
        if !ACCENT.contains(&accent) {
            return Err(AppError::Other(
                "unsupported voice-design accent or dialect".into(),
            ));
        }
    }
    let mut parts = vec![
        attributes.gender.clone(),
        attributes.age.clone(),
        attributes.pitch.clone(),
    ];
    if attributes.whisper {
        parts.push("whisper".into());
    }
    if let Some(accent) = &attributes.accent {
        parts.push(accent.clone());
    }
    Ok(parts.join(", "))
}

fn fingerprint_parts(parts: &[(PathBuf, String)]) -> Result<(String, Vec<String>), AppError> {
    let mut all = Vec::new();
    let mut singles = Vec::new();
    for (path, transcript) in parts {
        let bytes = std::fs::read(path)?;
        let mut material = transcript.as_bytes().to_vec();
        material.extend_from_slice(&bytes);
        singles.push(sha256_hex(&material));
        all.extend_from_slice(transcript.as_bytes());
        all.extend_from_slice(&bytes);
    }
    Ok((sha256_hex(&all), singles))
}

#[tauri::command]
pub async fn list_voice_profiles(
    state: State<'_, AppState>,
    game_dir: String,
) -> Result<Vec<VoiceProfile>, AppError> {
    let path = state.db_path();
    tokio::task::spawn_blocking(move || {
        let conn = crate::db::open_read_db(&path)?;
        let Some(project_id) = conn
            .query_row(
                "SELECT id FROM project WHERE game_root=?1",
                [&game_dir],
                |r| r.get(0),
            )
            .optional()?
        else {
            return Ok(Vec::new());
        };
        profiles_for_project(&conn, project_id)
    })
    .await
    .map_err(|e| AppError::Other(format!("voice profile read failed: {e}")))?
}

#[tauri::command]
pub async fn select_voice_reference_files(app: AppHandle) -> Result<Vec<String>, AppError> {
    let selected = app
        .dialog()
        .file()
        .add_filter("Audio", &["wav", "ogg", "mp3", "flac", "m4a", "aac", "wma"])
        .blocking_pick_files()
        .unwrap_or_default();
    selected
        .into_iter()
        .take(4)
        .map(|path| {
            path.into_path()
                .map(|path| path.to_string_lossy().into_owned())
                .map_err(|error| {
                    AppError::Other(format!("selected audio path is invalid: {error}"))
                })
        })
        .collect()
}

/// Decode every source to staging first; only after all clips validate do we create
/// the profile and atomically move its managed files into place.
#[tauri::command]
pub async fn create_imported_voice_profile(
    state: State<'_, AppState>,
    game_dir: String,
    display_name: String,
    clips: Vec<ImportedVoiceClipInput>,
) -> Result<VoiceProfile, AppError> {
    let display_name = validate_name(&display_name)?;
    if clips.is_empty() || clips.len() > 4 {
        return Err(AppError::Other("choose one to four reference clips".into()));
    }
    let transcripts = clips
        .iter()
        .enumerate()
        .map(|(i, c)| exact_transcript(&c.transcript, i))
        .collect::<Result<Vec<_>, _>>()?;
    let mut conn = state.db.lock().await;
    let project_id = project_id(&conn, &game_dir)?;
    let root = voice_root(&state.db_path(), project_id);
    let staging = root.join(format!(
        ".staging-{}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&staging)?;
    let ffmpeg = ffmpeg::resolve_ffmpeg(&state.tools)
        .ok_or_else(|| AppError::Other("ffmpeg is required to import voice references".into()))?;
    let staged = (|| -> Result<Vec<PathBuf>, AppError> {
        let mut out = Vec::new();
        for (index, clip) in clips.iter().enumerate() {
            // Decode from the on-disk path (seekable). Piping bytes breaks MP4/M4A
            // containers that need random access — ffmpeg can exit 0 with an empty WAV.
            let src = Path::new(&clip.path);
            let path = staging.join(format!("reference-{index}.wav"));
            ffmpeg::decode_path_to_derivative(&ffmpeg, src, &path)?;
            validate_file(&path)?;
            out.push(path);
        }
        Ok(out)
    })();
    let staged = match staged {
        Ok(paths) => paths,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error);
        }
    };
    let parts: Vec<_> = staged
        .iter()
        .cloned()
        .zip(transcripts.iter().cloned())
        .collect();
    let (fingerprint, reference_fingerprints) = match fingerprint_parts(&parts) {
        Ok(value) => value,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error);
        }
    };
    let now = chrono::Utc::now().to_rfc3339();
    let tx = match conn.transaction() {
        Ok(tx) => tx,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error.into());
        }
    };
    if let Err(error) = tx.execute(
        "INSERT INTO voice_profile(project_id,display_name,origin,availability,reference_fingerprint,created_at,updated_at) \
         VALUES(?1,?2,'imported','available',?3,?4,?4)",
        params![project_id,display_name,fingerprint,now],
    ) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error.into());
    }
    let profile_id = tx.last_insert_rowid();
    let final_dir = root.join(profile_id.to_string());
    let installed = (|| -> Result<(), AppError> {
        std::fs::create_dir_all(&final_dir)?;
        for (index, staged_path) in staged.iter().enumerate() {
            let final_path = final_dir.join(format!("reference-{index}.wav"));
            std::fs::rename(staged_path, &final_path)?;
            tx.execute(
                "INSERT INTO voice_profile_reference(voice_profile_id,managed_path,transcript,sort_order,fingerprint) \
                 VALUES(?1,?2,?3,?4,?5)",
                params![profile_id,final_path.to_string_lossy().as_ref(),transcripts[index],index as i64,reference_fingerprints[index]],
            )?;
        }
        Ok(())
    })();
    if let Err(error) = installed {
        drop(tx);
        let _ = std::fs::remove_dir_all(&final_dir);
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error);
    }
    if let Err(error) = tx.commit() {
        let _ = std::fs::remove_dir_all(&final_dir);
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error.into());
    }
    let _ = std::fs::remove_dir_all(&staging);
    profile_by_id(&conn, profile_id)?
        .ok_or_else(|| AppError::Other("imported voice profile vanished".into()))
}

#[tauri::command]
pub async fn generate_designed_voice_candidates(
    state: State<'_, AppState>,
    game_dir: String,
    text: String,
    attributes: DesignVoiceAttributes,
) -> Result<DesignedVoiceCandidatesResult, AppError> {
    let text = exact_transcript(&text, 0)?;
    let instruct = validate_design(&attributes)?;
    let (project_id, locale) = {
        let conn = state.db.lock().await;
        let project_id = project_id(&conn, &game_dir)?;
        let locale: String = conn.query_row(
            "SELECT active_language FROM project WHERE id=?1",
            [project_id],
            |r| r.get(0),
        )?;
        (project_id, locale)
    };
    let health = state.omnivoice.ensure_ready().await?;
    if !health.voice_design {
        return Err(AppError::Other(
            "this OmniVoice engine does not advertise voice-design support; update the engine"
                .into(),
        ));
    }
    let preview_dir = workspace(&state.db_path(), project_id).join("voice-design-previews");
    std::fs::create_dir_all(&preview_dir)?;
    // The directory is app-owned and non-durable. Remove only direct children whose
    // extension is WAV; never follow a user-supplied path.
    if let Ok(entries) = std::fs::read_dir(&preview_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.parent() == Some(preview_dir.as_path())
                && path.extension().and_then(|x| x.to_str()) == Some("wav")
            {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    let nonce = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
    let mut candidates = Vec::new();
    for (index, seed) in [42_i64, 137, 911].into_iter().enumerate() {
        let preview_id = format!("{nonce}-{index}-{seed}");
        let output = preview_dir.join(format!("{preview_id}.wav"));
        let response = crate::tts::omnivoice::design_voice_to_file(
            &state.http,
            &state.omnivoice.base_url(),
            &text,
            &instruct,
            &output,
            seed,
        )
        .await?;
        validate_file(&output)?;
        candidates.push(DesignedVoiceCandidate {
            preview_id,
            output_path: output.to_string_lossy().into_owned(),
            seed,
            duration_secs: response.duration,
        });
    }
    let language = locale
        .split(['_', '-'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let quality_warning = (!matches!(language.as_str(), "en" | "zh")).then(||
        "OmniVoice voice design is trained primarily on English and Chinese; this locale may sound less consistent.".to_string()
    );
    Ok(DesignedVoiceCandidatesResult {
        candidates,
        quality_warning,
    })
}

#[tauri::command]
pub async fn save_designed_voice_profile(
    state: State<'_, AppState>,
    game_dir: String,
    display_name: String,
    preview_id: String,
    text: String,
    attributes: DesignVoiceAttributes,
) -> Result<VoiceProfile, AppError> {
    let display_name = validate_name(&display_name)?;
    let text = exact_transcript(&text, 0)?;
    validate_design(&attributes)?;
    if preview_id.is_empty() || !preview_id.chars().all(|c| c.is_ascii_digit() || c == '-') {
        return Err(AppError::Other("invalid designed-voice preview id".into()));
    }
    let mut conn = state.db.lock().await;
    let project_id = project_id(&conn, &game_dir)?;
    let preview_dir = workspace(&state.db_path(), project_id).join("voice-design-previews");
    let preview_path = preview_dir.join(format!("{preview_id}.wav"));
    if preview_path.parent() != Some(preview_dir.as_path()) || !preview_path.exists() {
        return Err(AppError::Other(
            "designed-voice preview is missing or expired".into(),
        ));
    }
    validate_file(&preview_path)?;
    let bytes = std::fs::read(&preview_path)?;
    let mut material = text.as_bytes().to_vec();
    material.extend_from_slice(&bytes);
    let fingerprint = sha256_hex(&material);
    let now = chrono::Utc::now().to_rfc3339();
    let design_json = serde_json::to_string(&attributes)?;
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO voice_profile(project_id,display_name,origin,design_spec_json,availability,reference_fingerprint,created_at,updated_at) \
         VALUES(?1,?2,'designed',?3,'available',?4,?5,?5)",
        params![project_id,display_name,design_json,fingerprint,now],
    )?;
    let profile_id = tx.last_insert_rowid();
    let final_dir = voice_root(&state.db_path(), project_id).join(profile_id.to_string());
    std::fs::create_dir_all(&final_dir)?;
    let final_path = final_dir.join("reference-0.wav");
    if let Err(error) = std::fs::rename(&preview_path, &final_path) {
        let _ = std::fs::remove_dir_all(&final_dir);
        return Err(error.into());
    }
    if let Err(error) = tx.execute(
        "INSERT INTO voice_profile_reference(voice_profile_id,managed_path,transcript,sort_order,fingerprint) VALUES(?1,?2,?3,0,?4)",
        params![profile_id,final_path.to_string_lossy().as_ref(),text,fingerprint],
    ) {
        let _ = std::fs::remove_dir_all(&final_dir);
        return Err(error.into());
    }
    if let Err(error) = tx.commit() {
        let _ = std::fs::remove_dir_all(&final_dir);
        return Err(error.into());
    }
    profile_by_id(&conn, profile_id)?
        .ok_or_else(|| AppError::Other("designed voice profile vanished".into()))
}

#[tauri::command]
pub async fn bind_speaker_voice_profile(
    state: State<'_, AppState>,
    game_dir: String,
    speaker_id: i64,
    voice_profile_id: i64,
) -> Result<VoiceProfile, AppError> {
    let conn = state.db.lock().await;
    let project_id = project_id(&conn, &game_dir)?;
    // Soft-invalidate only: keep prior clips playable and marked voice-changed.
    bind_profile_to_group(
        &conn,
        project_id,
        speaker_id,
        voice_profile_id,
        BindingSource::Override,
    )?;
    crate::generator::metadata_binding::sync_harvested_pool_voice_for_speaker(
        &conn,
        project_id,
        speaker_id,
    )?;
    profile_by_id(&conn, voice_profile_id)?
        .ok_or_else(|| AppError::Other("voice profile vanished after binding".into()))
}

#[tauri::command]
pub async fn rename_voice_profile(
    state: State<'_, AppState>,
    game_dir: String,
    voice_profile_id: i64,
    display_name: String,
) -> Result<VoiceProfile, AppError> {
    let conn = state.db.lock().await;
    let project_id = project_id(&conn, &game_dir)?;
    let name = validate_name(&display_name)?;
    let changed = conn.execute(
        "UPDATE voice_profile SET display_name=?3,updated_at=?4 WHERE id=?1 AND project_id=?2",
        params![
            voice_profile_id,
            project_id,
            name,
            chrono::Utc::now().to_rfc3339()
        ],
    )?;
    if changed == 0 {
        return Err(AppError::Other("voice profile not found".into()));
    }
    Ok(profile_by_id(&conn, voice_profile_id)?.expect("updated profile"))
}

#[tauri::command]
pub async fn delete_voice_profile(
    state: State<'_, AppState>,
    game_dir: String,
    voice_profile_id: i64,
    dry_run: Option<bool>,
) -> Result<DeleteVoiceProfileResult, AppError> {
    let conn = state.db.lock().await;
    let project_id = project_id(&conn, &game_dir)?;
    let _profile = profile_by_id(&conn, voice_profile_id)?
        .filter(|p| p.project_id == project_id)
        .ok_or_else(|| AppError::Other("voice profile not found".into()))?;

    if dry_run.unwrap_or(false) {
        let speaker_count: usize = conn.query_row(
            "SELECT COUNT(DISTINCT speaker_id) FROM clone WHERE voice_profile_id=?1",
            [voice_profile_id],
            |r| r.get::<_, i64>(0),
        )? as usize;
        let affected_pools: usize = conn.query_row(
            "SELECT COUNT(*) FROM metadata_binding_profile WHERE voice_profile_id=?1",
            [voice_profile_id],
            |r| r.get::<_, i64>(0),
        )? as usize;
        let reset_generations: usize = conn.query_row(
            "SELECT COUNT(*) FROM generation g \
             JOIN clone c ON c.id=g.clone_id \
             WHERE c.voice_profile_id=?1 AND g.status='done' AND g.output_path IS NOT NULL",
            [voice_profile_id],
            |r| r.get::<_, i64>(0),
        )? as usize;
        return Ok(DeleteVoiceProfileResult {
            affected_speakers: speaker_count,
            affected_pools,
            reset_generations,
            files_deleted: 0,
        });
    }

    let (mut result, managed_paths) =
        crate::db::voice_profiles::delete_profile(&conn, project_id, voice_profile_id)?;

    // Only remove this profile's managed reference audio — never generation output.
    let root = voice_root(&state.db_path(), project_id);
    for path in managed_paths {
        if path.starts_with(&root) && std::fs::remove_file(&path).is_ok() {
            result.files_deleted += 1;
        }
    }
    let dir = root.join(voice_profile_id.to_string());
    if dir.parent() == Some(root.as_path()) {
        let _ = std::fs::remove_dir_all(dir);
    }
    Ok(result)
}
