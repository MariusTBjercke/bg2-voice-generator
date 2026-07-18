//! Folder-isolated profiles: registry (`profiles.json`) + per-profile data dirs.
//!
//! Layout under the Tauri app-data root:
//! ```text
//! profiles.json
//! profiles/<id>/bg2vg.db
//! profiles/<id>/workspaces/
//! profiles/<id>/agent-workspace/
//! ```
//! Engine runtime (`venv`, `hf-cache`) stays at the app-data root (shared).

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::db::{self, DB_FILE_NAME};
use crate::error::AppError;

pub const REGISTRY_FILE: &str = "profiles.json";
pub const PROFILES_DIR: &str = "profiles";
pub const DEFAULT_PROFILE_ID: &str = "1";
pub const DEFAULT_PROFILE_NAME: &str = "Default";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileInfo {
    pub id: String,
    pub name: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileRegistry {
    pub active_id: String,
    pub profiles: Vec<ProfileInfo>,
}

impl ProfileRegistry {
    pub fn active(&self) -> Result<&ProfileInfo, AppError> {
        self.profiles
            .iter()
            .find(|p| p.id == self.active_id)
            .ok_or_else(|| {
                AppError::Other(format!(
                    "active profile id '{}' is missing from the registry",
                    self.active_id
                ))
            })
    }
}

/// Result of resolving which profile directory to open at startup.
#[derive(Debug, Clone)]
pub struct ResolvedProfile {
    pub registry: ProfileRegistry,
    pub info: ProfileInfo,
    pub profile_dir: PathBuf,
}

fn now_iso() -> String {
    // RFC3339-ish UTC without pulling chrono; good enough for display/sort.
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

pub fn registry_path(app_data: &Path) -> PathBuf {
    app_data.join(REGISTRY_FILE)
}

pub fn profiles_root(app_data: &Path) -> PathBuf {
    app_data.join(PROFILES_DIR)
}

pub fn profile_dir(app_data: &Path, id: &str) -> PathBuf {
    profiles_root(app_data).join(id)
}

pub fn load_registry(app_data: &Path) -> Result<Option<ProfileRegistry>, AppError> {
    let path = registry_path(app_data);
    if !path.is_file() {
        return Ok(None);
    }
    let mut f = fs::File::open(&path)?;
    let mut buf = String::new();
    f.read_to_string(&mut buf)?;
    // PowerShell Set-Content -Encoding utf8 writes a UTF-8 BOM; strip it.
    let trimmed = buf.trim_start_matches('\u{feff}').trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let reg: ProfileRegistry = serde_json::from_str(trimmed)
        .map_err(|e| AppError::Other(format!("invalid profiles.json: {e}")))?;
    Ok(Some(reg))
}

pub fn save_registry(app_data: &Path, registry: &ProfileRegistry) -> Result<(), AppError> {
    let path = registry_path(app_data);
    let json = serde_json::to_string_pretty(registry)
        .map_err(|e| AppError::Other(format!("serialize profiles.json: {e}")))?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        // Write UTF-8 without BOM so loaders (and hand edits) stay interoperable.
        f.write_all(json.as_bytes())?;
        f.write_all(b"\n")?;
        f.sync_all()?;
    }
    fs::rename(&tmp, &path)?;
    Ok(())
}

/// Refuse to start if a legacy flat-layout DB is still at the app-data root.
pub fn guard_legacy_root_db(app_data: &Path) -> Result<(), AppError> {
    let legacy = app_data.join(DB_FILE_NAME);
    if legacy.is_file() {
        return Err(AppError::Other(format!(
            "legacy database found at {} — move it into profiles/<id>/{} \
             (expected layout: profiles.json + profiles/<id>/). \
             A one-shot migrate script may be used if you still have pre-profile AppData.",
            legacy.display(),
            DB_FILE_NAME
        )));
    }
    Ok(())
}

fn default_name_for_id(id: &str) -> String {
    if id == DEFAULT_PROFILE_ID {
        DEFAULT_PROFILE_NAME.to_string()
    } else {
        format!("Profile {id}")
    }
}

fn next_profile_id(registry: &ProfileRegistry) -> String {
    let mut max: u64 = 0;
    for p in &registry.profiles {
        if let Ok(n) = p.id.parse::<u64>() {
            max = max.max(n);
        }
    }
    (max + 1).to_string()
}

/// Ensure a usable profile layout exists; create Default on a fresh install.
pub fn ensure_profile_layout(app_data: &Path) -> Result<ResolvedProfile, AppError> {
    guard_legacy_root_db(app_data)?;
    fs::create_dir_all(profiles_root(app_data))?;

    let registry = match load_registry(app_data)? {
        Some(reg) if !reg.profiles.is_empty() => reg,
        Some(_) | None => {
            let info = ProfileInfo {
                id: DEFAULT_PROFILE_ID.to_string(),
                name: DEFAULT_PROFILE_NAME.to_string(),
                created_at: now_iso(),
            };
            let dir = profile_dir(app_data, &info.id);
            fs::create_dir_all(&dir)?;
            let reg = ProfileRegistry {
                active_id: info.id.clone(),
                profiles: vec![info],
            };
            save_registry(app_data, &reg)?;
            reg
        }
    };

    let info = registry.active()?.clone();
    let dir = profile_dir(app_data, &info.id);
    fs::create_dir_all(&dir)?;
    Ok(ResolvedProfile {
        registry,
        info,
        profile_dir: dir,
    })
}

pub fn list_profiles(app_data: &Path) -> Result<ProfileRegistry, AppError> {
    ensure_profile_layout(app_data).map(|r| r.registry)
}

pub fn create_profile(app_data: &Path, name: Option<String>) -> Result<ProfileInfo, AppError> {
    let mut registry = list_profiles(app_data)?;
    let id = next_profile_id(&registry);
    let display = name
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default_name_for_id(&id));
    let info = ProfileInfo {
        id: id.clone(),
        name: display,
        created_at: now_iso(),
    };
    let dir = profile_dir(app_data, &id);
    if dir.exists() {
        return Err(AppError::Other(format!(
            "profile directory already exists: {}",
            dir.display()
        )));
    }
    fs::create_dir_all(&dir)?;
    // Seed an empty DB with migrations + default dictionary/tag rules.
    let conn = db::open_db(&dir)?;
    drop(conn);
    registry.profiles.push(info.clone());
    save_registry(app_data, &registry)?;
    Ok(info)
}

pub fn rename_profile(app_data: &Path, id: &str, name: &str) -> Result<ProfileInfo, AppError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::Other("profile name cannot be empty".into()));
    }
    let mut registry = list_profiles(app_data)?;
    let info = registry
        .profiles
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or_else(|| AppError::Other(format!("unknown profile id '{id}'")))?;
    info.name = name.to_string();
    let out = info.clone();
    save_registry(app_data, &registry)?;
    Ok(out)
}

pub fn set_active_id(app_data: &Path, id: &str) -> Result<ProfileInfo, AppError> {
    let mut registry = list_profiles(app_data)?;
    let info = registry
        .profiles
        .iter()
        .find(|p| p.id == id)
        .cloned()
        .ok_or_else(|| AppError::Other(format!("unknown profile id '{id}'")))?;
    let dir = profile_dir(app_data, id);
    if !dir.join(DB_FILE_NAME).is_file() {
        return Err(AppError::Other(format!(
            "profile '{id}' has no database at {}",
            dir.join(DB_FILE_NAME).display()
        )));
    }
    registry.active_id = id.to_string();
    save_registry(app_data, &registry)?;
    Ok(info)
}

pub fn delete_profile(app_data: &Path, id: &str, active_id: &str) -> Result<ProfileRegistry, AppError> {
    let mut registry = list_profiles(app_data)?;
    if registry.profiles.len() <= 1 {
        return Err(AppError::Other(
            "cannot delete the last profile".into(),
        ));
    }
    if id == active_id {
        return Err(AppError::Other(
            "cannot delete the active profile; switch to another profile first".into(),
        ));
    }
    if !registry.profiles.iter().any(|p| p.id == id) {
        return Err(AppError::Other(format!("unknown profile id '{id}'")));
    }
    registry.profiles.retain(|p| p.id != id);
    save_registry(app_data, &registry)?;
    let dir = profile_dir(app_data, id);
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    Ok(registry)
}

/// Deep-copy a profile folder tree into a new id (demo sandbox).
pub fn duplicate_profile(
    app_data: &Path,
    source_id: &str,
    name: Option<String>,
) -> Result<ProfileInfo, AppError> {
    let mut registry = list_profiles(app_data)?;
    if !registry.profiles.iter().any(|p| p.id == source_id) {
        return Err(AppError::Other(format!("unknown profile id '{source_id}'")));
    }
    let src = profile_dir(app_data, source_id);
    if !src.is_dir() {
        return Err(AppError::Other(format!(
            "source profile directory missing: {}",
            src.display()
        )));
    }
    let id = next_profile_id(&registry);
    let display = name
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            let src_name = registry
                .profiles
                .iter()
                .find(|p| p.id == source_id)
                .map(|p| p.name.as_str())
                .unwrap_or("Profile");
            format!("Copy of {src_name}")
        });
    let dest = profile_dir(app_data, &id);
    if dest.exists() {
        return Err(AppError::Other(format!(
            "profile directory already exists: {}",
            dest.display()
        )));
    }
    copy_dir_recursive(&src, &dest)?;
    let dest_db = dest.join(DB_FILE_NAME);
    if dest_db.is_file() {
        rewrite_profile_paths(&dest_db, &src, &dest)?;
    }
    let info = ProfileInfo {
        id,
        name: display,
        created_at: now_iso(),
    };
    registry.profiles.push(info.clone());
    save_registry(app_data, &registry)?;
    Ok(info)
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<(), AppError> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let to = dest.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &to)?;
        } else if ty.is_file() {
            fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

/// Per-project derivative workspace: `<profile_dir>/workspaces/<project_id>`.
pub fn workspace_dir(profile_dir: &Path, project_id: i64) -> PathBuf {
    profile_dir
        .join("workspaces")
        .join(project_id.to_string())
}

/// Agent docs workspace: `<profile_dir>/agent-workspace/<project_id>`.
pub fn agent_workspace_dir(profile_dir: &Path, project_id: i64) -> PathBuf {
    profile_dir
        .join("agent-workspace")
        .join(project_id.to_string())
}

/// Profile data root from a DB file path (`.../profiles/<id>/bg2vg.db` → `.../profiles/<id>`).
pub fn profile_dir_from_db(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

/// After copying a profile folder, rewrite absolute paths stored in SQLite so they
/// point at `new_root` instead of `old_root` (duplicate / import / layout move).
///
/// Only rewrites values that **start with** `old_root` and do **not** already start
/// with `new_root` (so `old` being a prefix of `new` cannot double-nest).
pub fn rewrite_profile_paths(
    db_path: &Path,
    old_root: &Path,
    new_root: &Path,
) -> Result<usize, AppError> {
    if old_root == new_root {
        return Ok(0);
    }
    let old = old_root.to_string_lossy().to_string();
    let new = new_root.to_string_lossy().to_string();
    let old_fwd = old.replace('\\', "/");
    let new_fwd = new.replace('\\', "/");

    let mut conn = Connection::open(db_path)?;
    db::tune_connection(&conn)?;
    let tx = conn.transaction()?;
    let mut total = 0usize;
    for (table, col) in [
        ("reference_sample", "local_derivative_path"),
        ("generation", "output_path"),
        ("voice_profile_reference", "managed_path"),
        ("render_candidate", "output_path"),
        ("export", "weidu_pack_path"),
    ] {
        // Prefix-only rewrite; skip rows already under new_root.
        let sql = format!(
            "UPDATE {table} SET {col} = ?2 || substr({col}, length(?1) + 1) \
             WHERE {col} IS NOT NULL \
               AND instr({col}, ?1) = 1 \
               AND instr({col}, ?2) != 1"
        );
        total += tx.execute(&sql, params![old.as_str(), new.as_str()])? as usize;
        if old_fwd != old {
            // Forward-slash stored paths: same prefix-only rule.
            let sql_fwd = format!(
                "UPDATE {table} SET {col} = ?2 || substr({col}, length(?1) + 1) \
                 WHERE {col} IS NOT NULL \
                   AND instr({col}, ?1) = 1 \
                   AND instr({col}, ?2) != 1"
            );
            total += tx.execute(&sql_fwd, params![old_fwd.as_str(), new_fwd.as_str()])? as usize;
        }
    }
    tx.commit()?;
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn rewrite_paths_does_not_double_nest_when_old_is_prefix_of_new() {
        let tmp = tempdir().unwrap();
        let old = tmp.path().join("app");
        let nested = old.join("profiles").join("1");
        fs::create_dir_all(&nested).unwrap();
        db::open_db(&nested).unwrap();
        let db_path = nested.join(DB_FILE_NAME);
        let old_path = old.join("workspaces").join("1").join("a.wav");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "INSERT INTO project (game_root, edition, active_language, generator_version, created_at)
             VALUES ('x','BG2EE','en_US','0.1.0','t')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO speaker (project_id, cre_resref, display_name) VALUES (1,'a','A')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO reference_sample (speaker_id, decision, local_derivative_path, provenance_json)
             VALUES (1,'approved',?1,'{}')",
            [old_path.to_string_lossy().as_ref()],
        )
        .unwrap();
        drop(conn);

        let n = rewrite_profile_paths(&db_path, &old, &nested).unwrap();
        assert!(n >= 1);
        let n2 = rewrite_profile_paths(&db_path, &old, &nested).unwrap();
        assert_eq!(n2, 0, "second rewrite must be a no-op");

        let conn = Connection::open(&db_path).unwrap();
        let path: String = conn
            .query_row(
                "SELECT local_derivative_path FROM reference_sample WHERE id=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            path.starts_with(nested.to_string_lossy().as_ref()),
            "{path}"
        );
        assert!(
            !path.contains(r"profiles\1\profiles\1") && !path.contains("profiles/1/profiles/1"),
            "{path}"
        );
    }

    #[test]
    fn load_registry_strips_utf8_bom() {
        let tmp = tempdir().unwrap();
        let path = registry_path(tmp.path());
        let body = "{\n  \"active_id\": \"1\",\n  \"profiles\": [{\"id\":\"1\",\"name\":\"Default\",\"created_at\":\"0\"}]\n}\n";
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend(body.as_bytes());
        fs::write(&path, bytes).unwrap();
        let reg = load_registry(tmp.path()).unwrap().unwrap();
        assert_eq!(reg.active_id, "1");
        assert_eq!(reg.profiles[0].name, "Default");
    }

    #[test]
    fn fresh_install_creates_default_profile() {
        let tmp = tempdir().unwrap();
        let resolved = ensure_profile_layout(tmp.path()).unwrap();
        assert_eq!(resolved.info.id, "1");
        assert_eq!(resolved.info.name, "Default");
        assert!(registry_path(tmp.path()).is_file());
        assert!(resolved.profile_dir.is_dir());
    }

    #[test]
    fn legacy_root_db_is_rejected() {
        let tmp = tempdir().unwrap();
        fs::write(tmp.path().join(DB_FILE_NAME), b"x").unwrap();
        let err = ensure_profile_layout(tmp.path()).unwrap_err();
        assert!(err.to_string().contains("legacy database"), "{err}");
    }

    #[test]
    fn create_rename_duplicate_delete() {
        let tmp = tempdir().unwrap();
        ensure_profile_layout(tmp.path()).unwrap();
        // Need a real DB for default so duplicate works
        db::open_db(&profile_dir(tmp.path(), "1")).unwrap();

        let p2 = create_profile(tmp.path(), Some("Demo".into())).unwrap();
        assert_eq!(p2.id, "2");
        assert_eq!(p2.name, "Demo");
        assert!(profile_dir(tmp.path(), "2").join(DB_FILE_NAME).is_file());

        let renamed = rename_profile(tmp.path(), "2", "Showcase").unwrap();
        assert_eq!(renamed.name, "Showcase");

        let dup = duplicate_profile(tmp.path(), "1", None).unwrap();
        assert_eq!(dup.id, "3");
        assert!(dup.name.contains("Copy"));
        assert!(profile_dir(tmp.path(), "3").join(DB_FILE_NAME).is_file());

        set_active_id(tmp.path(), "2").unwrap();
        let reg = delete_profile(tmp.path(), "1", "2").unwrap();
        assert!(!reg.profiles.iter().any(|p| p.id == "1"));
        assert!(!profile_dir(tmp.path(), "1").exists());
    }

    #[test]
    fn cannot_delete_last_or_active() {
        let tmp = tempdir().unwrap();
        ensure_profile_layout(tmp.path()).unwrap();
        db::open_db(&profile_dir(tmp.path(), "1")).unwrap();
        let err = delete_profile(tmp.path(), "1", "1").unwrap_err();
        assert!(err.to_string().contains("last profile") || err.to_string().contains("active"));

        create_profile(tmp.path(), None).unwrap();
        let err = delete_profile(tmp.path(), "1", "1").unwrap_err();
        assert!(err.to_string().contains("active"));
    }
}
