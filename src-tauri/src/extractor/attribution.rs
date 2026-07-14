//! Speaker attribution (item-06): reconcile CRE creatures with the DLG actor
//! states they own to decide *who speaks each voiceable line*, with a confidence
//! score and machine-readable provenance, then classify each line's kind
//! (voiceable state vs deferred transition/script/token) and group shared strrefs.
//!
//! The attribution path is CRE -> its `dialog_resref` -> DLG actor **state** ->
//! TLK strref (-> optional attached sound resref). Player **transitions** and
//! script-displayed text are NOT actor states and are excluded from voicing.
//!
//! This module is PURE: it operates over already-parsed [`Cre`]/[`Dlg`] values
//! and a text/voiced lookup closure, so it is fully fixture-testable with no game
//! install. The IO orchestration that resolves resources from an install and
//! persists the result lives in the command layer + `db`.

use std::collections::BTreeMap;

use super::cre::Cre;
use super::dlg::Dlg;
use super::token_resolve::{self, TokenReplacements};
use super::tokens;

/// The confidence assigned when exactly one CRE owns a DLG (unambiguous).
pub const CONFIDENCE_UNIQUE: f64 = 1.0;
/// The confidence assigned when several CREs share one DLG (ambiguous owner).
pub const CONFIDENCE_AMBIGUOUS: f64 = 0.4;
/// The confidence for a line we could not attribute to any speaker.
pub const CONFIDENCE_NONE: f64 = 0.0;
/// The confidence for a line recovered by attributing a multi-owner DLG to a
/// single representative owner (permissive attribution). Lower than
/// [`CONFIDENCE_AMBIGUOUS`] so these are always distinguishable from an exact,
/// single-owner attribution while still being generatable.
pub const CONFIDENCE_PERMISSIVE: f64 = 0.2;

/// How a voiceable line was classified. Mirrors `models::LineKind` tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// An NPC actor response state - the voiceable case.
    State,
    /// A player choice transition - deferred (not an NPC voice line).
    Transition,
    /// A tokenized string (`<PRO_*>`/`<CHARNAME>`/...) - deferred, unsafe to voice.
    Token,
}

impl LineKind {
    /// The serde/DB token (kept in lockstep with `models::LineKind`).
    pub fn token(self) -> &'static str {
        match self {
            LineKind::State => "state",
            LineKind::Transition => "transition",
            LineKind::Token => "token",
        }
    }
}

/// A creature attributed as a speaker, carrying the factual metadata the CRE
/// reader produced plus its owned dialogue resref and an attribution confidence.
#[derive(Debug, Clone, PartialEq)]
pub struct AttributedSpeaker {
    pub cre_resref: String,
    pub dialogue_resref: Option<String>,
    pub sex: u8,
    pub race: u8,
    pub class: u8,
    pub kit: u32,
    pub alignment: u8,
    pub creature_category: u8,
    pub long_name_strref: Option<u32>,
    /// Resolved from TLK at scan time when `long_name_strref` is set.
    pub display_name: Option<String>,
    /// `1.0` when this CRE uniquely owns its DLG, lower when the DLG is shared.
    pub confidence: f64,
    /// JSON provenance describing how the attribution was derived.
    pub provenance_json: String,
}

/// One classified dialogue line with its (best) attributed speaker.
#[derive(Debug, Clone, PartialEq)]
pub struct AttributedLine {
    pub strref: u32,
    pub dlg_resref: String,
    pub state_index: u32,
    pub text: String,
    /// Raw TLK when token stand-ins changed `text`; empty when never tokenized.
    pub original_text: String,
    pub kind: LineKind,
    pub is_voiced: bool,
    /// TLK sound resref when the line already has attached audio (official or pack).
    pub existing_sound_resref: Option<String>,
    pub has_tokens: bool,
    pub token_mask: i64,
    /// `Some(cre_resref)` when a single owner CRE was found, else `None`.
    pub speaker_cre_resref: Option<String>,
    pub confidence: f64,
    pub provenance_json: String,
}

/// A CRE paired with the resolved dialogue it owns.
pub struct CreDialog<'a> {
    pub cre_resref: String,
    pub cre: &'a Cre,
    pub dlg: &'a Dlg,
}

/// Facts a TLK lookup must surface for a strref during attribution.
#[derive(Debug, Clone, Default)]
pub struct StrrefFacts {
    /// The line already has an attached sound resref (already voiced).
    pub is_voiced: bool,
    /// The attached sound resref, when [`is_voiced`] is true.
    pub sound_resref: Option<String>,
}

/// Attribute every actor state across `inputs` to a speaker.
///
/// `text_of` returns the TLK text for a strref (empty string when unknown);
/// `facts_of` returns per-strref facts (voiced state); `owner_has_voice` reports
/// whether an owner CRE already has a voice (a ready clone / approved reference
/// sample) - a tiebreak signal when a DLG has several owners. All are closures so
/// the caller controls resolution and this stays pure/testable.
///
/// Returns `(speakers, lines)`. Each DLG's states are emitted **once**, regardless
/// of how many CREs reference it (a DLG shared by N CREs must not yield N duplicate
/// line groups). A uniquely-owned DLG keeps [`CONFIDENCE_UNIQUE`]; a multi-owner
/// DLG is attributed to a single deterministic representative owner at
/// [`CONFIDENCE_PERMISSIVE`] (permissive attribution), recording the chosen +
/// rejected owners in the line's provenance.
pub fn attribute<TextFn, FactsFn, VoiceFn>(
    inputs: &[CreDialog<'_>],
    mut text_of: TextFn,
    mut facts_of: FactsFn,
    owner_has_voice: VoiceFn,
    token_reps: &TokenReplacements,
) -> (Vec<AttributedSpeaker>, Vec<AttributedLine>)
where
    TextFn: FnMut(u32) -> String,
    FactsFn: FnMut(u32) -> StrrefFacts,
    VoiceFn: Fn(&str) -> bool,
{
    // Group CRE indices by the (lowercased) dialogue resref they own, so each DLG
    // is walked ONCE with its full owner set - this is what de-duplicates the
    // per-CRE emission that previously produced N copies of a shared DLG's lines.
    let mut by_dlg: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, cd) in inputs.iter().enumerate() {
        if let Some(dlg) = cd.cre.dialog_resref.as_ref() {
            by_dlg.entry(dlg.to_ascii_lowercase()).or_default().push(i);
        }
    }

    // One speaker per input CRE; confidence still reflects whether its DLG is
    // uniquely owned (a shared DLG makes each co-owner an ambiguous speaker).
    let mut speakers = Vec::with_capacity(inputs.len());
    for cd in inputs {
        let owner_count = cd
            .cre
            .dialog_resref
            .as_ref()
            .and_then(|d| by_dlg.get(&d.to_ascii_lowercase()))
            .map(Vec::len)
            .unwrap_or(0);
        let confidence = if cd.cre.dialog_resref.is_none() {
            CONFIDENCE_NONE
        } else if owner_count <= 1 {
            CONFIDENCE_UNIQUE
        } else {
            CONFIDENCE_AMBIGUOUS
        };
        speakers.push(speaker_of(cd, confidence, owner_count));
    }

    // Emit each DLG's actor-state lines once, attributed to a single representative
    // owner. Uniquely-owned DLGs keep full confidence; a multi-owner DLG picks a
    // deterministic winner (permissive) at low confidence and records the decision.
    let mut lines = Vec::new();
    for (dlg_lc, idxs) in &by_dlg {
        let unique = idxs.len() <= 1;
        let winner = choose_representative(inputs, idxs, dlg_lc, &owner_has_voice);
        let winner_cd = &inputs[winner];
        let confidence = if unique { CONFIDENCE_UNIQUE } else { CONFIDENCE_PERMISSIVE };
        let rejected: Vec<String> = idxs
            .iter()
            .filter(|&&i| i != winner)
            .map(|&i| inputs[i].cre_resref.to_ascii_lowercase())
            .collect();

        for state in &winner_cd.dlg.states {
            if let Some(strref) = state.text_strref {
                let text = text_of(strref);
                let facts = facts_of(strref);
                lines.push(line_of(
                    winner_cd,
                    dlg_lc,
                    state.index,
                    strref,
                    text,
                    facts,
                    unique,
                    confidence,
                    &rejected,
                    token_reps,
                ));
            }
        }
    }

    lines.sort_by(|a, b| {
        (a.dlg_resref.as_str(), a.state_index).cmp(&(b.dlg_resref.as_str(), b.state_index))
    });
    (speakers, lines)
}

/// Pick the representative owner (an index into `inputs`) for a DLG shared by the
/// owners in `idxs`. Deterministic tiebreak, highest signal first: (1) the CRE
/// resref equals the DLG resref (e.g. `lyros.cre` <-> `lyros.dlg`), (2) the CRE
/// has a long name (a named character vs a generic one), (3) the owner already has
/// a voice (ready clone / approved sample), then (4) the lexically-smallest
/// `cre_resref` for a stable result. (Co-owners of one DLG share the same
/// attribution confidence, so it cannot differentiate and is omitted here.)
fn choose_representative(
    inputs: &[CreDialog<'_>],
    idxs: &[usize],
    dlg_lc: &str,
    owner_has_voice: &impl Fn(&str) -> bool,
) -> usize {
    *idxs
        .iter()
        .max_by(|&&a, &&b| {
            rep_key(inputs, a, dlg_lc, owner_has_voice)
                .cmp(&rep_key(inputs, b, dlg_lc, owner_has_voice))
        })
        .expect("a DLG owner group always has at least one member")
}

/// The comparable tiebreak key for one owner (greatest wins). The three booleans
/// prefer `true`; the trailing [`std::cmp::Reverse`] makes the lexically-smallest
/// `cre_resref` sort greatest, so ties resolve to a stable, predictable winner.
fn rep_key(
    inputs: &[CreDialog<'_>],
    i: usize,
    dlg_lc: &str,
    owner_has_voice: &impl Fn(&str) -> bool,
) -> (bool, bool, bool, std::cmp::Reverse<String>) {
    let cre_lc = inputs[i].cre_resref.to_ascii_lowercase();
    (
        cre_lc == dlg_lc,
        inputs[i].cre.long_name_strref.is_some(),
        owner_has_voice(&cre_lc),
        std::cmp::Reverse(cre_lc),
    )
}

/// Build the [`AttributedSpeaker`] for one CRE/dialogue pairing.
fn speaker_of(cd: &CreDialog<'_>, confidence: f64, owner_count: usize) -> AttributedSpeaker {
    let dialogue_resref = cd.cre.dialog_resref.as_ref().map(|d| d.to_ascii_lowercase());
    let provenance = serde_json::json!({
        "method": "cre_dialog_owner",
        "cre_resref": cd.cre_resref.to_ascii_lowercase(),
        "dialogue_resref": dialogue_resref,
        "dlg_owner_count": owner_count,
    });
    AttributedSpeaker {
        cre_resref: cd.cre_resref.to_ascii_lowercase(),
        dialogue_resref,
        sex: cd.cre.sex,
        race: cd.cre.race,
        class: cd.cre.class,
        kit: cd.cre.kit,
        alignment: cd.cre.alignment,
        creature_category: cd.cre.general,
        long_name_strref: cd.cre.long_name_strref,
        display_name: None,
        confidence,
        provenance_json: provenance.to_string(),
    }
}

/// Classify + attribute one actor-state line to the representative owner `cd`.
pub(crate) fn line_of(
    cd: &CreDialog<'_>,
    dlg_resref: &str,
    state_index: u32,
    strref: u32,
    raw_text: String,
    facts: StrrefFacts,
    unique: bool,
    confidence: f64,
    rejected_owners: &[String],
    token_reps: &TokenReplacements,
) -> AttributedLine {
    let dlg_resref = dlg_resref.to_ascii_lowercase();
    let owner_cre = cd.cre_resref.to_ascii_lowercase();
    let raw_has_tokens = tokens::has_dynamic_token(&raw_text);
    let found_tokens: Vec<String> = tokens::tokens_in(&raw_text).map(str::to_string).collect();

    let (text, original_text, kind, has_tokens, token_mask) = if raw_has_tokens {
        let resolved = token_resolve::resolve_tokens(&raw_text, token_reps);
        if resolved.unresolved.is_empty() {
            (
                resolved.spoken,
                raw_text,
                LineKind::State,
                false,
                resolved.mask,
            )
        } else {
            (
                raw_text.clone(),
                String::new(),
                LineKind::Token,
                true,
                resolved.mask,
            )
        }
    } else {
        (raw_text.clone(), String::new(), LineKind::State, false, 0)
    };
    // Uniquely-owned lines keep the plain `dlg_state` provenance; a multi-owner DLG
    // records the permissive decision (chosen + rejected owners) so a recovered line
    // is never confused with an exact, single-owner attribution.
    let provenance = if unique {
        serde_json::json!({
            "method": "dlg_state",
            "dlg_resref": dlg_resref,
            "state_index": state_index,
            "owner_cre": owner_cre,
            "unique_owner": true,
            "tokens": found_tokens,
        })
    } else {
        serde_json::json!({
            "method": "permissive_owner",
            "dlg_resref": dlg_resref,
            "state_index": state_index,
            "owner_cre": owner_cre,
            "chosen_owner": owner_cre,
            "rejected_owners": rejected_owners,
            "unique_owner": false,
            "tokens": found_tokens,
        })
    };
    AttributedLine {
        strref,
        dlg_resref,
        state_index,
        text,
        original_text,
        kind,
        is_voiced: facts.is_voiced,
        existing_sound_resref: facts.sound_resref,
        has_tokens,
        token_mask,
        speaker_cre_resref: Some(owner_cre),
        confidence,
        provenance_json: provenance.to_string(),
    }
}

/// Attribute one companion DLG state to its owning CRE (`interdia` banter or side-chain).
pub fn companion_state_line(
    cre_resref: &str,
    cre: &Cre,
    dlg_resref: &str,
    state_index: u32,
    strref: u32,
    text: String,
    facts: StrrefFacts,
    death_var: &str,
    method: &str,
    campaign: Option<&str>,
    dlg_prefix: Option<&str>,
    token_reps: &TokenReplacements,
) -> AttributedLine {
    let empty = Dlg {
        states: vec![],
        transitions: vec![],
    };
    let cd = CreDialog {
        cre_resref: cre_resref.to_ascii_lowercase(),
        cre,
        dlg: &empty,
    };
    let mut line = line_of(
        &cd,
        dlg_resref,
        state_index,
        strref,
        text,
        facts,
        true,
        CONFIDENCE_UNIQUE,
        &[],
        token_reps,
    );
    let dlg_lc = dlg_resref.to_ascii_lowercase();
    let mut provenance = serde_json::json!({
        "method": method,
        "dlg_resref": dlg_lc,
        "state_index": state_index,
        "owner_cre": line.speaker_cre_resref,
        "death_var": death_var,
    });
    if let Some(campaign) = campaign {
        provenance["campaign"] = serde_json::Value::String(campaign.to_string());
    }
    if let Some(prefix) = dlg_prefix {
        provenance["dlg_prefix"] = serde_json::Value::String(prefix.to_string());
    }
    line.provenance_json = provenance.to_string();
    line.confidence = CONFIDENCE_UNIQUE;
    line
}

/// How a shared strref should be handled. Mirrors `models::SharedResolution`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharedResolution {
    /// The same speaker voices every use - one clip can be reused safely.
    ReuseSameVoice,
    /// Uses map to different (or unknown) speakers - deferred out of export.
    DeferDiffVoice,
}

impl SharedResolution {
    /// The serde/DB token (kept in lockstep with `models::SharedResolution`).
    pub fn token(self) -> &'static str {
        match self {
            SharedResolution::ReuseSameVoice => "reuse_same_voice",
            SharedResolution::DeferDiffVoice => "defer_diff_voice",
        }
    }
}

/// A detected shared strref: one TLK entry referenced by more than one line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedStrrefGroup {
    pub strref: u32,
    pub resolution: SharedResolution,
    /// The distinct speaker resrefs (`None` -> an unattributed use) seen for it.
    pub members: Vec<Option<String>>,
}

/// Group `lines` by strref and classify each group that has more than one use.
///
/// A group is `ReuseSameVoice` only when every use resolves to the *same* known
/// speaker; any speaker disagreement OR an unattributed use makes it
/// `DeferDiffVoice` (the conservative default - different-voice groups are never
/// silently patched). Single-use strrefs are not shared and are omitted.
pub fn shared_groups(lines: &[AttributedLine]) -> Vec<SharedStrrefGroup> {
    let mut by_strref: BTreeMap<u32, Vec<Option<String>>> = BTreeMap::new();
    for l in lines {
        by_strref
            .entry(l.strref)
            .or_default()
            .push(l.speaker_cre_resref.clone());
    }

    by_strref
        .into_iter()
        .filter(|(_, members)| members.len() > 1)
        .map(|(strref, members)| {
            let first = &members[0];
            let same = first.is_some() && members.iter().all(|m| m == first);
            let resolution = if same {
                SharedResolution::ReuseSameVoice
            } else {
                SharedResolution::DeferDiffVoice
            };
            SharedStrrefGroup {
                strref,
                resolution,
                members,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extractor::dlg::{Dlg, DlgState};
    use crate::extractor::token_resolve::TokenReplacements;

    fn reps() -> TokenReplacements {
        TokenReplacements::default()
    }

    fn cre(dialog: Option<&str>) -> Cre {
        Cre {
            version: "V1.0".into(),
            long_name_strref: Some(100),
            short_name_strref: None,
            sex: 1,
            gender: 1,
            general: 1,
            race: 2,
            class: 3,
            specific: 0,
            ea: 0,
            alignment: 5,
            kit: 0x4004_0000,
            dialog_resref: dialog.map(str::to_string),
            sound_slots: vec![],
        }
    }

    fn dlg(state_strrefs: &[Option<u32>]) -> Dlg {
        Dlg {
            states: state_strrefs
                .iter()
                .enumerate()
                .map(|(i, s)| DlgState {
                    index: i as u32,
                    text_strref: *s,
                    first_transition: 0,
                    transition_count: 0,
                    has_trigger: false,
                })
                .collect(),
            transitions: vec![],
        }
    }

    fn text_for<'a>(map: &'a [(u32, &'a str)]) -> impl Fn(u32) -> String + 'a {
        move |s| {
            map.iter()
                .find(|(k, _)| *k == s)
                .map(|(_, v)| (*v).to_string())
                .unwrap_or_default()
        }
    }

    #[test]
    fn unique_owner_gets_full_confidence_and_state_lines() {
        let c = cre(Some("XZAR"));
        let d = dlg(&[Some(10), Some(11)]);
        let inputs = vec![CreDialog { cre_resref: "xzar".into(), cre: &c, dlg: &d }];
        let (speakers, lines) = attribute(
            &inputs,
            text_for(&[(10, "Hello there."), (11, "Farewell.")]),
            |_| StrrefFacts::default(),
            |_| false,
            &reps(),
        );
        assert_eq!(speakers.len(), 1);
        assert_eq!(speakers[0].confidence, CONFIDENCE_UNIQUE);
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|l| l.kind == LineKind::State));
        assert_eq!(lines[0].speaker_cre_resref.as_deref(), Some("xzar"));
    }

    #[test]
    fn multi_owner_dlg_emits_once_and_attributes_representative() {
        // Two CREs (npc_a, npc_b) both declare the SAME dialogue. The DLG's single
        // state must be emitted ONCE (deduplicated), attributed to a deterministic
        // representative (here the lexically-smallest resref, since neither matches
        // the dlg resref and both are named), at the low permissive confidence.
        let (c1, c2) = (cre(Some("SHARED")), cre(Some("shared")));
        let d = dlg(&[Some(10)]);
        let inputs = vec![
            CreDialog { cre_resref: "npc_b".into(), cre: &c1, dlg: &d },
            CreDialog { cre_resref: "npc_a".into(), cre: &c2, dlg: &d },
        ];
        let (speakers, lines) = attribute(
            &inputs,
            text_for(&[(10, "Greetings.")]),
            |_| StrrefFacts::default(),
            |_| false,
            &reps(),
        );
        // Each co-owner is still an ambiguous SPEAKER, but the line is recovered.
        assert!(speakers.iter().all(|s| s.confidence == CONFIDENCE_AMBIGUOUS));
        assert_eq!(lines.len(), 1, "the shared DLG's state must emit once");
        assert_eq!(lines[0].speaker_cre_resref.as_deref(), Some("npc_a"));
        assert_eq!(lines[0].confidence, CONFIDENCE_PERMISSIVE);
        assert!(lines[0].provenance_json.contains("permissive_owner"));
        assert!(lines[0].provenance_json.contains("npc_b"), "rejected owner recorded");
    }

    #[test]
    fn multi_owner_dlg_prefers_resref_matching_owner() {
        // Models the real xzar/lyros case: both CREs point at `lyros.dlg`, so the
        // owner whose resref matches the dlg resref (lyros) wins the tiebreak even
        // though `xzar` is lexically smaller.
        let (xzar, lyros) = (cre(Some("LYROS")), cre(Some("lyros")));
        let d = dlg(&[Some(22570)]);
        let inputs = vec![
            CreDialog { cre_resref: "xzar".into(), cre: &xzar, dlg: &d },
            CreDialog { cre_resref: "lyros".into(), cre: &lyros, dlg: &d },
        ];
        let (_, lines) = attribute(
            &inputs,
            text_for(&[(22570, "Bah! Fine, whatever you say.")]),
            |_| StrrefFacts::default(),
            |_| false,
            &reps(),
        );
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].speaker_cre_resref.as_deref(), Some("lyros"));
        assert_eq!(lines[0].kind, LineKind::State);
        assert!(!lines[0].is_voiced);
    }

    #[test]
    fn tokenized_state_is_resolved_to_spoken_text() {
        let c = cre(Some("KHALID"));
        let d = dlg(&[Some(10)]);
        let inputs = vec![CreDialog { cre_resref: "khalid".into(), cre: &c, dlg: &d }];
        let (_, lines) = attribute(
            &inputs,
            text_for(&[(10, "We leave <PRO_HISHER> path.")]),
            |_| StrrefFacts::default(),
            |_| false,
            &reps(),
        );
        assert_eq!(lines[0].kind, LineKind::State);
        assert!(!lines[0].has_tokens);
        assert_eq!(lines[0].text, "We leave their path.");
        assert_eq!(lines[0].original_text, "We leave <PRO_HISHER> path.");
        assert_ne!(lines[0].token_mask, 0);
    }

    #[test]
    fn same_speaker_shared_strref_is_reuse() {
        let c = cre(Some("FARM2"));
        let d = dlg(&[Some(6140), Some(6140)]);
        let inputs = vec![CreDialog { cre_resref: "farm2".into(), cre: &c, dlg: &d }];
        let (_, lines) = attribute(
            &inputs,
            text_for(&[(6140, "Simple farmer.")]),
            |_| StrrefFacts::default(),
            |_| false,
            &reps(),
        );
        let groups = shared_groups(&lines);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].strref, 6140);
        assert_eq!(groups[0].resolution, SharedResolution::ReuseSameVoice);
    }

    #[test]
    fn different_speaker_shared_strref_is_deferred() {
        let (c1, c2) = (cre(Some("dlg_a")), cre(Some("dlg_b")));
        let (d1, d2) = (dlg(&[Some(768)]), dlg(&[Some(768)]));
        let inputs = vec![
            CreDialog { cre_resref: "garric".into(), cre: &c1, dlg: &d1 },
            CreDialog { cre_resref: "other".into(), cre: &c2, dlg: &d2 },
        ];
        let (_, lines) = attribute(
            &inputs,
            text_for(&[(768, "Reused line.")]),
            |_| StrrefFacts::default(),
            |_| false,
            &reps(),
        );
        let groups = shared_groups(&lines);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].resolution, SharedResolution::DeferDiffVoice);
    }
}
