//! Pure reference-sample candidate discovery (item-07).
//!
//! Given a speaker and the voiced dialogue lines attributed to it (item-06), plus
//! the speaker's creature sound-slot strrefs, decide which original clips are
//! usable voice references and how much to trust each. This module is PURE: it
//! operates over already-resolved facts (no filesystem, no ffmpeg), so the
//! selection policy is fully fixture-testable. The IO layer (`voices::harvest`)
//! resolves + decodes the winners.
//!
//! Copyright note: a candidate names an ORIGINAL source (strref + sound resref)
//! only so the IO layer can produce a LOCAL derivative; the original bytes are
//! never carried here, persisted, or exported (see `00-context.md`).

use super::reference_text;

/// Max automatic companion-dialogue candidates decoded per speaker (after text gate).
pub const MAX_AUTOMATIC_COMPANION_CANDIDATES: usize = 12;
/// Max Attribution gap-fill candidates decoded per thin speaker (after text gate).
pub const MAX_AUTOMATIC_GAP_FILL_CANDIDATES: usize = 8;

/// How a candidate clip was discovered, in descending trust order. Mirrors the
/// `origin` token embedded in a sample's provenance JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateOrigin {
    /// CRE -> its DLG -> an actor state it uniquely owns -> a voiced TLK strref
    /// with an attached sound resref. The strongest proof of an NPC's own voice.
    DialogueState,
    /// Uniquely attributed official VO from Attribution rows (gap-fill for thin speakers).
    AttributionVoiced,
    /// Companion interdia / pdialog / side-chain DLG actor state with attached sound.
    CompanionDialogue,
    /// A voiced strref referenced by the creature's SNDSLOT.IDS sound slots.
    /// Secondary: soundset lines are often barks, not conversation.
    SoundSlot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateEligibility {
    Automatic,
    ManualOnly,
}

impl CandidateEligibility {
    pub fn token(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::ManualOnly => "manual_only",
        }
    }
}

impl CandidateOrigin {
    /// Lowercase provenance token.
    pub fn token(self) -> &'static str {
        match self {
            CandidateOrigin::DialogueState => "dialogue_state",
            CandidateOrigin::AttributionVoiced => "attribution_voiced",
            CandidateOrigin::CompanionDialogue => "companion_dialogue",
            CandidateOrigin::SoundSlot => "sound_slot",
        }
    }

    /// Baseline trust weight `[0,1]` contributed by the discovery path alone.
    pub fn provenance_weight(self) -> f64 {
        match self {
            CandidateOrigin::DialogueState => 1.0,
            CandidateOrigin::AttributionVoiced => 0.9,
            CandidateOrigin::CompanionDialogue => 0.85,
            CandidateOrigin::SoundSlot => 0.6,
        }
    }
}

/// One resolved reference-clip candidate for a speaker, before decode/scoring.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// The TLK strref the clip's text/sound came from.
    pub strref: u32,
    /// The attached sound resref (lowercased) to resolve + decode.
    pub sound_resref: String,
    /// Canonical TLK transcript for the strref (used for text gating + scoring).
    pub source_text: String,
    /// How this candidate was found.
    pub origin: CandidateOrigin,
    /// The attribution confidence of the owning line (`0.0` for sound-slot-only).
    pub attribution_confidence: f64,
    pub eligibility: CandidateEligibility,
    pub shared_source_count: usize,
}

/// A voiced line attributed to the speaker, as the harvest layer resolved it.
/// Only uniquely-attributed states are passed in; ambiguous ones are excluded by
/// the caller so a shared clip is never mistaken for one NPC's voice.
#[derive(Debug, Clone)]
pub struct VoicedLine {
    pub strref: u32,
    pub sound_resref: String,
    pub source_text: String,
    pub attribution_confidence: f64,
}

/// A voiced creature sound-slot entry (strref + its resolved sound resref).
#[derive(Debug, Clone)]
pub struct SlotSound {
    pub strref: u32,
    pub sound_resref: String,
    pub source_text: String,
}

/// Select the candidate set for a speaker, de-duplicating by sound resref and
/// preferring the higher-trust origin when the same clip appears via multiple paths.
///
/// Order: unique main dialogue → companion dialogue (quality-capped) → sound slots.
/// Candidates whose TLK text fails [`reference_text::is_usable_reference_text`]
/// are dropped before decode.
pub fn select(
    voiced: &[VoicedLine],
    companion: &[VoicedLine],
    slots: &[SlotSound],
) -> Vec<Candidate> {
    let mut out = select_dialogue(voiced, CandidateOrigin::DialogueState);
    let mut seen = out
        .iter()
        .map(|c| c.sound_resref.clone())
        .collect::<std::collections::HashSet<_>>();
    for candidate in select_companion(companion) {
        if seen.insert(candidate.sound_resref.clone()) {
            out.push(candidate);
        }
    }
    for candidate in select_slots(slots) {
        if seen.insert(candidate.sound_resref.clone()) {
            out.push(candidate);
        }
    }
    out
}

fn select_dialogue(voiced: &[VoicedLine], origin: CandidateOrigin) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    let mut states: Vec<&VoicedLine> = voiced.iter().collect();
    states.sort_by(|a, b| {
        b.attribution_confidence
            .partial_cmp(&a.attribution_confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.strref.cmp(&b.strref))
    });
    for l in states {
        if !reference_text::is_usable_reference_text(&l.source_text) {
            continue;
        }
        let resref = l.sound_resref.to_ascii_lowercase();
        if resref.is_empty() || !seen.insert(resref.clone()) {
            continue;
        }
        out.push(Candidate {
            strref: l.strref,
            sound_resref: resref,
            source_text: l.source_text.clone(),
            origin,
            attribution_confidence: l.attribution_confidence,
            eligibility: CandidateEligibility::Automatic,
            shared_source_count: 1,
        });
    }
    out
}

fn companion_quality(text: &str) -> f64 {
    reference_text::text_richness_score(text) * reference_text::ordinary_speech_score(text)
}

fn select_companion(companion: &[VoicedLine]) -> Vec<Candidate> {
    let mut usable: Vec<&VoicedLine> = companion
        .iter()
        .filter(|l| reference_text::is_usable_reference_text(&l.source_text))
        .filter(|l| !l.sound_resref.trim().is_empty())
        .collect();
    usable.sort_by(|a, b| {
        companion_quality(&b.source_text)
            .partial_cmp(&companion_quality(&a.source_text))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.strref.cmp(&b.strref))
    });

    let mut out: Vec<Candidate> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for l in usable {
        if out.len() >= MAX_AUTOMATIC_COMPANION_CANDIDATES {
            break;
        }
        let resref = l.sound_resref.to_ascii_lowercase();
        if !seen.insert(resref.clone()) {
            continue;
        }
        out.push(Candidate {
            strref: l.strref,
            sound_resref: resref,
            source_text: l.source_text.clone(),
            origin: CandidateOrigin::CompanionDialogue,
            attribution_confidence: l.attribution_confidence,
            eligibility: CandidateEligibility::Automatic,
            shared_source_count: 1,
        });
    }
    out
}

fn select_slots(slots: &[SlotSound]) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for s in slots {
        if !reference_text::is_usable_reference_text(&s.source_text) {
            continue;
        }
        let resref = s.sound_resref.to_ascii_lowercase();
        if resref.is_empty() || !seen.insert(resref.clone()) {
            continue;
        }
        out.push(Candidate {
            strref: s.strref,
            sound_resref: resref,
            source_text: s.source_text.clone(),
            origin: CandidateOrigin::SoundSlot,
            attribution_confidence: 0.0,
            eligibility: CandidateEligibility::ManualOnly,
            shared_source_count: 1,
        });
    }
    out
}

/// Select gap-fill candidates from uniquely attributed official VO lines.
/// `already` sounds (live harvest jobs or existing DB keys) are skipped; at most
/// [`MAX_AUTOMATIC_GAP_FILL_CANDIDATES`] remain after the text gate and quality rank.
pub fn select_gap_fill(
    lines: &[VoicedLine],
    already: &std::collections::HashSet<String>,
) -> Vec<Candidate> {
    let mut usable: Vec<&VoicedLine> = lines
        .iter()
        .filter(|l| reference_text::is_usable_reference_text(&l.source_text))
        .filter(|l| !l.sound_resref.trim().is_empty())
        .collect();
    usable.sort_by(|a, b| {
        companion_quality(&b.source_text)
            .partial_cmp(&companion_quality(&a.source_text))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.strref.cmp(&b.strref))
    });

    let mut out: Vec<Candidate> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for l in usable {
        if out.len() >= MAX_AUTOMATIC_GAP_FILL_CANDIDATES {
            break;
        }
        let resref = l.sound_resref.to_ascii_lowercase();
        if already.contains(&resref) || !seen.insert(resref.clone()) {
            continue;
        }
        out.push(Candidate {
            strref: l.strref,
            sound_resref: resref,
            source_text: l.source_text.clone(),
            origin: CandidateOrigin::AttributionVoiced,
            attribution_confidence: l.attribution_confidence,
            eligibility: CandidateEligibility::Automatic,
            shared_source_count: 1,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vl(strref: u32, resref: &str, text: &str, conf: f64) -> VoicedLine {
        VoicedLine {
            strref,
            sound_resref: resref.into(),
            source_text: text.into(),
            attribution_confidence: conf,
        }
    }

    fn slot(strref: u32, resref: &str, text: &str) -> SlotSound {
        SlotSound {
            strref,
            sound_resref: resref.into(),
            source_text: text.into(),
        }
    }

    #[test]
    fn dialogue_states_precede_sound_slots_and_sort_by_confidence() {
        let voiced = vec![
            vl(20, "xzar02", "I have much to teach you.", 0.4),
            vl(10, "xzar01", "Necromancy is my art.", 1.0),
        ];
        let slots = vec![slot(30, "xzarbark", "Heh!")];
        let got = select(&voiced, &[], &slots);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].sound_resref, "xzar01");
        assert_eq!(got[0].origin, CandidateOrigin::DialogueState);
        assert_eq!(got[1].origin, CandidateOrigin::DialogueState);
    }

    #[test]
    fn sound_slots_are_fallback_when_no_usable_dialogue() {
        let voiced = vec![vl(10, "xzar01", "Argh!", 1.0)];
        let slots = vec![slot(30, "xzarbark", "Well met, traveler.")];
        let got = select(&voiced, &[], &slots);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].origin, CandidateOrigin::SoundSlot);
    }

    #[test]
    fn retains_sound_slots_when_dialogue_exists() {
        let voiced = vec![vl(10, "xzar01", "Necromancy is my art.", 1.0)];
        let slots = vec![slot(30, "xzarbark", "Well met, traveler.")];
        let got = select(&voiced, &[], &slots);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].origin, CandidateOrigin::DialogueState);
        assert_eq!(got[1].eligibility, CandidateEligibility::ManualOnly);
    }

    #[test]
    fn dedupes_by_resref_preferring_dialogue_state() {
        let voiced = vec![vl(10, "SHARED", "A line of real dialogue here.", 1.0)];
        let slots = vec![slot(30, "shared", "Another line of dialogue.")];
        let got = select(&voiced, &[], &slots);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].origin, CandidateOrigin::DialogueState);
    }

    #[test]
    fn companion_follows_main_dialogue_and_precedes_slots() {
        let voiced = vec![vl(10, "main01", "I speak through my own dialogue file.", 1.0)];
        let companion = vec![vl(
            20,
            "banter01",
            "This banter line is rich enough for cloning references.",
            1.0,
        )];
        let slots = vec![slot(30, "bark01", "Well met, traveler.")];
        let got = select(&voiced, &companion, &slots);
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].origin, CandidateOrigin::DialogueState);
        assert_eq!(got[1].origin, CandidateOrigin::CompanionDialogue);
        assert_eq!(got[2].origin, CandidateOrigin::SoundSlot);
        assert!((CandidateOrigin::CompanionDialogue.provenance_weight() - 0.85).abs() < 1e-9);
    }

    #[test]
    fn dedupes_companion_preferring_main_dialogue() {
        let voiced = vec![vl(10, "shared", "A line of real dialogue here.", 1.0)];
        let companion = vec![vl(20, "SHARED", "Companion copy of the same sound clip.", 1.0)];
        let got = select(&voiced, &companion, &[]);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].origin, CandidateOrigin::DialogueState);
    }

    #[test]
    fn caps_companion_candidates_at_max_automatic() {
        let companion: Vec<VoicedLine> = (0..20)
            .map(|i| {
                vl(
                    i,
                    &format!("cmp{i:02}"),
                    &format!("Companion spoken line number {i} with enough lexical content."),
                    1.0,
                )
            })
            .collect();
        let got = select(&[], &companion, &[]);
        assert_eq!(got.len(), MAX_AUTOMATIC_COMPANION_CANDIDATES);
        assert!(got
            .iter()
            .all(|c| c.origin == CandidateOrigin::CompanionDialogue));
    }

    #[test]
    fn drops_blank_resrefs_and_non_lexical_text() {
        assert!(select(&[vl(1, "", "Hello there.", 1.0)], &[], &[]).is_empty());
        assert!(select(&[vl(1, "a01", "[grunt]", 1.0)], &[], &[]).is_empty());
    }

    #[test]
    fn gap_fill_caps_and_skips_already_present_sounds() {
        let lines: Vec<VoicedLine> = (0..12)
            .map(|i| {
                vl(
                    i,
                    &format!("gap{i:02}"),
                    &format!("Gap fill spoken line number {i} with enough lexical content."),
                    1.0,
                )
            })
            .collect();
        let already = std::collections::HashSet::from(["gap00".into()]);
        let got = select_gap_fill(&lines, &already);
        assert_eq!(got.len(), MAX_AUTOMATIC_GAP_FILL_CANDIDATES);
        assert!(got
            .iter()
            .all(|c| c.origin == CandidateOrigin::AttributionVoiced));
        assert!(!got.iter().any(|c| c.sound_resref == "gap00"));
        assert!((CandidateOrigin::AttributionVoiced.provenance_weight() - 0.9).abs() < 1e-9);
    }
}
