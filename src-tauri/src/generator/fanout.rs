//! Fan-out: synthesize identical dialogue text once, copy bytes to every other line
//! that shares the same clone + reference within a batch group.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::audio::vorbis::replace_with_temp;
use crate::error::AppError;
use crate::generator::run::{output_path_for, LineJob};

/// One unique text to render, plus every other line id that should receive a copy.
#[derive(Debug, Clone)]
pub struct DedupBundle {
    pub render: LineJob,
    pub fanout_line_ids: Vec<i64>,
}

/// Collapse whitespace so "Hello  there" and "Hello there" dedupe together.
pub fn normalize_line_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Within one reference group, keep one render job per normalized text AND resolved
/// settings fingerprint; every true duplicate line id is listed in `fanout_line_ids`.
pub fn dedup_jobs(jobs: Vec<LineJob>) -> Vec<DedupBundle> {
    dedup_jobs_impl(jobs)
}

fn dedup_jobs_impl(jobs: Vec<LineJob>) -> Vec<DedupBundle> {
    let mut order: Vec<(String, String)> = Vec::new();
    let mut buckets: HashMap<(String, String), DedupBundle> = HashMap::new();
    for job in jobs {
        let key = (
            normalize_line_text(&job.text),
            job.render_settings_fingerprint.clone(),
        );
        if let Some(bundle) = buckets.get_mut(&key) {
            bundle.fanout_line_ids.push(job.line_id);
        } else {
            order.push(key.clone());
            buckets.insert(
                key,
                DedupBundle {
                    render: job,
                    fanout_line_ids: Vec::new(),
                },
            );
        }
    }
    order
        .into_iter()
        .filter_map(|k| buckets.remove(&k))
        .collect()
}

/// Copy a completed compressed clip to another line's stable output path.
pub fn fanout_wav(source: &Path, dest: &Path) -> Result<(), AppError> {
    if !source.exists() {
        return Err(AppError::Other(format!(
            "fan-out source missing: {}",
            source.display()
        )));
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AppError::Other(format!("fan-out mkdir {}: {e}", parent.display())))?;
    }
    let tmp = dest.with_extension("ogg.part");
    std::fs::copy(source, &tmp).map_err(|e| {
        AppError::Other(format!(
            "fan-out copy {} -> {}: {e}",
            source.display(),
            tmp.display()
        ))
    })?;
    replace_with_temp(&tmp, dest)?;
    Ok(())
}

/// Destination paths for every fan-out member of a canonical render.
pub fn fanout_dest_paths(
    workspace: &Path,
    _canonical_line_id: i64,
    canonical_path: &Path,
    fanout_line_ids: &[i64],
) -> Vec<(i64, PathBuf)> {
    fanout_line_ids
        .iter()
        .map(|&line_id| (line_id, output_path_for(workspace, line_id)))
        .filter(|(_, dest)| dest.as_path() != canonical_path)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn job(id: i64, text: &str) -> LineJob {
        LineJob {
            line_id: id,
            clone_id: 1,
            voice_profile_id: None,
            reference_sample_id: 1,
            binding_source: crate::models::BindingSource::Default,
            text: text.into(),
            reference_path: PathBuf::from("/ref.wav"),
            reference_text: String::new(),
            render_settings: crate::models::OmniVoiceRenderSettings::default(),
            render_settings_fingerprint: crate::models::OmniVoiceRenderSettings::default()
                .fingerprint()
                .unwrap(),
            reference_fingerprint: "reference".into(),
            reference_is_composite: false,
        }
    }

    #[test]
    fn dedup_keeps_first_canonical_and_lists_duplicates() {
        let bundles = dedup_jobs(vec![
            job(1, "Hello  there"),
            job(2, "Hello there"),
            job(3, "Goodbye"),
        ]);
        assert_eq!(bundles.len(), 2);
        assert_eq!(bundles[0].render.line_id, 1);
        assert_eq!(bundles[0].fanout_line_ids, vec![2]);
        assert_eq!(bundles[1].render.line_id, 3);
        assert!(bundles[1].fanout_line_ids.is_empty());
    }

    #[test]
    fn normalize_collapses_whitespace() {
        assert_eq!(normalize_line_text("  a   b  "), "a b");
    }

    #[test]
    fn dedup_does_not_fan_out_across_different_settings() {
        let first = job(1, "Same text");
        let mut second = job(2, "Same text");
        second.render_settings.speed = Some(1.15);
        second.render_settings_fingerprint = second.render_settings.fingerprint().unwrap();
        let bundles = dedup_jobs(vec![first, second]);
        assert_eq!(bundles.len(), 2);
        assert!(bundles.iter().all(|bundle| bundle.fanout_line_ids.is_empty()));
    }

    #[test]
    fn fanout_replaces_an_existing_compressed_clip() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.ogg");
        let dest = dir.path().join("dest.ogg");
        std::fs::write(&source, b"new compressed bytes").unwrap();
        std::fs::write(&dest, b"old compressed bytes").unwrap();

        fanout_wav(&source, &dest).unwrap();

        assert_eq!(std::fs::read(dest).unwrap(), b"new compressed bytes");
    }
}
