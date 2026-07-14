//! DLG V1 reader (see IESDP "DLG V1"). Keeps actor response **states** distinct
//! from player **transitions**: a state carries the NPC's spoken TLK strref (the
//! voiceable line), while a transition is a player choice. Trigger/action
//! presence is surfaced so later items can flag script/token-driven lines.

use crate::error::AppError;

use super::bytes::{parse_err, resref, tag4, u32_le};

const FMT: &str = "dlg";
const STATE_LEN: usize = 16;
const TRANS_LEN: usize = 32;
const NO_INDEX: u32 = 0xFFFF_FFFF;

const T_TEXT: u32 = 0x01;
const T_TRIGGER: u32 = 0x02;
const T_ACTION: u32 = 0x04;
const T_TERMINATES: u32 = 0x08;
const T_JOURNAL: u32 = 0x10;

/// An actor response state: the NPC line the party hears (a voiceable strref).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DlgState {
    pub index: u32,
    /// Actor response TLK strref, or `None` when the field is -1 (no text).
    pub text_strref: Option<u32>,
    pub first_transition: u32,
    pub transition_count: u32,
    pub has_trigger: bool,
}

/// A player dialogue option leading out of a state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DlgTransition {
    pub index: u32,
    pub flags: u32,
    /// Player line strref (present only when the text flag is set).
    pub player_text_strref: Option<u32>,
    pub journal_strref: Option<u32>,
    pub has_trigger: bool,
    pub has_action: bool,
    pub terminates: bool,
    /// Next dialogue resref (absent when the transition terminates).
    pub next_dlg: Option<String>,
    pub next_state: Option<u32>,
}

/// A parsed DLG: its states and transitions, kept as separate lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dlg {
    pub states: Vec<DlgState>,
    pub transitions: Vec<DlgTransition>,
}

impl Dlg {
    /// Parse a DLG byte image.
    pub fn parse(buf: &[u8]) -> Result<Self, AppError> {
        let sig = tag4(buf, 0, FMT)?;
        if sig != "DLG" {
            return Err(parse_err(FMT, format!("bad signature {sig:?}")));
        }
        let n_states = u32_le(buf, 0x08, FMT)? as usize;
        let states_off = u32_le(buf, 0x0C, FMT)? as usize;
        let n_trans = u32_le(buf, 0x10, FMT)? as usize;
        let trans_off = u32_le(buf, 0x14, FMT)? as usize;

        guard(buf.len(), states_off, n_states, STATE_LEN)?;
        guard(buf.len(), trans_off, n_trans, TRANS_LEN)?;

        let mut states = Vec::with_capacity(n_states);
        for i in 0..n_states {
            let b = states_off + i * STATE_LEN;
            let raw = u32_le(buf, b, FMT)?;
            states.push(DlgState {
                index: i as u32,
                text_strref: (raw != NO_INDEX).then_some(raw),
                first_transition: u32_le(buf, b + 4, FMT)?,
                transition_count: u32_le(buf, b + 8, FMT)?,
                has_trigger: u32_le(buf, b + 12, FMT)? != NO_INDEX,
            });
        }

        let mut transitions = Vec::with_capacity(n_trans);
        for i in 0..n_trans {
            let b = trans_off + i * TRANS_LEN;
            let flags = u32_le(buf, b, FMT)?;
            let terminates = flags & T_TERMINATES != 0;
            let next_dlg = resref(buf, b + 0x14, FMT)?;
            transitions.push(DlgTransition {
                index: i as u32,
                flags,
                player_text_strref: (flags & T_TEXT != 0).then(|| u32_le(buf, b + 4, FMT)).transpose()?,
                journal_strref: (flags & T_JOURNAL != 0).then(|| u32_le(buf, b + 8, FMT)).transpose()?,
                has_trigger: flags & T_TRIGGER != 0,
                has_action: flags & T_ACTION != 0,
                terminates,
                next_dlg: (!terminates && !next_dlg.is_empty()).then_some(next_dlg),
                next_state: (!terminates).then(|| u32_le(buf, b + 0x1C, FMT)).transpose()?,
            });
        }

        Ok(Dlg { states, transitions })
    }
}

fn guard(file_len: usize, off: usize, count: usize, stride: usize) -> Result<(), AppError> {
    let need = count
        .checked_mul(stride)
        .and_then(|n| n.checked_add(off))
        .ok_or_else(|| parse_err(FMT, "table size overflow"))?;
    if need > file_len {
        return Err(parse_err(FMT, "table exceeds file"));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn build_dlg(states: &[(u32, u32, u32)], trans: &[(u32, u32, u32, &str, u32)]) -> Vec<u8> {
    // states: (text_strref, first_transition, transition_count)
    // trans:  (flags, player_strref, journal_strref, next_dlg, next_state)
    let states_off = 0x34; // header incl. the BG2 flags dword
    let trans_off = states_off + states.len() * STATE_LEN;

    let mut out = Vec::new();
    out.extend_from_slice(b"DLG V1.0");
    out.extend_from_slice(&(states.len() as u32).to_le_bytes());
    out.extend_from_slice(&(states_off as u32).to_le_bytes());
    out.extend_from_slice(&(trans.len() as u32).to_le_bytes());
    out.extend_from_slice(&(trans_off as u32).to_le_bytes());
    for _ in 0..7 {
        // state-trigger off/count, trans-trigger off/count, action off/count, BG2 flags
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    for (text, first, count) in states {
        out.extend_from_slice(&text.to_le_bytes());
        out.extend_from_slice(&first.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&NO_INDEX.to_le_bytes()); // no state trigger
    }
    for (flags, player, journal, next, state) in trans {
        let mut nd = [0u8; 8];
        let b = next.as_bytes();
        nd[..b.len().min(8)].copy_from_slice(&b[..b.len().min(8)]);
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&player.to_le_bytes());
        out.extend_from_slice(&journal.to_le_bytes());
        out.extend_from_slice(&NO_INDEX.to_le_bytes()); // trigger idx
        out.extend_from_slice(&NO_INDEX.to_le_bytes()); // action idx
        out.extend_from_slice(&nd);
        out.extend_from_slice(&state.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separates_states_from_transitions() {
        let bytes = build_dlg(
            &[(1000, 0, 1), (NO_INDEX, 1, 1)],
            &[
                (T_TEXT, 2000, 0, "NEXTDLG", 3),
                (T_TERMINATES | T_TEXT, 2001, 0, "IGNORED", 9),
            ],
        );
        let dlg = Dlg::parse(&bytes).unwrap();
        assert_eq!(dlg.states.len(), 2);
        assert_eq!(dlg.states[0].text_strref, Some(1000));
        assert_eq!(dlg.states[1].text_strref, None);

        let t0 = &dlg.transitions[0];
        assert_eq!(t0.player_text_strref, Some(2000));
        assert!(!t0.terminates);
        assert_eq!(t0.next_dlg.as_deref(), Some("nextdlg"));
        assert_eq!(t0.next_state, Some(3));

        let t1 = &dlg.transitions[1];
        assert!(t1.terminates);
        assert_eq!(t1.next_dlg, None);
        assert_eq!(t1.next_state, None);
    }
}
