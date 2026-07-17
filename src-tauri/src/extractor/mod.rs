//! Game resource resolution for an installed (modded) BG2EE setup.
//!
//! Resolves the ACTIVE, merged game state - `override/` taking precedence over the
//! BIF archives named in `chitin.key`, and the active-language `dialog.tlk` - then
//! parses the formats item-04 needs: TLK (strref -> text/flags/sound), DLG (actor
//! states vs player transitions), and CRE (factual creature metadata). Readers use
//! bounds-checked manual little-endian parsing (no external IE crate) so the module
//! stays self-contained and fixture-testable. See `docs/adr/0003-repo-module-layout.md`.

mod bytes;
pub mod attribution;
pub mod bif;
pub mod companion;
pub mod cre;
pub mod dlg;
pub mod ids;
pub mod key;
pub mod lang;
pub mod resource;
pub mod restype;
pub mod spoken_text;
pub mod tlk;
pub mod token_resolve;
pub mod tokens;
pub mod twoda;
pub mod views;

use std::path::Path;

use crate::error::AppError;

use attribution::{AttributedLine, AttributedSpeaker, CreDialog, SharedStrrefGroup, StrrefFacts};
use companion::{
    companion_voiced_sources, interdia_banter_dlg_resrefs, pdialog_dlg_resrefs, CompanionScanStats,
    scan_companion_side_dlgs, scan_interdia, scan_pdialog,
};
use resource::GameResources;
use tlk::Tlk;
use views::{CreView, DlgView, GameLanguages, TlkEntryView, TlkSummary};

/// The owned outputs of an attribution scan (item-06), ready to persist.
pub struct AttributionScan {
    pub speakers: Vec<AttributedSpeaker>,
    pub lines: Vec<AttributedLine>,
    pub groups: Vec<SharedStrrefGroup>,
    pub companion: CompanionScanStats,
}

/// A voiced original clip resolved for a speaker (item-07 harvest input): the
/// source strref plus its attached sound resref. Only the resref/strref travel -
/// never the original bytes (the harvest IO layer resolves + decodes them).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoicedSource {
    pub strref: u32,
    pub sound_resref: String,
    pub source_text: String,
}

/// Per-speaker voiced sources for reference harvesting: clips the CRE speaks
/// through the DLG it UNIQUELY owns (`dialogue`), companion banter/post/join/side
/// DLGs (`companion`), and voiced entries in its SNDSLOT.IDS soundset (`slots`).
/// Ambiguous (shared-DLG) speakers omit main-dialogue clips so a shared clip is
/// never mistaken for one NPC's voice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeakerSources {
    pub cre_resref: String,
    /// Stable character identity for cross-CRE source-reuse checks. Named CRE
    /// variants share their TLK long-name strref; unnamed CREs remain separate.
    pub identity_key: String,
    pub dialogue: Vec<VoicedSource>,
    /// Voiced states from companion interdia / pdialog / side-chain DLGs.
    pub companion: Vec<VoicedSource>,
    pub slots: Vec<VoicedSource>,
    /// Source occurrences rejected because one sound resref advertised multiple
    /// distinct non-empty transcripts in the active TLK.
    pub unsafe_metadata_skipped: usize,
}

/// Sound resources attached to contradictory non-empty transcripts in one TLK.
fn conflicting_sound_resrefs(tlk: &Tlk) -> std::collections::HashSet<String> {
    let mut texts: std::collections::HashMap<String, std::collections::HashSet<String>> =
        std::collections::HashMap::new();
    for strref in 0..tlk.count {
        let Ok(entry) = tlk.entry(strref) else {
            continue;
        };
        let Some(sound) = entry.sound_resref else {
            continue;
        };
        let normalized = entry
            .text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        if !normalized.is_empty() {
            texts.entry(sound).or_default().insert(normalized);
        }
    }
    texts
        .into_iter()
        .filter_map(|(sound, variants)| (variants.len() > 1).then_some(sound))
        .collect()
}

/// Resolve, per uniquely-attributed speaker, the voiced original clips usable as
/// voice references (item-07). Mirrors the CRE->DLG->TLK resolution of
/// [`scan_attribution`] but surfaces the attached sound resref of each voiced
/// state (and voiced soundset slot) that the pure `candidates` selector needs.
pub fn harvest_sources(
    game_dir: &Path,
    locale: Option<&str>,
) -> Result<Vec<SpeakerSources>, AppError> {
    let res = GameResources::open(game_dir)?;
    let paths = lang::resolve_tlk(game_dir, locale)?;
    let tlk = Tlk::parse(std::fs::read(&paths.dialog)?)?;
    let conflicting_sounds = conflicting_sound_resrefs(&tlk);

    // Count DLG owners so ambiguous (shared) speakers can be excluded.
    let mut cres: Vec<(String, cre::Cre, dlg::Dlg)> = Vec::new();
    let mut owners: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for resref in res.resrefs_of_type(restype::TYPE_CRE) {
        let Some(src) = res.resolve(&resref, restype::TYPE_CRE) else {
            continue;
        };
        let Ok(bytes) = res.read_source(&src) else {
            continue;
        };
        let Ok(creature) = cre::Cre::parse(&bytes) else {
            continue;
        };
        let Some(dlg_resref) = creature.dialog_resref.clone() else {
            continue;
        };
        let Some(dsrc) = res.resolve(&dlg_resref, restype::TYPE_DLG) else {
            continue;
        };
        let Ok(dbytes) = res.read_source(&dsrc) else {
            continue;
        };
        let Ok(dialogue) = dlg::Dlg::parse(&dbytes) else {
            continue;
        };
        *owners.entry(dlg_resref.to_ascii_lowercase()).or_default() += 1;
        cres.push((resref.to_ascii_lowercase(), creature, dialogue));
    }

    // Resolve the attached sound resref for a voiced strref (None when unvoiced).
    let sound_of = |strref: u32| -> Option<String> {
        tlk.entry(strref).ok().and_then(|e| e.sound_resref)
    };
    let text_of = |strref: u32| -> String {
        tlk.entry(strref).ok().map(|e| e.text).unwrap_or_default()
    };

    let mut out: std::collections::BTreeMap<String, SpeakerSources> =
        std::collections::BTreeMap::new();
    for (cre_resref, creature, dialogue) in &cres {
        let dlg_lc = creature
            .dialog_resref
            .as_ref()
            .map(|d| d.to_ascii_lowercase());
        // Only uniquely-owned dialogue is a trustworthy voice proof.
        let unique = dlg_lc
            .as_ref()
            .and_then(|d| owners.get(d))
            .map(|n| *n <= 1)
            .unwrap_or(false);

        let mut dialogue_sources = Vec::new();
        let mut unsafe_metadata_skipped = 0usize;
        if unique {
            for state in &dialogue.states {
                if let Some(strref) = state.text_strref {
                    if let Some(sound_resref) = sound_of(strref) {
                        if conflicting_sounds.contains(&sound_resref) {
                            unsafe_metadata_skipped += 1;
                            continue;
                        }
                        dialogue_sources.push(VoicedSource {
                            strref,
                            sound_resref,
                            source_text: text_of(strref),
                        });
                    }
                }
            }
        }

        let mut slot_sources = Vec::new();
        for &strref in &creature.sound_slots {
            if let Some(sound_resref) = sound_of(strref) {
                if conflicting_sounds.contains(&sound_resref) {
                    unsafe_metadata_skipped += 1;
                    continue;
                }
                slot_sources.push(VoicedSource {
                    strref,
                    sound_resref,
                    source_text: text_of(strref),
                });
            }
        }

        if !dialogue_sources.is_empty()
            || !slot_sources.is_empty()
            || unsafe_metadata_skipped > 0
        {
            out.insert(
                cre_resref.clone(),
                SpeakerSources {
                    cre_resref: cre_resref.clone(),
                    identity_key: creature
                        .long_name_strref
                        .map(|strref| strref.to_string())
                        .unwrap_or_else(|| format!("ungrouped:{cre_resref}")),
                    dialogue: dialogue_sources,
                    companion: Vec::new(),
                    slots: slot_sources,
                    unsafe_metadata_skipped,
                },
            );
        }
    }

    for companion in companion_voiced_sources(&res, &tlk, &conflicting_sounds)? {
        let entry = out
            .entry(companion.cre_resref.clone())
            .or_insert_with(|| SpeakerSources {
                cre_resref: companion.cre_resref.clone(),
                identity_key: companion.identity_key.clone(),
                dialogue: Vec::new(),
                companion: Vec::new(),
                slots: Vec::new(),
                unsafe_metadata_skipped: 0,
            });
        entry.companion = companion.sources;
        entry.unsafe_metadata_skipped += companion.unsafe_metadata_skipped;
        if entry.identity_key.starts_with("ungrouped:")
            && !companion.identity_key.starts_with("ungrouped:")
        {
            entry.identity_key = companion.identity_key;
        }
    }

    Ok(out.into_values().collect())
}

#[cfg(test)]
mod harvest_policy_tests {
    use super::*;

    #[test]
    fn detects_conflicting_tlk_transcripts_for_one_sound() {
        let tlk = Tlk::parse(tlk::build_tlk(
            0,
            &[
                (0x03, "OGREM01", "Hoo hoo ha ha ha ha haa!"),
                (0x03, "ogrem01", "A completely unrelated spoken sentence."),
                (0x03, "same01", "  The same sentence.  "),
                (0x03, "same01", "The same sentence."),
            ],
        ))
        .unwrap();
        let conflicts = conflicting_sound_resrefs(&tlk);
        assert!(conflicts.contains("ogrem01"));
        assert!(!conflicts.contains("same01"));
    }
}

/// Scan an install: resolve every CRE, pair it with the DLG it owns, resolve the
/// active TLK for text + voiced facts, then attribute speakers/lines and group
/// shared strrefs. Pure attribution logic lives in `attribution`; this function
/// is the IO orchestration (resource + TLK resolution) around it.
///
/// `on_progress(done, total)` is called as the CRE loop advances (the command layer
/// throttles + emits it); `total` is the CRE count for a determinate bar.
/// `should_cancel` is polled per CRE; when it returns true the loop stops early and
/// attributes whatever was parsed so far (a clean partial scan). Both callbacks keep
/// this module free of any Tauri dependency (see item-06b / ADR 0003).
pub fn scan_attribution(
    game_dir: &Path,
    locale: Option<&str>,
    token_reps: &token_resolve::TokenReplacements,
    mut on_progress: impl FnMut(usize, usize),
    should_cancel: impl Fn() -> bool,
) -> Result<AttributionScan, AppError> {
    let res = GameResources::open(game_dir)?;
    let paths = lang::resolve_tlk(game_dir, locale)?;
    let tlk = Tlk::parse(std::fs::read(&paths.dialog)?)?;

    // Parse each CRE that owns a dialogue, keeping the parsed Cre + Dlg owned so
    // the borrow-based `CreDialog` inputs can reference them for `attribute`.
    let resrefs = res.resrefs_of_type(restype::TYPE_CRE);
    let total = resrefs.len();
    let mut owned: Vec<(String, cre::Cre, dlg::Dlg)> = Vec::new();
    for (i, resref) in resrefs.iter().enumerate() {
        if should_cancel() {
            break;
        }
        on_progress(i, total);
        let Some(src) = res.resolve(resref, restype::TYPE_CRE) else {
            continue;
        };
        let Ok(bytes) = res.read_source(&src) else { continue };
        let Ok(creature) = cre::Cre::parse(&bytes) else {
            continue;
        };
        let Some(dlg_resref) = creature.dialog_resref.clone() else {
            continue;
        };
        let Some(dsrc) = res.resolve(&dlg_resref, restype::TYPE_DLG) else {
            continue;
        };
        let Ok(dbytes) = res.read_source(&dsrc) else {
            continue;
        };
        let Ok(dialogue) = dlg::Dlg::parse(&dbytes) else {
            continue;
        };
        owned.push((resref.to_ascii_lowercase(), creature, dialogue));
    }
    on_progress(total, total);

    let inputs: Vec<CreDialog<'_>> = owned
        .iter()
        .map(|(r, c, d)| CreDialog {
            cre_resref: r.clone(),
            cre: c,
            dlg: d,
        })
        .collect();

    let (mut speakers, mut lines) = attribution::attribute(
        &inputs,
        |strref| tlk.entry(strref).map(|e| e.text).unwrap_or_default(),
        |strref| StrrefFacts {
            is_voiced: tlk
                .entry(strref)
                .map(|e| e.sound_resref.is_some())
                .unwrap_or(false),
            sound_resref: tlk.entry(strref).ok().and_then(|e| e.sound_resref.clone()),
        },
        // No voice/clone signal is available during the scan (clones don't exist
        // yet, and the extractor holds no DB): the multi-owner tiebreak resolves on
        // resref-match -> named -> lexical, which the investigation confirmed
        // decides every recoverable multi-owner DLG. See attribution::attribute.
        |_| false,
        token_reps,
    );

    let mut existing_keys: std::collections::HashSet<(u32, String, u32)> = lines
        .iter()
        .map(|l| (l.strref, l.dlg_resref.clone(), l.state_index))
        .collect();
    let (companion_lines, companion_speakers, mut companion_stats) =
        scan_interdia(&res, &tlk, token_reps, &existing_keys)?;
    for l in &companion_lines {
        existing_keys.insert((l.strref, l.dlg_resref.clone(), l.state_index));
    }
    lines.extend(companion_lines);

    let (pdialog_lines, pdialog_speakers, pdialog_stats) =
        scan_pdialog(&res, &tlk, token_reps, &existing_keys)?;
    for l in &pdialog_lines {
        existing_keys.insert((l.strref, l.dlg_resref.clone(), l.state_index));
    }
    lines.extend(pdialog_lines);
    companion_stats.side_dlgs_scanned += pdialog_stats.side_dlgs_scanned;
    companion_stats.side_lines_added += pdialog_stats.side_lines_added;
    companion_stats.rows_unmapped += pdialog_stats.rows_unmapped;

    let main_dlgs: std::collections::HashSet<String> = owned
        .iter()
        .filter_map(|(_, c, _)| {
            c.dialog_resref
                .as_ref()
                .map(|d| d.to_ascii_lowercase())
        })
        .collect();
    let mut excluded_dlgs = main_dlgs;
    excluded_dlgs.extend(interdia_banter_dlg_resrefs(&res)?);
    excluded_dlgs.extend(pdialog_dlg_resrefs(&res)?);

    let (side_lines, side_speakers, side_stats) =
        scan_companion_side_dlgs(&res, &tlk, token_reps, &existing_keys, &excluded_dlgs)?;
    lines.extend(side_lines);
    companion_stats.side_dlgs_scanned += side_stats.side_dlgs_scanned;
    companion_stats.side_lines_added += side_stats.side_lines_added;

    // INTERDIA/pdialog death-variable resolution is stronger identity evidence than a
    // shared display-name strref. Mark every CRE carrying a resolved companion's
    // long name so binding can safely share one voice across transformations and
    // level variants without doing the same for generic same-name NPCs.
    let companion_identity_strrefs: std::collections::HashSet<u32> = companion_speakers
        .iter()
        .chain(pdialog_speakers.iter())
        .chain(side_speakers.iter())
        .filter_map(|speaker| speaker.long_name_strref)
        .collect();

    for cs in companion_speakers
        .into_iter()
        .chain(pdialog_speakers)
        .chain(side_speakers)
    {
        if !speakers.iter().any(|s| s.cre_resref == cs.cre_resref) {
            speakers.push(cs);
        }
    }
    lines.sort_by(|a, b| {
        (a.dlg_resref.as_str(), a.state_index).cmp(&(b.dlg_resref.as_str(), b.state_index))
    });
    for speaker in &mut speakers {
        if speaker
            .long_name_strref
            .is_some_and(|strref| companion_identity_strrefs.contains(&strref))
        {
            let mut provenance: serde_json::Value = serde_json::from_str(&speaker.provenance_json)
                .unwrap_or_else(|_| serde_json::json!({}));
            provenance["verified_voice_identity"] = serde_json::Value::String(format!(
                "companion:{}",
                speaker.long_name_strref.unwrap()
            ));
            speaker.provenance_json = provenance.to_string();
        }
        if speaker.display_name.is_none() {
            if let Some(strref) = speaker.long_name_strref {
                speaker.display_name = tlk
                    .entry(strref)
                    .ok()
                    .map(|e| e.text)
                    .filter(|t| !t.trim().is_empty());
            }
        }
    }
    let groups = attribution::shared_groups(&lines);
    Ok(AttributionScan {
        speakers,
        lines,
        groups,
        companion: companion_stats,
    })
}

/// List installed locales and the resolved active one (prefers `en_US`).
pub fn game_languages(game_dir: &Path) -> Result<GameLanguages, AppError> {
    let locales = lang::list_locales(game_dir);
    let active = lang::resolve_tlk(game_dir, None).ok().map(|p| p.locale);
    Ok(GameLanguages { locales, active })
}

/// Open the active-language `dialog.tlk` and return its header facts.
pub fn tlk_summary(game_dir: &Path, locale: Option<&str>) -> Result<TlkSummary, AppError> {
    let paths = lang::resolve_tlk(game_dir, locale)?;
    let tlk = Tlk::parse(std::fs::read(&paths.dialog)?)?;
    Ok(TlkSummary {
        locale: paths.locale,
        language_id: tlk.language_id,
        entry_count: tlk.count,
    })
}

/// Resolve one TLK strref (text, flags, attached sound resref).
pub fn tlk_entry(
    game_dir: &Path,
    locale: Option<&str>,
    strref: u32,
) -> Result<TlkEntryView, AppError> {
    let paths = lang::resolve_tlk(game_dir, locale)?;
    let tlk = Tlk::parse(std::fs::read(&paths.dialog)?)?;
    Ok(tlk.entry(strref)?.into())
}

/// Resolve and parse a DLG resource (override precedence honored).
pub fn resolve_dialog(game_dir: &Path, resref: &str) -> Result<DlgView, AppError> {
    let res = GameResources::open(game_dir)?;
    let src = res
        .resolve(resref, restype::TYPE_DLG)
        .ok_or_else(|| AppError::Other(format!("DLG {resref:?} not found")))?;
    let origin = src.origin();
    let dlg = dlg::Dlg::parse(&res.read_source(&src)?)?;
    Ok(DlgView::new(resref.to_ascii_lowercase(), origin, &dlg))
}

/// Resolve and parse a CRE resource (override precedence honored).
pub fn resolve_creature(game_dir: &Path, resref: &str) -> Result<CreView, AppError> {
    let res = GameResources::open(game_dir)?;
    let src = res
        .resolve(resref, restype::TYPE_CRE)
        .ok_or_else(|| AppError::Other(format!("CRE {resref:?} not found")))?;
    let origin = src.origin();
    let cre = cre::Cre::parse(&res.read_source(&src)?)?;
    Ok(CreView::new(resref.to_ascii_lowercase(), origin, cre))
}

/// Real-install smoke tests. Ignored by default (they need a full BG2EE tree);
/// run with `cargo test -- --ignored` after pointing `BG2_GAME_DIR` at an install
/// (the default is the local Steam path). They prove the readers parse live,
/// modded data - not just synthetic fixtures.
#[cfg(test)]
mod real_install {
    use super::*;

    const DEFAULT_DIR: &str =
        r"D:\SteamLibrary\steamapps\common\Baldur's Gate II Enhanced Edition";

    fn game_dir() -> std::path::PathBuf {
        std::env::var("BG2_GAME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::path::PathBuf::from(DEFAULT_DIR))
    }

    #[test]
    #[ignore = "requires a real BG2EE install"]
    fn active_tlk_and_locales_parse() {
        let dir = game_dir();
        let langs = game_languages(&dir).unwrap();
        assert!(langs.locales.iter().any(|l| l == "en_US"));
        assert_eq!(langs.active.as_deref(), Some("en_US"));

        let summary = tlk_summary(&dir, None).unwrap();
        assert!(summary.entry_count > 1000, "unexpectedly small dialog.tlk");
        // strref 0 always exists; entry lookup must not panic on real data.
        let entry = tlk_entry(&dir, None, 0).unwrap();
        assert_eq!(entry.strref, 0);
    }

    #[test]
    #[ignore = "requires a real BG2EE install"]
    fn resolves_and_parses_real_dlg_and_cre() {
        let dir = game_dir();
        let res = GameResources::open(&dir).unwrap();

        let dlg_ref = res
            .resrefs_of_type(restype::TYPE_DLG)
            .into_iter()
            .find(|r| resolve_dialog(&dir, r).is_ok())
            .expect("no parseable DLG found in install");
        let dlg = resolve_dialog(&dir, &dlg_ref).unwrap();
        assert!(dlg.state_count > 0 || dlg.transition_count > 0);
        assert!(matches!(dlg.origin.as_str(), "override" | "bif"));

        let cre_ref = res
            .resrefs_of_type(restype::TYPE_CRE)
            .into_iter()
            .find(|r| resolve_creature(&dir, r).is_ok())
            .expect("no parseable CRE found in install");
        let cre = resolve_creature(&dir, &cre_ref).unwrap();
        assert_eq!(cre.version, "V1.0");
        assert!(matches!(cre.origin.as_str(), "override" | "bif"));
    }

    #[test]
    #[ignore = "requires a real BG2EE install"]
    fn attributes_speakers_on_real_install() {
        let dir = game_dir();
        let reps = token_resolve::TokenReplacements::default();
        let scan = scan_attribution(&dir, None, &reps, |_, _| {}, || false).unwrap();
        // A modded BG2EE install has thousands of CRE-owned dialogue lines.
        assert!(!scan.speakers.is_empty(), "no speakers attributed");
        assert!(scan.lines.len() > 100, "suspiciously few attributed lines");
        // Uniquely-owned lines must carry a speaker; every line is classified.
        assert!(scan
            .lines
            .iter()
            .any(|l| l.speaker_cre_resref.is_some() && l.kind == attribution::LineKind::State));
        // Token stand-ins must resolve real tokenized lines (<CHARNAME> etc.).
        assert!(scan.lines.iter().any(|l| l.token_mask != 0 || l.has_tokens));

        // Dedup invariant: every (strref, dlg_resref, state_index) is emitted at
        // most once. The pre-fix scan produced 3,298 duplicate groups; the DLG-keyed
        // pass must now leave zero.
        let mut seen = std::collections::HashSet::new();
        let dup = scan
            .lines
            .iter()
            .find(|l| !seen.insert((l.strref, l.dlg_resref.clone(), l.state_index)));
        assert!(dup.is_none(), "duplicate line-row group emitted: {dup:?}");

        // Representative-owner recovery: the multi-owner `lyros.dlg` (xzar + lyros)
        // must attribute every one of its state lines to a single owner instead of
        // leaving them unattributed/blocked.
        let lyros: Vec<_> = scan
            .lines
            .iter()
            .filter(|l| l.dlg_resref == "lyros" && l.kind == attribution::LineKind::State)
            .collect();
        assert!(!lyros.is_empty(), "lyros.dlg produced no state lines");
        assert!(
            lyros.iter().all(|l| l.speaker_cre_resref.is_some()),
            "lyros.dlg has unattributed state lines"
        );
        assert!(
            scan.companion.lines_added > 0 || scan.companion.dlgs_scanned > 0,
            "interdia.2da companion DLGs should contribute lines on a full install"
        );
    }

    #[test]
    #[ignore = "requires a real BG2EE install"]
    fn companion_side_dlg_includes_jaheiraj_harper_line() {
        let dir = game_dir();
        let reps = token_resolve::TokenReplacements::default();
        let scan = scan_attribution(&dir, None, &reps, |_, _| {}, || false).unwrap();

        let jaheiraj: Vec<_> = scan
            .lines
            .iter()
            .filter(|l| l.dlg_resref == "jaheiraj")
            .collect();
        assert!(!jaheiraj.is_empty(), "jaheiraj.dlg lines missing from scan");

        let harper = jaheiraj.iter().find(|l| l.strref == 49599);
        assert!(harper.is_some(), "strref 49599 (Harper line) missing from jaheiraj");
        let line = harper.unwrap();
        let speaker_ref = line
            .speaker_cre_resref
            .as_deref()
            .expect("side-chain line should have a companion speaker");
        let speaker = scan
            .speakers
            .iter()
            .find(|speaker| speaker.cre_resref == speaker_ref)
            .expect("attributed companion speaker should be present");
        assert_eq!(speaker.display_name.as_deref(), Some("Jaheira"));
        assert_ne!(speaker.display_name.as_deref(), Some("Harper"));
        let tob_banter: Vec<_> = scan
            .lines
            .iter()
            .filter(|candidate| candidate.dlg_resref == "bjahei25")
            .collect();
        assert!(!tob_banter.is_empty(), "bjahei25.dlg lines missing from scan");
        assert!(
            tob_banter
                .iter()
                .all(|candidate| candidate.speaker_cre_resref.as_deref() == Some(speaker_ref)),
            "ToB banter and side-chain dialogue should share Jaheira's identity"
        );
        assert_ne!(line.token_mask, 0, "Harper line should record its resolved token");
        assert!(
            line.provenance_json.contains("companion_side_dlg")
                || line.provenance_json.contains("companion_pdialog"),
            "side-chain or pdialog provenance expected: {}",
            line.provenance_json
        );

        assert!(
            scan.companion.side_dlgs_scanned > 0 || scan.companion.side_lines_added > 0,
            "side-DLG scan stats should be non-zero on a full install"
        );
    }

    #[test]
    #[ignore = "requires a real BG2EE install"]
    fn pdialog_scan_covers_yoshimo_hexxat_wilson_party_files() {
        let dir = game_dir();
        let reps = token_resolve::TokenReplacements::default();
        let scan = scan_attribution(&dir, None, &reps, |_, _| {}, || false).unwrap();

        for need in ["yoshp", "yoshj", "hexxatp", "hexxatj", "wilsonp"] {
            let n = scan
                .lines
                .iter()
                .filter(|line| line.dlg_resref == *need)
                .count();
            assert!(n > 0, "{need}.dlg missing from attribution scan");
        }
        assert!(
            scan.lines.iter().any(|line| line.strref == 22_272),
            "Yoshimo wait-here strref 22272 missing from yoshp"
        );
        let yoshp = scan
            .lines
            .iter()
            .find(|line| line.strref == 22_272)
            .unwrap();
        assert_eq!(yoshp.dlg_resref, "yoshp");
        assert!(
            yoshp.provenance_json.contains("companion_pdialog"),
            "expected pdialog provenance: {}",
            yoshp.provenance_json
        );
    }
}
