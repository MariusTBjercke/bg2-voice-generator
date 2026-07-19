//! Full-profile backup/restore: ZIP of a profile directory (DB + workspaces +
//! agent-workspace), including local audio. Intended for personal machine moves
//! and demos — not for redistributing copyrighted game-derived audio publicly.
//! WeiDU export packs remain the shareable voice-pack path.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::db::DB_FILE_NAME;
use crate::error::AppError;
use crate::profile::{self, ProfileInfo, ProfileRegistry};

pub const PROFILE_TRANSFER_KIND: &str = "bg2-voice-generator-profile";
pub const PROFILE_TRANSFER_VERSION: i64 = 1;
pub const MANIFEST_ENTRY: &str = "manifest.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileTransferManifest {
    pub kind: String,
    pub version: i64,
    pub created_at: String,
    pub app_version: String,
    pub profile_id: String,
    pub profile_name: String,
    /// Absolute profile directory on the exporting machine (for path rewrite).
    pub exported_profile_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileExportResult {
    pub dest_path: String,
    pub profile_id: String,
    pub profile_name: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileImportResult {
    pub profile: ProfileInfo,
    pub switched: bool,
    pub paths_rewritten: usize,
}

fn zip_err(e: zip::result::ZipError) -> AppError {
    AppError::Other(format!("Zip error: {e}"))
}

fn now_iso() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

/// Checkpoint WAL so the on-disk DB file is complete before zipping/copying.
pub fn checkpoint_db(conn: &Connection) -> Result<(), AppError> {
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    Ok(())
}

/// True for SQLite WAL/SHM sidecars that must not be archived (DB is checkpointed first).
fn is_sqlite_sidecar(name: &str) -> bool {
    name.ends_with("-wal")
        || name.ends_with("-shm")
        || name.ends_with(".db-wal")
        || name.ends_with(".db-shm")
}

/// Count file entries that [`export_profile_dir`] will archive (excludes dirs + sidecars).
pub fn count_export_files(profile_dir: &Path) -> usize {
    walkdir::WalkDir::new(profile_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            if !e.file_type().is_file() {
                return false;
            }
            let path = e.path();
            let Ok(rel) = path.strip_prefix(profile_dir) else {
                return false;
            };
            if rel.as_os_str().is_empty() {
                return false;
            }
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            !is_sqlite_sidecar(name)
        })
        .count()
}

/// Zip the profile directory (db + workspaces + agent-workspace) to `dest_path`.
///
/// When `on_progress` is set it is called once per archived **file** as
/// `(index_1based, file_total, entry_name)`, matching the WeiDU export ZIP ticker.
pub fn export_profile_dir(
    profile_dir: &Path,
    info: &ProfileInfo,
    dest_path: &Path,
    app_version: &str,
    mut on_progress: Option<&mut dyn FnMut(usize, usize, &str)>,
) -> Result<ProfileExportResult, AppError> {
    if !profile_dir.join(DB_FILE_NAME).is_file() {
        return Err(AppError::Other(format!(
            "profile has no database at {}",
            profile_dir.join(DB_FILE_NAME).display()
        )));
    }
    if let Some(parent) = dest_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file_total = count_export_files(profile_dir);
    let file = File::create(dest_path)?;
    let mut zip = ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    let manifest = ProfileTransferManifest {
        kind: PROFILE_TRANSFER_KIND.to_string(),
        version: PROFILE_TRANSFER_VERSION,
        created_at: now_iso(),
        app_version: app_version.to_string(),
        profile_id: info.id.clone(),
        profile_name: info.name.clone(),
        exported_profile_dir: profile_dir.to_string_lossy().to_string(),
    };
    let manifest_json = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| AppError::Other(format!("manifest serialize: {e}")))?;
    zip.start_file(MANIFEST_ENTRY, opts)
        .map_err(zip_err)?;
    zip.write_all(&manifest_json)?;

    add_dir_to_zip(
        &mut zip,
        profile_dir,
        Path::new("profile"),
        opts,
        file_total,
        &mut on_progress,
    )?;
    zip.finish().map_err(zip_err)?;

    let bytes = fs::metadata(dest_path)?.len();
    Ok(ProfileExportResult {
        dest_path: dest_path.to_string_lossy().to_string(),
        profile_id: info.id.clone(),
        profile_name: info.name.clone(),
        bytes,
    })
}

fn add_dir_to_zip(
    zip: &mut ZipWriter<File>,
    src: &Path,
    zip_prefix: &Path,
    opts: SimpleFileOptions,
    file_total: usize,
    on_progress: &mut Option<&mut dyn FnMut(usize, usize, &str)>,
) -> Result<(), AppError> {
    let mut file_index = 0usize;
    for entry in walkdir::WalkDir::new(src).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        let rel = path
            .strip_prefix(src)
            .map_err(|e| AppError::Other(format!("strip prefix: {e}")))?;
        if rel.as_os_str().is_empty() {
            continue;
        }
        // Skip WAL/SHM sidecars — DB should be checkpointed first.
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if is_sqlite_sidecar(name) {
            continue;
        }
        let zip_path = zip_prefix.join(rel);
        let zip_name = zip_path.to_string_lossy().replace('\\', "/");
        if entry.file_type().is_dir() {
            let dir_name = format!("{zip_name}/");
            zip.add_directory(dir_name, opts).map_err(zip_err)?;
        } else if entry.file_type().is_file() {
            zip.start_file(&zip_name, opts).map_err(zip_err)?;
            let mut f = File::open(path)?;
            let mut buf = Vec::new();
            f.read_to_end(&mut buf)?;
            zip.write_all(&buf)?;
            file_index += 1;
            if let Some(progress) = on_progress.as_mut() {
                progress(file_index, file_total, &zip_name);
            }
        }
    }
    Ok(())
}

/// Count extractable `profile/` file entries in a transfer ZIP (excludes dirs + manifest).
pub fn count_import_files(bundle_path: &Path) -> Result<usize, AppError> {
    let file = File::open(bundle_path)?;
    let mut archive = ZipArchive::new(file).map_err(zip_err)?;
    let mut count = 0usize;
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(zip_err)?;
        let name = entry.name();
        if name == MANIFEST_ENTRY || name.ends_with('/') {
            continue;
        }
        if name.strip_prefix("profile/").is_some_and(|rel| !rel.is_empty()) {
            count += 1;
        }
    }
    Ok(count)
}

/// Import a profile ZIP into a new profile id under `app_data`.
///
/// When `on_progress` is set it is called once per extracted **file** as
/// `(index_1based, file_total, entry_name)`.
pub fn import_profile_zip(
    app_data: &Path,
    bundle_path: &Path,
    display_name: Option<String>,
    mut on_progress: Option<&mut dyn FnMut(usize, usize, &str)>,
) -> Result<(ProfileImportResult, ProfileRegistry), AppError> {
    let file = File::open(bundle_path)?;
    let mut archive = ZipArchive::new(file).map_err(zip_err)?;

    let manifest = {
        let mut entry = archive.by_name(MANIFEST_ENTRY).map_err(zip_err)?;
        let mut buf = String::new();
        entry.read_to_string(&mut buf)?;
        let m: ProfileTransferManifest = serde_json::from_str(&buf)
            .map_err(|e| AppError::Other(format!("invalid profile manifest: {e}")))?;
        if m.kind != PROFILE_TRANSFER_KIND {
            return Err(AppError::Other(format!(
                "not a profile bundle (kind={})",
                m.kind
            )));
        }
        if m.version > PROFILE_TRANSFER_VERSION {
            return Err(AppError::Other(format!(
                "profile bundle version {} is newer than this app supports ({})",
                m.version, PROFILE_TRANSFER_VERSION
            )));
        }
        m
    };

    let mut registry = profile::list_profiles(app_data)?;
    let mut max: u64 = 0;
    for p in &registry.profiles {
        if let Ok(n) = p.id.parse::<u64>() {
            max = max.max(n);
        }
    }
    let new_id = (max + 1).to_string();
    let name = display_name
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| manifest.profile_name.clone());

    let dest = profile::profile_dir(app_data, &new_id);
    if dest.exists() {
        return Err(AppError::Other(format!(
            "profile directory already exists: {}",
            dest.display()
        )));
    }
    fs::create_dir_all(&dest)?;

    // Re-open archive for extraction (manifest already consumed).
    drop(archive);
    let file_total = count_import_files(bundle_path)?;
    let file = File::open(bundle_path)?;
    let mut archive = ZipArchive::new(file).map_err(zip_err)?;

    let mut file_index = 0usize;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(zip_err)?;
        let name = entry.name().to_string();
        if name == MANIFEST_ENTRY || name.ends_with('/') {
            continue;
        }
        let Some(rel) = name.strip_prefix("profile/") else {
            continue;
        };
        if rel.is_empty() {
            continue;
        }
        let out = crate::transfer::safe_rel_join(&dest, rel)?;
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut outfile = File::create(&out)?;
        std::io::copy(&mut entry, &mut outfile)?;
        file_index += 1;
        if let Some(progress) = on_progress.as_mut() {
            progress(file_index, file_total, &name);
        }
    }

    let dest_db = dest.join(DB_FILE_NAME);
    if !dest_db.is_file() {
        let _ = fs::remove_dir_all(&dest);
        return Err(AppError::Other(
            "imported bundle did not contain a profile database".into(),
        ));
    }

    let old_root = PathBuf::from(&manifest.exported_profile_dir);
    let paths_rewritten = profile::rewrite_profile_paths(&dest_db, &old_root, &dest)?;

    let info = ProfileInfo {
        id: new_id,
        name,
        created_at: now_iso(),
    };
    registry.profiles.push(info.clone());
    profile::save_registry(app_data, &registry)?;

    Ok((
        ProfileImportResult {
            profile: info,
            switched: false,
            paths_rewritten,
        },
        registry,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use tempfile::tempdir;

    #[test]
    fn export_import_round_trip_includes_workspace_file() {
        let tmp = tempdir().unwrap();
        let app = tmp.path().join("app");
        fs::create_dir_all(&app).unwrap();
        let resolved = profile::ensure_profile_layout(&app).unwrap();
        db::open_db(&resolved.profile_dir).unwrap();
        let ws = resolved.profile_dir.join("workspaces").join("1");
        fs::create_dir_all(&ws).unwrap();
        let sample = ws.join("clip.wav");
        fs::write(&sample, b"RIFF").unwrap();

        let zip_path = tmp.path().join("profile.zip");
        let mut export_ticks = Vec::new();
        export_profile_dir(
            &resolved.profile_dir,
            &resolved.info,
            &zip_path,
            "0.1.0",
            Some(&mut |done, total, name| {
                export_ticks.push((done, total, name.to_string()));
            }),
        )
        .unwrap();
        assert!(!export_ticks.is_empty());
        let (last_done, last_total, _) = export_ticks.last().unwrap();
        assert_eq!(*last_done, *last_total);
        assert_eq!(*last_total, count_export_files(&resolved.profile_dir));

        let mut import_ticks = Vec::new();
        let (result, _) = import_profile_zip(
            &app,
            &zip_path,
            Some("Imported".into()),
            Some(&mut |done, total, name| {
                import_ticks.push((done, total, name.to_string()));
            }),
        )
        .unwrap();
        assert!(!import_ticks.is_empty());
        let (last_done, last_total, _) = import_ticks.last().unwrap();
        assert_eq!(*last_done, *last_total);
        assert_eq!(result.profile.id, "2");
        assert_eq!(result.profile.name, "Imported");
        let imported = profile::profile_dir(&app, "2");
        assert!(imported.join(DB_FILE_NAME).is_file());
        assert!(imported.join("workspaces").join("1").join("clip.wav").is_file());
    }
}
