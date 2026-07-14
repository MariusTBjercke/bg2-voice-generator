//! Pure export eligibility + `PackPlan` assembly (item-09).
//!
//! Given the candidate rows the DB layer joined (each a line + its completed
//! generation + the speaker/clone it was cloned from), decide which lines are SAFE
//! to patch and assign each a unique staged resref. Every deferred category from
//! `00-context.md` is blocked here with a recorded reason, so the caller can never
//! silently ship a tokenized/transition/script/shared-diff line. PURE (no IO): the
//! caller supplies the set of resrefs already present in the target so collisions
//! are avoided without this module touching the filesystem.

use std::collections::HashSet;

use crate::error::AppError;
use crate::export::resref::is_pack_generated_resref;
use crate::fingerprint::PackFingerprintInputs;
use crate::models::LineKind;

use super::manifest::{sha256_hex, DeferredLine, PackFingerprint, PackLine};
use super::resref::resref_for;

/// One candidate the DB layer produced for a project: a line joined to its `done`
/// generation and the speaker it was cloned from. `clip_on_disk` is the caller's
/// verdict on whether the generation's `output_path` still exists (resume rule).
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub line_id: i64,
    pub strref: i64,
    /// Text sent to the voice engine. This may contain placeholder stand-ins such
    /// as `Hero` so the generated clip can pronounce a dynamic token.
    pub text: String,
    /// Raw TLK text before placeholder resolution. When present, this is the text
    /// WeiDU writes back while attaching the generated sound, preserving the game's
    /// manuscript and its runtime tokens.
    pub original_text: String,
    pub kind: LineKind,
    pub is_voiced: bool,
    pub has_tokens: bool,
    pub existing_sound_resref: Option<String>,
    pub speaker_id: Option<i64>,
    pub shared_group_id: Option<i64>,
    /// `true` iff this strref's shared group resolved to `defer_diff_voice`.
    pub shared_deferred: bool,
    pub speaker_resref: String,
    pub binding_source: String,
    /// The clip was rendered from a different reference than the current binding.
    pub voice_changed: bool,
    /// Absolute path to the generated clip (the generation `output_path`).
    pub audio_source_path: String,
    pub clip_on_disk: bool,
}

/// A ready-to-write pack: the eligible lines (each with a staged resref + source
/// audio) and the deferred lines with reasons. `fingerprint` describes the target.
#[derive(Debug, Clone, PartialEq)]
pub struct PackPlan {
    pub pack_name: String,
    pub fingerprint: PackFingerprint,
    pub lines: Vec<PlannedLine>,
    pub deferred: Vec<DeferredLine>,
}

/// An eligible line plus the on-disk source of its generated clip (staged by
/// `build`). `entry` is the manifest/tp2 view; `audio_source_path` is IO-only.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedLine {
    pub entry: PackLine,
    pub audio_source_path: String,
}

/// Whether WeiDU can `STRING_SET` this line on the target install. The patched
/// index is the line's `strref` and must fall within `dialog.tlk`.
fn strref_is_patchable(strref: i64, tlk_entry_count: u32) -> bool {
    strref >= 0 && (strref as u64) < tlk_entry_count as u64
}

/// Why a candidate was excluded (mirrors the deferred categories). Returns `None`
/// when the candidate IS eligible.
fn defer_reason(c: &Candidate, fp: &PackFingerprintInputs) -> Option<&'static str> {
    if c.kind != LineKind::State {
        return Some("not an NPC dialogue state (transition/script/token kind)");
    }
    if c.has_tokens {
        return Some("line has dynamic tokens (<PRO_*>/<CHARNAME>/...)");
    }
    if !crate::extractor::spoken_text::has_speakable_dialogue(&c.text) {
        return Some("line is intentionally silent (no speakable dialogue text)");
    }
    if c.is_voiced
        && !c
            .existing_sound_resref
            .as_deref()
            .map(is_pack_generated_resref)
            .unwrap_or(false)
    {
        return Some("line is already voiced in the target");
    }
    if c.speaker_id.is_none() {
        return Some("line has no uniquely attributed speaker");
    }
    if c.shared_deferred {
        return Some("shared strref with a different/unknown voice (deferred)");
    }
    if !c.clip_on_disk {
        return Some("generated clip is missing on disk");
    }
    if !strref_is_patchable(c.strref, fp.tlk_entry_count) {
        return Some("strref is not present in the target dialog.tlk");
    }
    None
}

/// Assemble the plan. `pack_name` names the WeiDU folder; `fp_inputs` describes the
/// target; `existing_resrefs` are names already present in the target `override/`/
/// BIFs (uppercased) so staged names never collide.
pub fn assemble(
    pack_name: &str,
    fp_inputs: &PackFingerprintInputs,
    existing_resrefs: &HashSet<String>,
    candidates: &[Candidate],
) -> Result<PackPlan, AppError> {
    let mut taken: HashSet<String> = existing_resrefs.iter().map(|r| r.to_ascii_uppercase()).collect();
    let mut lines = Vec::new();
    let mut deferred = Vec::new();

    for c in candidates {
        if let Some(reason) = defer_reason(c, fp_inputs) {
            deferred.push(DeferredLine {
                line_id: c.line_id,
                strref: c.strref,
                reason: reason.to_string(),
            });
            continue;
        }
        let resref = resref_for(c.strref, &mut taken)?;
        // `text` is generation-only when token stand-ins have changed it. STRING_SET
        // must receive the raw TLK text so it updates only the sound reference from a
        // player's perspective, not the dialogue they see or the tokens the game resolves.
        let subtitle_text = if c.original_text.is_empty() {
            &c.text
        } else {
            &c.original_text
        };
        lines.push(PlannedLine {
            entry: PackLine {
                line_id: c.line_id,
                strref: c.strref,
                resref,
                text: subtitle_text.clone(),
                text_sha256: sha256_hex(subtitle_text.as_bytes()),
                speaker_resref: c.speaker_resref.clone(),
                binding_source: c.binding_source.clone(),
            },
            audio_source_path: c.audio_source_path.clone(),
        });
    }

    Ok(PackPlan {
        pack_name: pack_name.to_string(),
        fingerprint: PackFingerprint {
            edition: fp_inputs.edition.clone(),
            edition_version: fp_inputs.edition_version.clone(),
            language: fp_inputs.language.clone(),
            mod_state_hash: fp_inputs.mod_state_hash.clone(),
            tlk_entry_count: fp_inputs.tlk_entry_count,
        },
        lines,
        deferred,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp() -> PackFingerprintInputs {
        PackFingerprintInputs {
            edition: "bg2ee".into(),
            edition_version: "2.6".into(),
            language: "en_US".into(),
            mod_state_hash: "hash".into(),
            tlk_entry_count: 200_000,
        }
    }

    fn base(line_id: i64, strref: i64) -> Candidate {
        Candidate {
            line_id,
            strref,
            text: "Hello.".into(),
            original_text: String::new(),
            kind: LineKind::State,
            is_voiced: false,
            has_tokens: false,
            existing_sound_resref: None,
            speaker_id: Some(1),
            shared_group_id: None,
            shared_deferred: false,
            speaker_resref: "XZAR".into(),
            binding_source: "default".into(),
            voice_changed: false,
            audio_source_path: "/ws/1.wav".into(),
            clip_on_disk: true,
        }
    }

    #[test]
    fn eligible_line_gets_a_unique_resref_and_text_hash() {
        let plan = assemble("P", &fp(), &HashSet::new(), &[base(1, 22570)]).unwrap();
        assert_eq!(plan.lines.len(), 1);
        assert!(plan.deferred.is_empty());
        let l = &plan.lines[0].entry;
        assert_eq!(l.strref, 22570);
        assert_eq!(l.text_sha256, sha256_hex(b"Hello."));
        assert_eq!(l.resref.len(), 8);
    }

    #[test]
    fn token_standin_is_generation_only_and_original_text_is_exported() {
        let mut candidate = base(1, 22570);
        candidate.text = "Hello Hero, welcome.".into();
        candidate.original_text = "Hello <CHARNAME>, welcome.".into();

        let plan = assemble("P", &fp(), &HashSet::new(), &[candidate]).unwrap();
        let line = &plan.lines[0].entry;
        assert_eq!(line.text, "Hello <CHARNAME>, welcome.");
        assert_eq!(line.text_sha256, sha256_hex(b"Hello <CHARNAME>, welcome."));
    }

    #[test]
    fn each_deferred_category_is_blocked_with_a_reason() {
        let mut token = base(1, 1);
        token.has_tokens = true;
        let mut trans = base(2, 2);
        trans.kind = LineKind::Transition;
        let mut voiced = base(3, 3);
        voiced.is_voiced = true;
        voiced.existing_sound_resref = Some("XZAR01".into());
        let mut pack_voiced = base(7, 7);
        pack_voiced.is_voiced = true;
        pack_voiced.existing_sound_resref = Some("Z0000700".into());
        let mut unattr = base(4, 4);
        unattr.speaker_id = None;
        let mut shared = base(5, 5);
        shared.shared_deferred = true;
        let mut missing = base(6, 6);
        missing.clip_on_disk = false;
        let mut silent = base(8, 8);
        silent.text = "...".into();
        let plan = assemble("P", &fp(), &HashSet::new(), &[token, trans, voiced, unattr, shared, missing, silent, pack_voiced]).unwrap();
        assert_eq!(plan.lines.len(), 1);
        assert_eq!(plan.lines[0].entry.strref, 7);
        assert_eq!(plan.deferred.len(), 7, "official-voiced + six other unsafe cases deferred");
        assert!(plan.deferred.iter().any(|line| {
            line.strref == 8 && line.reason.contains("intentionally silent")
        }));
    }

    #[test]
    fn out_of_range_strref_is_deferred() {
        let mut fp = fp();
        fp.tlk_entry_count = 103_778;
        let plan = assemble(
            "P",
            &fp,
            &HashSet::new(),
            &[base(1, 22570), base(2, 199_845)],
        )
        .unwrap();
        assert_eq!(plan.lines.len(), 1);
        assert_eq!(plan.lines[0].entry.strref, 22570);
        assert_eq!(plan.deferred.len(), 1);
        assert_eq!(plan.deferred[0].strref, 199_845);
        assert!(plan.deferred[0]
            .reason
            .contains("not present in the target dialog.tlk"));
    }

    #[test]
    fn staged_names_avoid_existing_and_each_other() {
        let mut existing = HashSet::new();
        // Seed with the deterministic first choice for strref 7 so it must skip it.
        let determined = {
            let mut probe = HashSet::new();
            resref_for(7, &mut probe).unwrap()
        };
        existing.insert(determined.clone());
        let plan = assemble("P", &fp(), &existing, &[base(1, 7), base(2, 7)]).unwrap();
        let names: HashSet<_> = plan.lines.iter().map(|l| l.entry.resref.clone()).collect();
        assert_eq!(names.len(), 2, "two lines get two distinct resrefs");
        assert!(!names.contains(&determined), "must avoid the pre-existing resref");
    }
}
