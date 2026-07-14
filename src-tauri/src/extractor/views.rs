//! Serde view models returned across the command boundary. These are the
//! MINIMAL, forward-compatible shapes item-04 exposes; the authoritative domain
//! contracts (with DB persistence) land in item-05. All fields use snake_case so
//! the TypeScript mirror in `src/lib/types/index.ts` matches 1:1.

use serde::{Deserialize, Serialize};

use super::{cre::Cre, dlg::Dlg, dlg::DlgState, dlg::DlgTransition, tlk::TlkEntry};

/// Installed locales plus the resolved active one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GameLanguages {
    pub locales: Vec<String>,
    pub active: Option<String>,
}

/// Header-level TLK facts for the active language.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TlkSummary {
    pub locale: String,
    pub language_id: u16,
    pub entry_count: u32,
}

/// A single resolved TLK strref.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TlkEntryView {
    pub strref: u32,
    pub has_text: bool,
    pub has_sound: bool,
    pub sound_resref: Option<String>,
    pub text: String,
}

impl From<TlkEntry> for TlkEntryView {
    fn from(e: TlkEntry) -> Self {
        TlkEntryView {
            strref: e.strref,
            has_text: e.has_text,
            has_sound: e.has_sound,
            sound_resref: e.sound_resref,
            text: e.text,
        }
    }
}

/// An actor response state (a voiceable NPC line).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DlgStateView {
    pub index: u32,
    pub text_strref: Option<u32>,
    pub transition_count: u32,
    pub has_trigger: bool,
}

impl From<&DlgState> for DlgStateView {
    fn from(s: &DlgState) -> Self {
        DlgStateView {
            index: s.index,
            text_strref: s.text_strref,
            transition_count: s.transition_count,
            has_trigger: s.has_trigger,
        }
    }
}

/// A player transition (dialogue option).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DlgTransitionView {
    pub index: u32,
    pub player_text_strref: Option<u32>,
    pub terminates: bool,
    pub has_trigger: bool,
    pub has_action: bool,
    pub next_dlg: Option<String>,
    pub next_state: Option<u32>,
}

impl From<&DlgTransition> for DlgTransitionView {
    fn from(t: &DlgTransition) -> Self {
        DlgTransitionView {
            index: t.index,
            player_text_strref: t.player_text_strref,
            terminates: t.terminates,
            has_trigger: t.has_trigger,
            has_action: t.has_action,
            next_dlg: t.next_dlg.clone(),
            next_state: t.next_state,
        }
    }
}

/// A resolved DLG: states kept distinct from transitions, plus provenance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DlgView {
    pub resref: String,
    pub origin: String,
    pub state_count: u32,
    pub transition_count: u32,
    pub states: Vec<DlgStateView>,
    pub transitions: Vec<DlgTransitionView>,
}

impl DlgView {
    pub fn new(resref: String, origin: &str, dlg: &Dlg) -> Self {
        DlgView {
            resref,
            origin: origin.to_string(),
            state_count: dlg.states.len() as u32,
            transition_count: dlg.transitions.len() as u32,
            states: dlg.states.iter().map(DlgStateView::from).collect(),
            transitions: dlg.transitions.iter().map(DlgTransitionView::from).collect(),
        }
    }
}

/// A resolved creature's factual metadata (raw IDS byte values retained).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreView {
    pub resref: String,
    pub origin: String,
    pub version: String,
    pub long_name_strref: Option<u32>,
    pub short_name_strref: Option<u32>,
    pub sex: u8,
    pub gender: u8,
    pub general: u8,
    pub race: u8,
    pub class: u8,
    pub specific: u8,
    pub ea: u8,
    pub alignment: u8,
    pub kit: u32,
    pub dialog_resref: Option<String>,
    pub sound_slots: Vec<u32>,
}

impl CreView {
    pub fn new(resref: String, origin: &str, c: Cre) -> Self {
        CreView {
            resref,
            origin: origin.to_string(),
            version: c.version,
            long_name_strref: c.long_name_strref,
            short_name_strref: c.short_name_strref,
            sex: c.sex,
            gender: c.gender,
            general: c.general,
            race: c.race,
            class: c.class,
            specific: c.specific,
            ea: c.ea,
            alignment: c.alignment,
            kit: c.kit,
            dialog_resref: c.dialog_resref,
            sound_slots: c.sound_slots,
        }
    }
}
