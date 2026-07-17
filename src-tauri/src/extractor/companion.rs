//! Companion banter/interjection dialogue from `interdia.2da`, plus party
//! post/join dialogue from `pdialog.2da`.
//!
//! Party NPCs store banter and interjection lines in DLGs listed by `interdia.2da`,
//! not in the CRE's `dialog_resref`. This module loads those DLGs and attributes
//! their actor states to the companion CRE named by each row's death variable.
//! `pdialog.2da` supplies the post-party (`*P`) and joined (`*J`) files the engine
//! uses when reforming the party or talking to a companion already in the party.
//! A follow-on pass also picks up orphan side-chain DLGs (e.g. `jaheiraj.dlg`) whose
//! resref shares the companion prefix but is not the CRE's main dialogue or an
//! `interdia` / `pdialog` file.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::error::AppError;

use super::attribution::{self, AttributedLine, AttributedSpeaker, StrrefFacts, CONFIDENCE_UNIQUE};
use super::cre::Cre;
use super::dlg::Dlg;
use super::resource::GameResources;
use super::restype::{TYPE_2DA, TYPE_CRE, TYPE_DLG};
use super::tlk::Tlk;
use super::token_resolve::TokenReplacements;
use super::VoicedSource;

/// Stats from scanning companion `interdia.2da` DLGs and side-chain DLGs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompanionScanStats {
    pub lines_added: usize,
    pub dlgs_scanned: usize,
    pub rows_unmapped: usize,
    pub side_dlgs_scanned: usize,
    pub side_lines_added: usize,
}

/// Every banter/interjection DLG resref listed in `interdia.2da` (lowercased).
pub fn interdia_banter_dlg_resrefs(res: &GameResources) -> Result<HashSet<String>, AppError> {
    let mut out = HashSet::new();
    let bytes = match res.read("interdia", TYPE_2DA) {
        Ok(b) => b,
        Err(_) => return Ok(out),
    };
    let table = super::twoda::parse_2da(&bytes)?;
    let file_col = table.column_index("FILE");
    let tob_col = table.column_index("25FILE");
    for row in &table.rows {
        for col in [file_col, tob_col].into_iter().flatten() {
            if let Some(dlg) = row.values.get(col) {
                if is_usable_dlg_resref(dlg) {
                    out.insert(dlg.trim().to_ascii_lowercase());
                }
            }
        }
    }
    Ok(out)
}

/// Post/join DLG resrefs from `pdialog.2da` (SoA + ToB columns, lowercased).
pub fn pdialog_dlg_resrefs(res: &GameResources) -> Result<HashSet<String>, AppError> {
    let mut out = HashSet::new();
    let bytes = match res.read("pdialog", TYPE_2DA) {
        Ok(b) => b,
        Err(_) => return Ok(out),
    };
    let table = super::twoda::parse_2da(&bytes)?;
    for col_name in [
        "POST_DIALOG_FILE",
        "JOIN_DIALOG_FILE",
        "25POST_DIALOG_FILE",
        "25JOIN_DIALOG_FILE",
    ] {
        let Some(col) = table.column_index(col_name) else {
            continue;
        };
        for row in &table.rows {
            if let Some(dlg) = row.values.get(col) {
                if is_usable_dlg_resref(dlg) {
                    out.insert(dlg.trim().to_ascii_lowercase());
                }
            }
        }
    }
    Ok(out)
}

/// Companion CRE/DLG search prefixes derived from `interdia.2da` and `pdialog.2da`.
pub fn interdia_companion_prefixes(res: &GameResources) -> Result<HashSet<String>, AppError> {
    let mut out = HashSet::new();
    for table_name in ["interdia", "pdialog"] {
        let bytes = match res.read(table_name, TYPE_2DA) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let table = super::twoda::parse_2da(&bytes)?;
        for row in &table.rows {
            let death_var = row.label.trim().to_ascii_lowercase();
            if death_var.is_empty() {
                continue;
            }
            for prefix in companion_dlg_search_prefixes(&death_var) {
                out.insert(prefix.to_string());
            }
        }
    }
    Ok(out)
}

/// Scan `interdia.2da` and emit attributed lines not already present in `existing`.
pub fn scan_interdia(
    res: &GameResources,
    tlk: &Tlk,
    token_reps: &TokenReplacements,
    existing: &HashSet<(u32, String, u32)>,
) -> Result<
    (
        Vec<AttributedLine>,
        Vec<AttributedSpeaker>,
        CompanionScanStats,
    ),
    AppError,
> {
    let bytes = match res.read("interdia", TYPE_2DA) {
        Ok(b) => b,
        Err(_) => return Ok((Vec::new(), Vec::new(), CompanionScanStats::default())),
    };
    let table = super::twoda::parse_2da(&bytes)?;
    let file_col = table.column_index("FILE");
    let tob_col = table.column_index("25FILE");

    let mut lines = Vec::new();
    let mut extra_speakers = Vec::new();
    let mut speaker_seen: BTreeSet<String> = BTreeSet::new();
    let mut stats = CompanionScanStats::default();
    let mut dlgs_seen: BTreeSet<String> = BTreeSet::new();

    for row in &table.rows {
        let death_var = row.label.trim().to_ascii_lowercase();
        let Some(identity) = resolve_death_var_cre(res, &death_var) else {
            stats.rows_unmapped += 1;
            continue;
        };

        let mut dlg_jobs: Vec<(&str, &str)> = Vec::new();
        if let Some(col) = file_col {
            if let Some(dlg) = row.values.get(col).map(|s| s.as_str()) {
                if is_usable_dlg_resref(dlg) {
                    dlg_jobs.push((dlg, "soa"));
                }
            }
        }
        if let Some(col) = tob_col {
            if let Some(dlg) = row.values.get(col).map(|s| s.as_str()) {
                if is_usable_dlg_resref(dlg) {
                    dlg_jobs.push((dlg, "tob"));
                }
            }
        }

        if dlg_jobs.is_empty() {
            continue;
        }

        if speaker_seen.insert(identity.cre_resref.clone()) {
            extra_speakers.push(companion_speaker(&identity));
        }

        for (dlg_resref, campaign) in dlg_jobs {
            let dlg_lc = dlg_resref.to_ascii_lowercase();
            if !dlgs_seen.insert(dlg_lc.clone()) {
                continue;
            }
            let emitted = emit_companion_dlg_states(
                res,
                tlk,
                token_reps,
                existing,
                &identity,
                &dlg_lc,
                &death_var,
                "companion_interdia",
                Some(campaign),
                None,
                &mut lines,
            )?;
            stats.dlgs_scanned += emitted.dlgs_scanned;
            stats.lines_added += emitted.lines_added;
        }
    }

    Ok((lines, extra_speakers, stats))
}

/// Scan `pdialog.2da` post/join DLGs (SoA + ToB) not already present in `existing`.
pub fn scan_pdialog(
    res: &GameResources,
    tlk: &Tlk,
    token_reps: &TokenReplacements,
    existing: &HashSet<(u32, String, u32)>,
) -> Result<
    (
        Vec<AttributedLine>,
        Vec<AttributedSpeaker>,
        CompanionScanStats,
    ),
    AppError,
> {
    let bytes = match res.read("pdialog", TYPE_2DA) {
        Ok(b) => b,
        Err(_) => return Ok((Vec::new(), Vec::new(), CompanionScanStats::default())),
    };
    let table = super::twoda::parse_2da(&bytes)?;
    let jobs: [(&str, &str, &str); 4] = [
        ("POST_DIALOG_FILE", "soa", "post"),
        ("JOIN_DIALOG_FILE", "soa", "join"),
        ("25POST_DIALOG_FILE", "tob", "post"),
        ("25JOIN_DIALOG_FILE", "tob", "join"),
    ];

    let mut lines = Vec::new();
    let mut extra_speakers = Vec::new();
    let mut speaker_seen: BTreeSet<String> = BTreeSet::new();
    let mut stats = CompanionScanStats::default();
    let mut dlgs_seen: BTreeSet<String> = BTreeSet::new();

    for row in &table.rows {
        let death_var = row.label.trim().to_ascii_lowercase();
        if death_var.is_empty() {
            continue;
        }
        let Some(identity) = resolve_death_var_cre(res, &death_var) else {
            stats.rows_unmapped += 1;
            continue;
        };

        let mut dlg_jobs: Vec<(String, &str, &str)> = Vec::new();
        for (col_name, campaign, role) in jobs {
            let Some(col) = table.column_index(col_name) else {
                continue;
            };
            if let Some(dlg) = row.values.get(col).map(|s| s.as_str()) {
                if is_usable_dlg_resref(dlg) {
                    dlg_jobs.push((dlg.to_ascii_lowercase(), campaign, role));
                }
            }
        }
        if dlg_jobs.is_empty() {
            continue;
        }

        if speaker_seen.insert(identity.cre_resref.clone()) {
            extra_speakers.push(companion_speaker(&identity));
        }

        for (dlg_lc, campaign, role) in dlg_jobs {
            if !dlgs_seen.insert(dlg_lc.clone()) {
                continue;
            }
            let emitted = emit_companion_dlg_states(
                res,
                tlk,
                token_reps,
                existing,
                &identity,
                &dlg_lc,
                &death_var,
                "companion_pdialog",
                Some(campaign),
                None,
                &mut lines,
            )?;
            // Tag role in provenance after emit (emit does not take an extra role field).
            if emitted.lines_added > 0 {
                let start = lines.len().saturating_sub(emitted.lines_added);
                for line in &mut lines[start..] {
                    if let Ok(mut provenance) =
                        serde_json::from_str::<serde_json::Value>(&line.provenance_json)
                    {
                        provenance["pdialog_role"] =
                            serde_json::Value::String(role.to_string());
                        line.provenance_json = provenance.to_string();
                    }
                }
            }
            stats.side_dlgs_scanned += emitted.dlgs_scanned;
            stats.side_lines_added += emitted.lines_added;
        }
    }

    Ok((lines, extra_speakers, stats))
}

/// Prefix-based orphan scan: DLGs sharing a companion resref prefix but not listed
/// in `interdia.2da` / `pdialog.2da` or the main CRE scan (`excluded_dlgs`).
pub fn scan_companion_side_dlgs(
    res: &GameResources,
    tlk: &Tlk,
    token_reps: &TokenReplacements,
    existing: &HashSet<(u32, String, u32)>,
    excluded_dlgs: &HashSet<String>,
) -> Result<
    (
        Vec<AttributedLine>,
        Vec<AttributedSpeaker>,
        CompanionScanStats,
    ),
    AppError,
> {
    let mut death_vars: BTreeSet<String> = BTreeSet::new();
    for table_name in ["interdia", "pdialog"] {
        let Ok(bytes) = res.read(table_name, TYPE_2DA) else {
            continue;
        };
        let table = super::twoda::parse_2da(&bytes)?;
        for row in &table.rows {
            let death_var = row.label.trim().to_ascii_lowercase();
            if !death_var.is_empty() {
                death_vars.insert(death_var);
            }
        }
    }
    if death_vars.is_empty() {
        return Ok((Vec::new(), Vec::new(), CompanionScanStats::default()));
    }

    let mut lines = Vec::new();
    let mut extra_speakers = Vec::new();
    let mut speaker_seen: BTreeSet<String> = BTreeSet::new();
    let mut stats = CompanionScanStats::default();
    let mut dlgs_seen: BTreeSet<String> = BTreeSet::new();
    let all_dlgs = res.resrefs_of_type(TYPE_DLG);

    for death_var in death_vars {
        let Some(identity) = resolve_death_var_cre(res, &death_var) else {
            continue;
        };
        let prefixes = companion_dlg_search_prefixes(&death_var);

        let side_dlgs: Vec<String> = all_dlgs
            .iter()
            .filter(|d| {
                prefixes.iter().any(|p| d.starts_with(p)) && !excluded_dlgs.contains(d.as_str())
            })
            .cloned()
            .collect();
        if side_dlgs.is_empty() {
            continue;
        }

        if speaker_seen.insert(identity.cre_resref.clone()) {
            extra_speakers.push(companion_speaker(&identity));
        }

        for dlg_resref in side_dlgs {
            let dlg_lc = dlg_resref.to_ascii_lowercase();
            if !dlgs_seen.insert(dlg_lc.clone()) {
                continue;
            }
            let matched_prefix = prefixes
                .iter()
                .find(|p| dlg_lc.starts_with(*p))
                .copied()
                .unwrap_or("");
            let emitted = emit_companion_dlg_states(
                res,
                tlk,
                token_reps,
                existing,
                &identity,
                &dlg_lc,
                &death_var,
                "companion_side_dlg",
                None,
                Some(matched_prefix),
                &mut lines,
            )?;
            stats.side_dlgs_scanned += emitted.dlgs_scanned;
            stats.side_lines_added += emitted.lines_added;
        }
    }

    Ok((lines, extra_speakers, stats))
}

/// Per-companion voiced clips discovered from interdia / pdialog / side DLGs for harvest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanionVoicedSpeaker {
    pub cre_resref: String,
    pub identity_key: String,
    pub sources: Vec<VoicedSource>,
    pub unsafe_metadata_skipped: usize,
}

/// Live voiced-source pass for harvest: same companion DLG coverage as Attribution,
/// but only states with a non-conflicting TLK sound attachment (no line attribution).
pub fn companion_voiced_sources(
    res: &GameResources,
    tlk: &Tlk,
    conflicting_sounds: &HashSet<String>,
) -> Result<Vec<CompanionVoicedSpeaker>, AppError> {
    let mut by_cre: BTreeMap<String, CompanionVoicedBucket> = BTreeMap::new();
    let mut dlgs_seen: BTreeSet<String> = BTreeSet::new();
    let mut excluded_dlgs: HashSet<String> = HashSet::new();

    // Main CRE dialogs are harvested separately; exclude them from the side pass.
    for cre_resref in res.resrefs_of_type(TYPE_CRE) {
        if let Ok(cre) = load_cre(res, &cre_resref) {
            if let Some(dlg) = cre.dialog_resref {
                excluded_dlgs.insert(dlg.to_ascii_lowercase());
            }
        }
    }

    if let Ok(bytes) = res.read("interdia", TYPE_2DA) {
        let table = super::twoda::parse_2da(&bytes)?;
        let file_col = table.column_index("FILE");
        let tob_col = table.column_index("25FILE");
        for row in &table.rows {
            let death_var = row.label.trim().to_ascii_lowercase();
            let Some(identity) = resolve_death_var_cre(res, &death_var) else {
                continue;
            };
            for col in [file_col, tob_col].into_iter().flatten() {
                let Some(dlg) = row.values.get(col).map(|s| s.as_str()) else {
                    continue;
                };
                if !is_usable_dlg_resref(dlg) {
                    continue;
                }
                let dlg_lc = dlg.to_ascii_lowercase();
                excluded_dlgs.insert(dlg_lc.clone());
                if !dlgs_seen.insert(dlg_lc.clone()) {
                    continue;
                }
                collect_voiced_dlg(
                    res,
                    tlk,
                    conflicting_sounds,
                    &identity,
                    &dlg_lc,
                    &mut by_cre,
                )?;
            }
        }
    }

    if let Ok(bytes) = res.read("pdialog", TYPE_2DA) {
        let table = super::twoda::parse_2da(&bytes)?;
        for col_name in [
            "POST_DIALOG_FILE",
            "JOIN_DIALOG_FILE",
            "25POST_DIALOG_FILE",
            "25JOIN_DIALOG_FILE",
        ] {
            let Some(col) = table.column_index(col_name) else {
                continue;
            };
            for row in &table.rows {
                let death_var = row.label.trim().to_ascii_lowercase();
                if death_var.is_empty() {
                    continue;
                }
                let Some(identity) = resolve_death_var_cre(res, &death_var) else {
                    continue;
                };
                let Some(dlg) = row.values.get(col).map(|s| s.as_str()) else {
                    continue;
                };
                if !is_usable_dlg_resref(dlg) {
                    continue;
                }
                let dlg_lc = dlg.to_ascii_lowercase();
                excluded_dlgs.insert(dlg_lc.clone());
                if !dlgs_seen.insert(dlg_lc.clone()) {
                    continue;
                }
                collect_voiced_dlg(
                    res,
                    tlk,
                    conflicting_sounds,
                    &identity,
                    &dlg_lc,
                    &mut by_cre,
                )?;
            }
        }
    }

    let mut death_vars: BTreeSet<String> = BTreeSet::new();
    for table_name in ["interdia", "pdialog"] {
        let Ok(bytes) = res.read(table_name, TYPE_2DA) else {
            continue;
        };
        let table = super::twoda::parse_2da(&bytes)?;
        for row in &table.rows {
            let death_var = row.label.trim().to_ascii_lowercase();
            if !death_var.is_empty() {
                death_vars.insert(death_var);
            }
        }
    }
    if !death_vars.is_empty() {
        let all_dlgs = res.resrefs_of_type(TYPE_DLG);
        for death_var in death_vars {
            let Some(identity) = resolve_death_var_cre(res, &death_var) else {
                continue;
            };
            let prefixes = companion_dlg_search_prefixes(&death_var);
            for dlg_resref in &all_dlgs {
                if !prefixes.iter().any(|p| dlg_resref.starts_with(p)) {
                    continue;
                }
                if excluded_dlgs.contains(dlg_resref.as_str()) {
                    continue;
                }
                let dlg_lc = dlg_resref.to_ascii_lowercase();
                if !dlgs_seen.insert(dlg_lc.clone()) {
                    continue;
                }
                collect_voiced_dlg(
                    res,
                    tlk,
                    conflicting_sounds,
                    &identity,
                    &dlg_lc,
                    &mut by_cre,
                )?;
            }
        }
    }

    Ok(by_cre
        .into_iter()
        .map(|(_, bucket)| CompanionVoicedSpeaker {
            cre_resref: bucket.cre_resref,
            identity_key: bucket.identity_key,
            sources: bucket.sources,
            unsafe_metadata_skipped: bucket.unsafe_metadata_skipped,
        })
        .filter(|s| !s.sources.is_empty() || s.unsafe_metadata_skipped > 0)
        .collect())
}

#[derive(Debug)]
struct CompanionVoicedBucket {
    cre_resref: String,
    identity_key: String,
    sources: Vec<VoicedSource>,
    seen_sounds: HashSet<String>,
    unsafe_metadata_skipped: usize,
}

fn companion_identity_key(identity: &ResolvedCompanionCre) -> String {
    identity
        .cre
        .long_name_strref
        .map(|strref| strref.to_string())
        .unwrap_or_else(|| format!("ungrouped:{}", identity.cre_resref))
}

fn collect_voiced_dlg(
    res: &GameResources,
    tlk: &Tlk,
    conflicting_sounds: &HashSet<String>,
    identity: &ResolvedCompanionCre,
    dlg_resref: &str,
    by_cre: &mut BTreeMap<String, CompanionVoicedBucket>,
) -> Result<(), AppError> {
    let Some(dlg) = load_dlg(res, dlg_resref)? else {
        return Ok(());
    };
    let cre_key = identity.cre_resref.to_ascii_lowercase();
    let bucket = by_cre.entry(cre_key.clone()).or_insert_with(|| CompanionVoicedBucket {
        cre_resref: cre_key,
        identity_key: companion_identity_key(identity),
        sources: Vec::new(),
        seen_sounds: HashSet::new(),
        unsafe_metadata_skipped: 0,
    });
    for state in &dlg.states {
        let Some(strref) = state.text_strref else {
            continue;
        };
        let Ok(entry) = tlk.entry(strref) else {
            continue;
        };
        let Some(sound_resref) = entry.sound_resref else {
            continue;
        };
        let sound_lc = sound_resref.to_ascii_lowercase();
        if conflicting_sounds.contains(&sound_lc) || conflicting_sounds.contains(&sound_resref) {
            bucket.unsafe_metadata_skipped += 1;
            continue;
        }
        if !bucket.seen_sounds.insert(sound_lc.clone()) {
            continue;
        }
        bucket.sources.push(VoicedSource {
            strref,
            sound_resref: sound_lc,
            source_text: entry.text,
        });
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct EmitResult {
    lines_added: usize,
    dlgs_scanned: usize,
}

fn emit_companion_dlg_states(
    res: &GameResources,
    tlk: &Tlk,
    token_reps: &TokenReplacements,
    existing: &HashSet<(u32, String, u32)>,
    identity: &ResolvedCompanionCre,
    dlg_resref: &str,
    death_var: &str,
    method: &str,
    campaign: Option<&str>,
    dlg_prefix: Option<&str>,
    lines: &mut Vec<AttributedLine>,
) -> Result<EmitResult, AppError> {
    let Some(dlg) = load_dlg(res, dlg_resref)? else {
        return Ok(EmitResult::default());
    };
    let mut result = EmitResult {
        dlgs_scanned: 1,
        ..EmitResult::default()
    };

    for state in &dlg.states {
        let Some(strref) = state.text_strref else {
            continue;
        };
        let key = (strref, dlg_resref.to_string(), state.index);
        if existing.contains(&key) {
            continue;
        }
        let text = tlk.entry(strref).map(|e| e.text).unwrap_or_default();
        let facts = StrrefFacts {
            is_voiced: tlk
                .entry(strref)
                .map(|e| e.sound_resref.is_some())
                .unwrap_or(false),
            sound_resref: tlk.entry(strref).ok().and_then(|e| e.sound_resref.clone()),
        };
        let mut line = attribution::companion_state_line(
            &identity.cre_resref,
            &identity.cre,
            dlg_resref,
            state.index,
            strref,
            text,
            facts,
            death_var,
            method,
            campaign,
            dlg_prefix,
            token_reps,
        );
        let mut provenance: serde_json::Value =
            serde_json::from_str(&line.provenance_json).unwrap_or_else(|_| serde_json::json!({}));
        provenance["identity_resolution"] =
            serde_json::Value::String(identity.resolution_method.to_string());
        provenance["identity_strref"] = identity
            .cre
            .long_name_strref
            .map(serde_json::Value::from)
            .unwrap_or(serde_json::Value::Null);
        provenance["identity_variant_count"] =
            serde_json::Value::from(identity.identity_variant_count as u64);
        provenance["identity_candidate_count"] =
            serde_json::Value::from(identity.candidate_count as u64);
        line.provenance_json = provenance.to_string();
        lines.push(line);
        result.lines_added += 1;
    }

    Ok(result)
}

fn is_usable_dlg_resref(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty() && !t.eq_ignore_ascii_case("NONE") && !t.chars().all(|c| c == '*')
}

#[derive(Debug, Clone)]
struct ResolvedCompanionCre {
    cre_resref: String,
    cre: Cre,
    resolution_method: &'static str,
    identity_variant_count: usize,
    candidate_count: usize,
}

/// Resolve a death variable using the dominant named identity among its CRE variants.
fn resolve_death_var_cre(res: &GameResources, death_var: &str) -> Option<ResolvedCompanionCre> {
    let death_var = death_var.trim().to_ascii_lowercase();
    if death_var.is_empty() {
        return None;
    }
    // BG2EE ships party templates as jahei1, anomen7, ohhex8, … — not always
    // `<death_var>.cre`. Search every known prefix for that death variable.
    let prefixes = companion_cre_search_prefixes(&death_var);
    let mut candidates: Vec<(String, Cre)> = res
        .resrefs_of_type(TYPE_CRE)
        .into_iter()
        .filter(|r| prefixes.iter().any(|p| r.starts_with(p)))
        .filter_map(|r| load_cre(res, &r).ok().map(|cre| (r, cre)))
        .collect();
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    if candidates.is_empty() {
        return None;
    }

    let mut named_buckets: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for (index, (_, cre)) in candidates.iter().enumerate() {
        if let Some(strref) = cre.long_name_strref {
            named_buckets.entry(strref).or_default().push(index);
        }
    }

    if let Some((_, winning)) = named_buckets.iter().max_by(|(_, a), (_, b)| {
        let a_exact = a.iter().any(|&i| candidates[i].0 == death_var);
        let b_exact = b.iter().any(|&i| candidates[i].0 == death_var);
        let a_first = a
            .iter()
            .map(|&i| candidates[i].0.as_str())
            .min()
            .unwrap_or("");
        let b_first = b
            .iter()
            .map(|&i| candidates[i].0.as_str())
            .min()
            .unwrap_or("");
        a.len()
            .cmp(&b.len())
            .then_with(|| a_exact.cmp(&b_exact))
            .then_with(|| b_first.cmp(a_first))
    }) {
        let winner = *winning
            .iter()
            .max_by(|&&a, &&b| {
                let (a_ref, a_cre) = &candidates[a];
                let (b_ref, b_cre) = &candidates[b];
                let a_usable = a_cre
                    .dialog_resref
                    .as_deref()
                    .map(is_usable_dlg_resref)
                    .unwrap_or(false);
                let b_usable = b_cre
                    .dialog_resref
                    .as_deref()
                    .map(is_usable_dlg_resref)
                    .unwrap_or(false);
                (a_ref == &death_var)
                    .cmp(&(b_ref == &death_var))
                    .then_with(|| a_usable.cmp(&b_usable))
                    .then_with(|| b_ref.cmp(a_ref))
            })
            .expect("a named identity bucket is never empty");
        let (cre_resref, cre) = candidates[winner].clone();
        return Some(ResolvedCompanionCre {
            cre_resref,
            cre,
            resolution_method: "long_name_consensus",
            identity_variant_count: winning.len(),
            candidate_count: candidates.len(),
        });
    }

    let winner = candidates
        .iter()
        .position(|(resref, _)| resref == &death_var)
        .unwrap_or(0);
    let exact = candidates[winner].0 == death_var;
    let (cre_resref, cre) = candidates[winner].clone();
    Some(ResolvedCompanionCre {
        cre_resref,
        cre,
        resolution_method: if exact {
            "exact_resref_fallback"
        } else {
            "lexical_prefix_fallback"
        },
        identity_variant_count: 1,
        candidate_count: candidates.len(),
    })
}

/// Prefixes used to locate a companion's CRE templates from a death-variable label.
fn companion_cre_search_prefixes(death_var: &str) -> Vec<&str> {
    match death_var {
        "jaheira" => vec!["jahei"],
        "anomen" => vec!["anome"],
        "haerdalis" => vec!["haer"],
        "keldorn" => vec!["keld"],
        "korgan" => vec!["korg"],
        "valygar" => vec!["valyg"],
        "viconia" => vec!["vicon"],
        // Party templates are yoshi7/yoshi8/…; never yoshimo.cre.
        "yoshimo" => vec!["yoshi"],
        "imoen2" | "imoen" => vec!["imoen"],
        "sarevok" => vec!["sarev"],
        "rasaad" => vec!["rasaa"],
        // Beamdog Hexxat CRE templates are ohhex*; hexxat.cre is the dialog owner only.
        "hexxat" => vec!["ohhex", "hexxat", "hexxa"],
        "ohhfak" => vec!["ohhfak"],
        _ => vec![death_var],
    }
}

/// Prefixes used to match orphan companion DLG resrefs (side-chain pass).
///
/// Broader than CRE search for cases like Yoshimo, where party files are `yoshp` /
/// `yoshj` but CRE templates are `yoshi*`.
fn companion_dlg_search_prefixes(death_var: &str) -> Vec<&str> {
    let mut out = companion_cre_search_prefixes(death_var);
    match death_var {
        "yoshimo" => {
            // yoshp / yoshj / yoshimo / yoshimox
            if !out.iter().any(|p| *p == "yosh") {
                out.push("yosh");
            }
        }
        "hexxat" => {
            if !out.iter().any(|p| *p == "hexxat") {
                out.push("hexxat");
            }
        }
        "haerdalis" => {
            // haerdap / haerdaj share the haerda* stem beyond CRE prefix "haer".
            if !out.iter().any(|p| *p == "haerda") {
                out.push("haerda");
            }
        }
        _ => {}
    }
    out.sort_unstable();
    out.dedup();
    // Prefer longer prefixes first so matching provenance reports the tightest stem.
    out.sort_by_key(|p| std::cmp::Reverse(p.len()));
    out
}

fn load_cre(res: &GameResources, cre_resref: &str) -> Result<Cre, AppError> {
    let bytes = res.read(cre_resref, TYPE_CRE)?;
    Cre::parse(&bytes)
}

fn load_dlg(res: &GameResources, dlg_resref: &str) -> Result<Option<Dlg>, AppError> {
    let Some(src) = res.resolve(dlg_resref, TYPE_DLG) else {
        return Ok(None);
    };
    let bytes = res.read_source(&src)?;
    Ok(Some(Dlg::parse(&bytes)?))
}

fn companion_speaker(identity: &ResolvedCompanionCre) -> AttributedSpeaker {
    let cre_resref = &identity.cre_resref;
    let cre = &identity.cre;
    AttributedSpeaker {
        cre_resref: cre_resref.to_ascii_lowercase(),
        dialogue_resref: cre.dialog_resref.clone().map(|d| d.to_ascii_lowercase()),
        sex: cre.sex,
        race: cre.race,
        class: cre.class,
        kit: cre.kit,
        alignment: cre.alignment,
        creature_category: cre.general,
        long_name_strref: cre.long_name_strref,
        display_name: None,
        confidence: CONFIDENCE_UNIQUE,
        provenance_json: serde_json::json!({
            "method": "companion_interdia",
            "cre_resref": cre_resref.to_ascii_lowercase(),
            "identity_resolution": identity.resolution_method,
            "identity_strref": cre.long_name_strref,
            "identity_variant_count": identity.identity_variant_count,
            "identity_candidate_count": identity.candidate_count,
        })
        .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extractor::cre::build_cre;
    use crate::extractor::dlg::build_dlg;
    use crate::extractor::restype::{TYPE_2DA, TYPE_CRE, TYPE_DLG};
    use crate::extractor::tlk::build_tlk;
    use crate::extractor::twoda::parse_2da;
    use std::path::Path;

    fn interdia_bytes() -> Vec<u8> {
        b"2DA V1.0\n\
NONE\n\
FILE\t25FILE\n\
JAHEIRA\tBJAHEIR\tBJAHEI25\n"
            .to_vec()
    }

    fn cre_with_identity(long_name_strref: Option<u32>, dialog_resref: Option<&str>) -> Vec<u8> {
        let mut cre = build_cre();
        let strref = long_name_strref.unwrap_or(u32::MAX);
        cre[0x0008..0x000C].copy_from_slice(&strref.to_le_bytes());
        cre[0x02CC..0x02D4].fill(0);
        if let Some(dialog) = dialog_resref {
            let bytes = dialog.as_bytes();
            cre[0x02CC..0x02CC + bytes.len().min(8)].copy_from_slice(&bytes[..bytes.len().min(8)]);
        }
        cre
    }

    fn build_key(bif_rel_path: &str, resources: &[(&str, u16, u32)]) -> Vec<u8> {
        const BIF_ENTRY_LEN: usize = 12;
        const RES_ENTRY_LEN: usize = 14;
        let bif_name = format!("{bif_rel_path}\0");
        let bif_off = 0x18usize;
        let res_off = bif_off + BIF_ENTRY_LEN;
        let strings_off = res_off + resources.len() * RES_ENTRY_LEN;

        let mut out = Vec::new();
        out.extend_from_slice(b"KEY V1.0");
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&(resources.len() as u32).to_le_bytes());
        out.extend_from_slice(&(bif_off as u32).to_le_bytes());
        out.extend_from_slice(&(res_off as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(strings_off as u32).to_le_bytes());
        out.extend_from_slice(&(bif_name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        for (name, rtype, file_index) in resources {
            let mut resref = [0u8; 8];
            let b = name.as_bytes();
            resref[..b.len().min(8)].copy_from_slice(&b[..b.len().min(8)]);
            out.extend_from_slice(&resref);
            out.extend_from_slice(&rtype.to_le_bytes());
            let locator = file_index & 0x3FFF;
            out.extend_from_slice(&locator.to_le_bytes());
        }
        out.extend_from_slice(bif_name.as_bytes());
        out
    }

    fn write_test_game(dir: &Path, resources: &[(&str, u16, u32, &[u8])]) {
        let bif_files: Vec<_> = resources
            .iter()
            .map(|(_, rtype, idx, data)| (*idx, *rtype, *data))
            .collect();
        let bif_bytes = crate::extractor::bif::build_bif(&bif_files);
        std::fs::create_dir_all(dir.join("data")).unwrap();
        std::fs::write(dir.join("data/test.bif"), bif_bytes).unwrap();
        let key_resources: Vec<_> = resources
            .iter()
            .map(|(name, rtype, idx, _)| (*name, *rtype, *idx))
            .collect();
        std::fs::write(
            dir.join("chitin.key"),
            build_key("data\\test.bif", &key_resources),
        )
        .unwrap();
    }

    fn jahei_fixture_game(dir: &Path) {
        let mut cre = build_cre();
        cre[0x02CC..0x02D4].fill(0);
        let jaheiraj = build_dlg(&[(0, 0, 0)], &[]);
        let bjaheir = build_dlg(&[(100, 0, 0)], &[]);
        let jaheira = build_dlg(&[(200, 0, 0)], &[]);
        write_test_game(
            dir,
            &[
                ("interdia", TYPE_2DA, 0, &interdia_bytes()),
                ("jahei1", TYPE_CRE, 1, &cre),
                ("bjaheir", TYPE_DLG, 2, &bjaheir),
                ("jaheira", TYPE_DLG, 3, &jaheira),
                ("jaheiraj", TYPE_DLG, 4, &jaheiraj),
            ],
        );
    }

    #[test]
    fn companion_voiced_sources_collects_interdia_sounds() {
        let dir = tempfile::tempdir().unwrap();
        let cre = cre_with_identity(Some(9_456), Some("JAHEIRA"));
        let bjaheir = build_dlg(&[(1, 0, 0)], &[]);
        write_test_game(
            dir.path(),
            &[
                ("interdia", TYPE_2DA, 0, &interdia_bytes()),
                ("jahei1", TYPE_CRE, 1, &cre),
                ("bjaheir", TYPE_DLG, 2, &bjaheir),
            ],
        );
        let res = GameResources::open(dir.path()).unwrap();
        let tlk = Tlk::parse(build_tlk(
            0,
            &[
                (0x01, "", ""),
                (
                    0x03,
                    "bjahe01",
                    "A rich companion banter line for cloning references.",
                ),
            ],
        ))
        .unwrap();

        let got = companion_voiced_sources(&res, &tlk, &HashSet::new()).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].cre_resref, "jahei1");
        assert_eq!(got[0].identity_key, "9456");
        assert_eq!(got[0].sources.len(), 1);
        assert_eq!(got[0].sources[0].sound_resref, "bjahe01");
        assert_eq!(got[0].sources[0].strref, 1);
    }

    #[test]
    fn companion_cre_search_prefixes_cover_party_rows() {
        assert_eq!(companion_cre_search_prefixes("jaheira"), vec!["jahei"]);
        assert_eq!(companion_cre_search_prefixes("anomen"), vec!["anome"]);
        assert_eq!(companion_cre_search_prefixes("cernd"), vec!["cernd"]);
        assert_eq!(
            companion_cre_search_prefixes("hexxat"),
            vec!["ohhex", "hexxat", "hexxa"]
        );
        assert_eq!(companion_cre_search_prefixes("yoshimo"), vec!["yoshi"]);
    }

    #[test]
    fn companion_dlg_prefixes_cover_yoshimo_party_files() {
        let prefixes = companion_dlg_search_prefixes("yoshimo");
        assert!(prefixes.iter().any(|p| "yoshp".starts_with(p)));
        assert!(prefixes.iter().any(|p| "yoshj".starts_with(p)));
        assert!(prefixes.iter().any(|p| "yoshimo".starts_with(p)));
    }

    #[test]
    fn companion_identity_resolves_hexxat_via_ohhex_templates() {
        let dir = tempfile::tempdir().unwrap();
        let hexxat = cre_with_identity(Some(77_001), Some("HEXXAT"));
        write_test_game(dir.path(), &[("ohhex8", TYPE_CRE, 0, &hexxat)]);
        let res = GameResources::open(dir.path()).unwrap();

        let identity = resolve_death_var_cre(&res, "hexxat").unwrap();
        assert_eq!(identity.cre_resref, "ohhex8");
        assert_eq!(identity.cre.long_name_strref, Some(77_001));
    }

    fn pdialog_bytes() -> Vec<u8> {
        b"2DA V1.0\n\
multig\n\
POST_DIALOG_FILE\tJOIN_DIALOG_FILE\tDREAM_SCRIPT_FILE\t25POST_DIALOG_FILE\t25JOIN_DIALOG_FILE\t25DREAM_SCRIPT_FILE\t25OVERRIDE_SCRIPT_FILE\n\
YOSHIMO\tYOSHP\tYOSHJ\tYOSHD\tYOSHP\tYOSHJ\tYOSHD\tyosh25\n"
            .to_vec()
    }

    #[test]
    fn pdialog_scan_includes_post_and_join_files() {
        let dir = tempfile::tempdir().unwrap();
        let cre = cre_with_identity(Some(9_999), Some("YOSHIMO"));
        let yoshp = build_dlg(&[(0, 0, 0)], &[]);
        let yoshj = build_dlg(&[(0, 0, 0)], &[]);
        write_test_game(
            dir.path(),
            &[
                ("pdialog", TYPE_2DA, 0, &pdialog_bytes()),
                ("yoshi8", TYPE_CRE, 1, &cre),
                ("yoshp", TYPE_DLG, 2, &yoshp),
                ("yoshj", TYPE_DLG, 3, &yoshj),
            ],
        );
        let res = GameResources::open(dir.path()).unwrap();
        let tlk = Tlk::parse(build_tlk(0, &[(0x01, "", "wait here")])).unwrap();
        let reps = TokenReplacements::default();

        let (lines, speakers, stats) =
            scan_pdialog(&res, &tlk, &reps, &HashSet::new()).unwrap();
        assert!(stats.side_dlgs_scanned >= 2, "expected post+join dlgs scanned");
        assert!(stats.side_lines_added >= 2);
        assert_eq!(speakers.len(), 1);
        assert_eq!(speakers[0].cre_resref, "yoshi8");
        let dlgs: HashSet<_> = lines.iter().map(|l| l.dlg_resref.as_str()).collect();
        assert!(dlgs.contains("yoshp"));
        assert!(dlgs.contains("yoshj"));
        assert!(lines
            .iter()
            .all(|l| l.provenance_json.contains("companion_pdialog")));
        assert!(lines
            .iter()
            .any(|l| l.provenance_json.contains("pdialog_role")));
    }

    #[test]
    fn pdialog_dlg_resrefs_collects_post_join_columns() {
        let dir = tempfile::tempdir().unwrap();
        write_test_game(dir.path(), &[("pdialog", TYPE_2DA, 0, &pdialog_bytes())]);
        let res = GameResources::open(dir.path()).unwrap();
        let set = pdialog_dlg_resrefs(&res).unwrap();
        assert!(set.contains("yoshp"));
        assert!(set.contains("yoshj"));
    }

    #[test]
    fn companion_identity_prefers_dominant_long_name_over_lexical_first() {
        let dir = tempfile::tempdir().unwrap();
        let harper = cre_with_identity(Some(61_416), Some("NONE"));
        let jaheira_a = cre_with_identity(Some(9_456), Some("JAHEIRA"));
        let jaheira_b = cre_with_identity(Some(9_456), Some("JAHEI25A"));
        write_test_game(
            dir.path(),
            &[
                ("jahei1", TYPE_CRE, 0, &harper),
                ("jahei12b", TYPE_CRE, 1, &jaheira_a),
                ("jahei14", TYPE_CRE, 2, &jaheira_b),
            ],
        );
        let res = GameResources::open(dir.path()).unwrap();

        let identity = resolve_death_var_cre(&res, "JAHEIRA").unwrap();

        assert_eq!(identity.cre_resref, "jahei12b");
        assert_eq!(identity.cre.long_name_strref, Some(9_456));
        assert_eq!(identity.resolution_method, "long_name_consensus");
        assert_eq!(identity.identity_variant_count, 2);
        assert_eq!(identity.candidate_count, 3);
    }

    #[test]
    fn companion_identity_tie_prefers_exact_death_variable_resref() {
        let dir = tempfile::tempdir().unwrap();
        let exact = cre_with_identity(Some(100), Some("CERNDP"));
        let variant = cre_with_identity(Some(200), Some("CERNDP"));
        write_test_game(
            dir.path(),
            &[
                ("cernd", TYPE_CRE, 0, &exact),
                ("cernd2", TYPE_CRE, 1, &variant),
            ],
        );
        let res = GameResources::open(dir.path()).unwrap();

        let identity = resolve_death_var_cre(&res, "cernd").unwrap();

        assert_eq!(identity.cre_resref, "cernd");
        assert_eq!(identity.cre.long_name_strref, Some(100));
    }

    #[test]
    fn companion_identity_unnamed_candidates_keep_stable_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let first = cre_with_identity(None, None);
        let second = cre_with_identity(None, None);
        write_test_game(
            dir.path(),
            &[
                ("korgan2", TYPE_CRE, 0, &second),
                ("korgan1", TYPE_CRE, 1, &first),
            ],
        );
        let res = GameResources::open(dir.path()).unwrap();

        let identity = resolve_death_var_cre(&res, "korgan").unwrap();

        assert_eq!(identity.cre_resref, "korgan1");
        assert_eq!(identity.resolution_method, "lexical_prefix_fallback");
    }

    #[test]
    fn skips_unusable_dlg_cells() {
        assert!(!is_usable_dlg_resref("NONE"));
        assert!(!is_usable_dlg_resref("****"));
        assert!(is_usable_dlg_resref("BJAHEIR"));
    }

    #[test]
    fn parse_fixture_interdia_columns() {
        let table = parse_2da(&interdia_bytes()).unwrap();
        assert_eq!(table.cell("JAHEIRA", "FILE"), Some("BJAHEIR"));
    }

    #[test]
    fn side_dlg_scan_includes_prefix_orphans_and_excludes_banter_and_main() {
        let dir = tempfile::tempdir().unwrap();
        jahei_fixture_game(dir.path());
        let res = GameResources::open(dir.path()).unwrap();
        let tlk = Tlk::parse(build_tlk(
            0,
            &[(0x01, "", "But it is the truth, <CHARNAME>.")],
        ))
        .unwrap();
        let reps = TokenReplacements::default();

        let mut excluded = HashSet::from(["jaheira".to_string(), "bjaheir".to_string()]);
        excluded.insert("bjahei25".to_string());

        let (lines, _, stats) =
            scan_companion_side_dlgs(&res, &tlk, &reps, &HashSet::new(), &excluded).unwrap();
        assert_eq!(stats.side_dlgs_scanned, 1);
        assert_eq!(stats.side_lines_added, 1);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].dlg_resref, "jaheiraj");
        assert_eq!(lines[0].strref, 0);
        assert_eq!(lines[0].speaker_cre_resref.as_deref(), Some("jahei1"));
        assert!(lines[0].provenance_json.contains("companion_side_dlg"));
        assert!(lines[0].provenance_json.contains("dlg_prefix"));
        assert!(lines[0].provenance_json.contains("long_name_consensus"));
        assert!(lines[0].provenance_json.contains("identity_strref"));
    }

    #[test]
    fn side_dlg_scan_skips_existing_keys() {
        let dir = tempfile::tempdir().unwrap();
        jahei_fixture_game(dir.path());
        let res = GameResources::open(dir.path()).unwrap();
        let tlk = Tlk::parse(build_tlk(0, &[(0, "line", "")])).unwrap();
        let reps = TokenReplacements::default();
        let existing = HashSet::from([(0u32, "jaheiraj".to_string(), 0u32)]);

        let (lines, _, stats) = scan_companion_side_dlgs(
            &res,
            &tlk,
            &reps,
            &existing,
            &HashSet::from(["jaheira".to_string()]),
        )
        .unwrap();
        assert_eq!(stats.side_lines_added, 0);
        assert!(lines.is_empty());
    }

    #[test]
    fn interdia_banter_dlg_resrefs_collects_file_columns() {
        let dir = tempfile::tempdir().unwrap();
        jahei_fixture_game(dir.path());
        let res = GameResources::open(dir.path()).unwrap();
        let banter = interdia_banter_dlg_resrefs(&res).unwrap();
        assert!(banter.contains("bjaheir"));
        assert!(!banter.contains("jaheiraj"));
    }
}
