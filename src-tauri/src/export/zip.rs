//! Self-contained per-project pack ZIP (item-10, artifact B).
//!
//! Item-09 writes the pack FOLDER; this turns it into a single ZIP a user can hand to
//! anyone and install with no toolchain. The layout follows the standard portable
//! WeiDU installer pattern: the bundled WeiDU sits at the ZIP ROOT as `setup-<pack>.exe`,
//! BESIDE the `<pack>/` folder - so extracting the ZIP straight into the game
//! directory puts the setup exe next to chitin.key and the tp2's
//! `COPY ~<pack>/audio~` paths resolve from the game root.
//!
//! ```text
//! <pack>-<edition>.zip
//!   setup-<pack>.exe          (bundled unmodified WeiDU, GPLv2 - see fetch-tools.ps1)
//!   <pack>/
//!     <pack>.tp2, audio/, tra/, backup/, manifest.json, README.txt
//! ```
//!
//! COPYRIGHT GUARD: this only ever archives the item-09 pack folder, whose contents are
//! already the generated-derivative-only set (`build::write_pack` refuses originals) plus
//! WeiDU. No game-derived audio reaches here.

use std::io::Write;
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;

use crate::error::AppError;

/// What `zip_pack` produced: the archive path and the setup exe name it staged (if any).
#[derive(Debug, Clone)]
pub struct ZippedPack {
    pub zip_path: PathBuf,
    /// `Some("setup-<pack>.exe")` when WeiDU was bundled; `None` when no WeiDU was
    /// available (the ZIP is still valid, but the user must supply their own WeiDU).
    pub setup_exe: Option<String>,
}

/// Stage the bundled WeiDU as `setup-<pack>.exe` BESIDE `pack_dir` (when `weidu` is
/// `Some` and exists), then archive both into `<pack_name>-<edition>.zip`: the setup
/// exe at the ZIP root next to the `<pack>/` folder, so extracting the ZIP into the
/// game directory yields the canonical WeiDU layout.
///
/// `pack_dir` is the item-09 pack folder (its file name is the pack name). Returns the
/// written ZIP path + the staged setup exe name. Overwrites a prior ZIP of the same name.
pub fn zip_pack(
    pack_dir: &Path,
    edition: &str,
    weidu: Option<&Path>,
) -> Result<ZippedPack, AppError> {
    let pack_name = pack_dir
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| AppError::Other(format!("pack dir has no name: {}", pack_dir.display())))?
        .to_string();

    let out_dir = pack_dir
        .parent()
        .ok_or_else(|| AppError::Other(format!("pack dir has no parent: {}", pack_dir.display())))?;

    // Stage WeiDU as setup-<pack>.exe NEXT TO the pack folder (never inside it): the
    // game-root layout is `setup-<pack>.exe` + `<pack>/` side by side, and staging it
    // here means the exports dir mirrors exactly what lands in the game dir.
    let setup_exe = match weidu {
        Some(w) if w.exists() => {
            let name = format!("setup-{pack_name}.exe");
            std::fs::copy(w, out_dir.join(&name)).map_err(|e| {
                AppError::Other(format!("failed to stage WeiDU as {name}: {e}"))
            })?;
            Some(name)
        }
        _ => None,
    };

    let zip_path = out_dir.join(format!("{pack_name}-{edition}.zip"));
    if zip_path.exists() {
        std::fs::remove_file(&zip_path)?;
    }

    let file = std::fs::File::create(&zip_path)?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    // The setup exe sits at the ZIP root, beside the <pack>/ folder.
    if let Some(name) = &setup_exe {
        zip.start_file(name, opts)
            .map_err(|e| AppError::Other(format!("zip start_file {name} failed: {e}")))?;
        let bytes = std::fs::read(out_dir.join(name))?;
        zip.write_all(&bytes)?;
    }

    // The pack folder sits under one top-level <pack>/ entry; entry names are
    // forward-slash relative paths (ZIP spec - never backslashes, even on Windows).
    for entry in walkdir::WalkDir::new(pack_dir).sort_by_file_name() {
        let entry = entry.map_err(|e| AppError::Other(format!("walk failed: {e}")))?;
        let rel = entry
            .path()
            .strip_prefix(pack_dir)
            .map_err(|e| AppError::Other(format!("strip_prefix failed: {e}")))?;
        // The pack_dir root itself strips to "" - skip it; its children carry the prefix.
        if rel.as_os_str().is_empty() {
            continue;
        }
        let name = format!("{pack_name}/{}", rel.to_string_lossy().replace('\\', "/"));
        if entry.file_type().is_dir() {
            zip.add_directory(format!("{name}/"), opts)
                .map_err(|e| AppError::Other(format!("zip add_directory {name} failed: {e}")))?;
        } else {
            zip.start_file(&name, opts)
                .map_err(|e| AppError::Other(format!("zip start_file {name} failed: {e}")))?;
            let bytes = std::fs::read(entry.path())?;
            zip.write_all(&bytes)?;
        }
    }
    zip.finish()
        .map_err(|e| AppError::Other(format!("zip finalize failed: {e}")))?;

    Ok(ZippedPack { zip_path, setup_exe })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    /// Build a minimal item-09-shaped pack folder under `root`.
    fn fake_pack(root: &Path, name: &str) -> PathBuf {
        let pack = root.join(name);
        std::fs::create_dir_all(pack.join("audio")).unwrap();
        std::fs::create_dir_all(pack.join("backup")).unwrap();
        std::fs::write(pack.join(format!("{name}.tp2")), b"BACKUP ~x~\n").unwrap();
        std::fs::write(pack.join("audio/Z0H6A00.wav"), b"RIFF....WAVE").unwrap();
        std::fs::write(pack.join("manifest.json"), b"{}").unwrap();
        pack
    }

    fn entry_names(zip_path: &Path) -> Vec<String> {
        let f = std::fs::File::open(zip_path).unwrap();
        let mut a = zip::ZipArchive::new(f).unwrap();
        (0..a.len()).map(|i| a.by_index(i).unwrap().name().to_string()).collect()
    }

    #[test]
    fn zips_pack_under_one_top_level_folder_with_forward_slashes() {
        let dir = tempfile::tempdir().unwrap();
        let pack = fake_pack(dir.path(), "BG2VG");
        let z = zip_pack(&pack, "bg2ee", None).unwrap();
        assert_eq!(z.zip_path.file_name().unwrap(), "BG2VG-bg2ee.zip");
        assert!(z.setup_exe.is_none());
        let names = entry_names(&z.zip_path);
        assert!(names.iter().all(|n| n.starts_with("BG2VG/")), "one top-level folder");
        assert!(names.iter().all(|n| !n.contains('\\')), "forward slashes only");
        assert!(names.contains(&"BG2VG/audio/Z0H6A00.wav".to_string()));
        assert!(names.contains(&"BG2VG/manifest.json".to_string()));
    }

    #[test]
    fn stages_bundled_weidu_at_zip_root_beside_the_pack_folder() {
        let dir = tempfile::tempdir().unwrap();
        let pack = fake_pack(dir.path(), "BG2VG");
        let weidu = dir.path().join("weidu.exe");
        std::fs::write(&weidu, b"MZfake").unwrap();
        let z = zip_pack(&pack, "bg2ee", Some(&weidu)).unwrap();
        assert_eq!(z.setup_exe.as_deref(), Some("setup-BG2VG.exe"));
        // Staged BESIDE the pack folder (the game-root layout), never inside it.
        assert!(dir.path().join("setup-BG2VG.exe").exists());
        assert!(!pack.join("setup-BG2VG.exe").exists());

        let f = std::fs::File::open(&z.zip_path).unwrap();
        let mut a = zip::ZipArchive::new(f).unwrap();
        let mut e = a.by_name("setup-BG2VG.exe").unwrap();
        let mut buf = Vec::new();
        e.read_to_end(&mut buf).unwrap();
        assert_eq!(buf, b"MZfake");
    }

    #[test]
    fn missing_weidu_still_produces_a_valid_zip() {
        let dir = tempfile::tempdir().unwrap();
        let pack = fake_pack(dir.path(), "BG2VG");
        let z = zip_pack(&pack, "bg2ee", Some(&dir.path().join("nope.exe"))).unwrap();
        assert!(z.setup_exe.is_none());
        assert!(!dir.path().join("setup-BG2VG.exe").exists());
        assert!(z.zip_path.exists());
    }
}
