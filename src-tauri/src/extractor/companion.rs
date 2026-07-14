//! Companion banter/interjection dialogue from `interdia.2da`.
//!
//! Party NPCs store banter and interjection lines in DLGs listed by `interdia.2da`,
//! not in the CRE's `dialog_resref`. This module loads those DLGs and attributes
//! their actor states to the companion CRE named by each row's death variable.
//! A follow-on pass also picks up orphan side-chain DLGs (e.g. `jaheiraj.dlg`) whose
//! resref shares the companion prefix but is not the CRE's main dialogue or an
//! `interdia` banter file.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::error::AppError;

use super::attribution::{self, AttributedLine, AttributedSpeaker, StrrefFacts, CONFIDENCE_UNIQUE};
use super::cre::Cre;
use super::dlg::Dlg;
use super::resource::GameResources;
use super::restype::{TYPE_2DA, TYPE_CRE, TYPE_DLG};
use super::tlk::Tlk;
use super::token_resolve::TokenReplacements;

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

/// Companion CRE resref prefixes derived from every `interdia.2da` row label.
pub fn interdia_companion_prefixes(res: &GameResources) -> Result<HashSet<String>, AppError> {
    let mut out = HashSet::new();
    let bytes = match res.read("interdia", TYPE_2DA) {
        Ok(b) => b,
        Err(_) => return Ok(out),
    };
    let table = super::twoda::parse_2da(&bytes)?;
    for row in &table.rows {
        let death_var = row.label.trim().to_ascii_lowercase();
        if !death_var.is_empty() {
            out.insert(companion_cre_prefix(&death_var).to_string());
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

/// Prefix-based orphan scan: DLGs sharing a companion resref prefix but not listed
/// in `interdia.2da` or the main CRE scan (`excluded_dlgs`).
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
    let all_dlgs = res.resrefs_of_type(TYPE_DLG);

    for row in &table.rows {
        let death_var = row.label.trim().to_ascii_lowercase();
        let Some(identity) = resolve_death_var_cre(res, &death_var) else {
            continue;
        };
        let prefix = companion_cre_prefix(&death_var);

        let mut excluded_for_row = excluded_dlgs.clone();
        if let Some(col) = file_col {
            if let Some(dlg) = row.values.get(col) {
                if is_usable_dlg_resref(dlg) {
                    excluded_for_row.insert(dlg.trim().to_ascii_lowercase());
                }
            }
        }
        if let Some(col) = tob_col {
            if let Some(dlg) = row.values.get(col) {
                if is_usable_dlg_resref(dlg) {
                    excluded_for_row.insert(dlg.trim().to_ascii_lowercase());
                }
            }
        }

        let side_dlgs: Vec<String> = all_dlgs
            .iter()
            .filter(|d| d.starts_with(prefix) && !excluded_for_row.contains(d.as_str()))
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
                Some(prefix),
                &mut lines,
            )?;
            stats.side_dlgs_scanned += emitted.dlgs_scanned;
            stats.side_lines_added += emitted.lines_added;
        }
    }

    Ok((lines, extra_speakers, stats))
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
    // BG2EE ships party templates as jahei1, anomen7, … — not jaheira.cre / anomen.cre.
    let prefix = companion_cre_prefix(&death_var);
    let mut candidates: Vec<(String, Cre)> = res
        .resrefs_of_type(TYPE_CRE)
        .into_iter()
        .filter(|r| r.starts_with(prefix))
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

/// Prefix for locating a companion's CRE template from an `interdia.2da` row label.
fn companion_cre_prefix(death_var: &str) -> &str {
    match death_var {
        "jaheira" => "jahei",
        "anomen" => "anome",
        "haerdalis" => "haer",
        "keldorn" => "keld",
        "korgan" => "korg",
        "valygar" => "valyg",
        "viconia" => "vicon",
        "yoshimo" => "yoshi",
        "imoen2" => "imoen",
        "sarevok" => "sarev",
        "rasaad" => "rasaa",
        "hexxat" => "hexxa",
        _ => death_var,
    }
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
    fn companion_cre_prefix_covers_party_rows() {
        assert_eq!(companion_cre_prefix("jaheira"), "jahei");
        assert_eq!(companion_cre_prefix("anomen"), "anome");
        assert_eq!(companion_cre_prefix("cernd"), "cernd");
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
