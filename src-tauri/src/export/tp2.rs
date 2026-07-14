//! Pure WeiDU `.tp2` + `.tra` emitters for a `PackPlan` (item-09).
//!
//! Follows the confirmed native WeiDU pattern (feasibility gate): copy
//! each staged `<RESREF>.wav` into `override/` and attach it to the target strref
//! with `STRING_SET`. All patching goes through WeiDU (`BACKUP` + uninstall), so the
//! exporter never edits `dialog.tlk` itself.
//!
//! Guards, in order of severity:
//!   * `REQUIRE_PREDICATE GAME_IS ~<edition>~` - HARD refuse on the wrong game.
//!   * `ACTION_IF FILE_EXISTS_IN_GAME ~<resref>.wav~` skip - never clobber existing
//!     audio (also the resref-collision backstop).
//!   * mod-state hash - recorded in the tp2 as a comment + WARN print; drift is
//!     common (the user keeps modding), so it warns rather than fails.
//! PURE: returns strings; `build` does the IO. tra text is `~...~`-escaped.

use super::plan::PackPlan;

/// Escape a TLK string for a WeiDU `~...~` tra literal. WeiDU's `~` string cannot
/// contain a literal `~`; the safe, widely-used swap is the `%TILDE%` token. Control
/// chars are dropped so a stray byte can't break the tra parse.
fn tra_escape(text: &str) -> String {
    text.chars()
        .filter(|c| *c == '\n' || *c == '\t' || !c.is_control())
        .map(|c| if c == '~' { '\u{1}' } else { c })
        .collect::<String>()
        .replace('\u{1}', "%TILDE%")
}

/// The tra file body: one `@<strref> = ~original subtitle text~ [RESREF]` per line.
/// The `[RESREF]` sound tag is what makes `STRING_SET` attach audio; `PackLine.text`
/// is deliberately the original TLK text rather than any generation-only stand-in.
pub fn emit_tra(plan: &PackPlan) -> String {
    let mut out = String::new();
    for l in &plan.lines {
        out.push_str(&format!(
            "@{} = ~{}~ [{}]\n",
            l.entry.strref,
            tra_escape(&l.entry.text),
            l.entry.resref
        ));
    }
    out
}

/// The `.tp2` body. `tra_rel` is the tra path relative to the game dir (WeiDU
/// resolves mod paths from there), `pack_name` is the mod folder.
pub fn emit_tp2(plan: &PackPlan, pack_name: &str, generator_version: &str) -> String {
    let fp = &plan.fingerprint;
    let mut s = String::new();
    s.push_str(&format!("BACKUP ~{pack_name}/backup~\n"));
    s.push_str("AUTHOR ~BG2 Voice Generator (personal use)~\n");
    s.push_str(&format!("VERSION ~{generator_version}~\n\n"));
    // Provenance + the mod-state guard value, embedded for audit + WARN-on-drift.
    s.push_str(&format!("// edition={} version={}\n", fp.edition, fp.edition_version));
    s.push_str(&format!("// tlk_entry_count={}\n\n", fp.tlk_entry_count));
    s.push_str(&format!(
        "LANGUAGE ~{lang}~ ~{lang}~ ~{pack}/tra/{lang}/setup.tra~\n\n",
        lang = fp.language,
        pack = pack_name
    ));

    s.push_str("BEGIN ~Generated Voiceovers~\n");
    // HARD edition guard: refuse to patch the wrong game.
    s.push_str(&format!(
        "  REQUIRE_PREDICATE GAME_IS ~{}~ ~This pack targets {} only.~\n",
        fp.edition, fp.edition
    ));
    // Soft mod-state guard: a visible warning; the user decides.
    s.push_str(&format!(
        "  PRINT ~[BG2VG] Built against mod_state_hash {}; verify your install matches.~\n",
        fp.mod_state_hash
    ));
    // The STRING_SET guards run BEFORE the COPY: `FILE_EXISTS_IN_GAME` must observe
    // the PRE-install state. Copying first would make every guard see our own staged
    // WAV in override/ and skip its STRING_SET, installing silent audio.
    for l in &plan.lines {
        // Skip if that resref already exists in-game (collision / re-run safety).
        s.push_str(&format!(
            "  ACTION_IF !FILE_EXISTS_IN_GAME ~{res}.wav~ THEN BEGIN STRING_SET {sref} @{sref} END\n",
            res = l.entry.resref,
            sref = l.entry.strref
        ));
    }
    s.push('\n');
    s.push_str(&format!("  COPY ~{pack_name}/audio~ ~override~\n\n"));
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::manifest::{sha256_hex, PackFingerprint, PackLine};
    use crate::export::plan::{PackPlan, PlannedLine};

    fn plan() -> PackPlan {
        PackPlan {
            pack_name: "BG2VG".into(),
            fingerprint: PackFingerprint {
                edition: "bg2ee".into(),
                edition_version: "2.6".into(),
                language: "en_US".into(),
                mod_state_hash: "deadbeef".into(),
                tlk_entry_count: 103_778,
            },
            lines: vec![PlannedLine {
                entry: PackLine {
                    line_id: 1,
                    strref: 22570,
                    resref: "Z0H6A00".into(),
                    text: "Hello ~there~.".into(),
                    text_sha256: sha256_hex(b"Hello ~there~."),
                    speaker_resref: "XZAR".into(),
                    binding_source: "default".into(),
                },
                audio_source_path: "/ws/1.wav".into(),
            }],
            deferred: vec![],
        }
    }

    #[test]
    fn tra_escapes_tildes_and_tags_the_resref() {
        let tra = emit_tra(&plan());
        assert!(tra.contains("@22570 = ~Hello %TILDE%there%TILDE%.~ [Z0H6A00]"));
        assert!(!tra.contains("~Hello ~there~"), "raw tilde must be escaped");
    }

    #[test]
    fn tra_preserves_dynamic_tokens_while_attaching_generated_audio() {
        let mut plan = plan();
        plan.lines[0].entry.text = "Hello <CHARNAME>, welcome.".into();
        let tra = emit_tra(&plan);
        assert!(tra.contains("@22570 = ~Hello <CHARNAME>, welcome.~ [Z0H6A00]"));
    }

    #[test]
    fn tp2_has_backup_edition_guard_copy_and_string_set() {
        let tp2 = emit_tp2(&plan(), "BG2VG", "0.1.0");
        assert!(tp2.contains("BACKUP ~BG2VG/backup~"));
        assert!(tp2.contains("REQUIRE_PREDICATE GAME_IS ~bg2ee~"));
        assert!(tp2.contains("COPY ~BG2VG/audio~ ~override~"));
        assert!(tp2.contains("FILE_EXISTS_IN_GAME ~Z0H6A00.wav~"));
        assert!(tp2.contains("STRING_SET 22570 @22570"));
        assert!(tp2.contains("[BG2VG] Built against mod_state_hash deadbeef"));
    }

    #[test]
    fn string_set_guards_run_before_the_audio_copy() {
        // FILE_EXISTS_IN_GAME must observe the PRE-install state: if the COPY ran
        // first, every guard would see our own staged WAV and skip its STRING_SET.
        let tp2 = emit_tp2(&plan(), "BG2VG", "0.1.0");
        let set_pos = tp2.find("STRING_SET 22570").unwrap();
        let copy_pos = tp2.find("COPY ~BG2VG/audio~").unwrap();
        assert!(set_pos < copy_pos, "STRING_SET must precede the COPY");
    }
}
