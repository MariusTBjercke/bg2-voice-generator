//! The human-readable README shipped inside every pack (item-09): install, verify,
//! and uninstall instructions plus a summary of what the pack patches and the
//! fingerprint it was built against. Pure string generation from the manifest.

use super::manifest::Manifest;

/// Render `README.txt` for a built pack.
pub fn readme(m: &Manifest) -> String {
    let fp = &m.fingerprint;
    let mut s = String::new();
    s.push_str(&format!("{} - generated native voice pack\n", m.pack_name));
    s.push_str("Produced by BG2 Voice Generator (personal use). Native WeiDU install;\n");
    s.push_str("no EEex, sidecar, runtime TTS, or background process is required to play.\n\n");

    s.push_str("BUILT AGAINST\n");
    s.push_str(&format!(
        "  edition:        {} ({})\n",
        fp.edition, fp.edition_version
    ));
    s.push_str(&format!("  language:       {}\n", fp.language));
    s.push_str(&format!("  tlk_entry_count:{}\n", fp.tlk_entry_count));
    s.push_str(&format!("  mod_state_hash: {}\n", fp.mod_state_hash));
    s.push_str(&format!(
        "  generator/export: {} / {}\n\n",
        m.generator_version, m.export_version
    ));
    s.push_str(&format!("  audio format:    {}\n\n", m.audio_format));

    s.push_str(&format!(
        "CONTENTS\n  {} voiced line(s), {} deferred line(s). See manifest.json for the\n  full per-line strref/resref/text-hash detail.\n\n",
        m.lines.len(),
        m.deferred.len()
    ));

    s.push_str("INSTALL\n");
    s.push_str("  1. BACK UP your game folder first (a full copy is the safest rollback).\n");
    s.push_str("  2. Extract the pack ZIP into the game directory (next to chitin.key):\n");
    s.push_str(&format!(
        "     that places setup-{}.exe and the '{}' folder side by\n",
        m.pack_name, m.pack_name
    ));
    s.push_str(&format!(
        "     side. Run setup-{}.exe from the game directory and install\n",
        m.pack_name
    ));
    s.push_str("     the 'Generated Voiceovers' component. That setup exe is a bundled,\n");
    s.push_str("     unmodified copy of WeiDU (GPLv2) - no separate WeiDU download is needed.\n");
    s.push_str("  3. WeiDU copies the audio into override/ and attaches each clip to its\n");
    s.push_str("     dialogue string while preserving the original dialogue text and runtime\n");
    s.push_str("     tokens (for example, <CHARNAME>). It refuses to install on the wrong game edition and\n");
    s.push_str("     warns if your mod state differs from the one it was built against.\n\n");

    s.push_str("VERIFY\n");
    s.push_str("  - Launch the game. Trigger a patched NPC line; it should now play audio with\n");
    s.push_str("    the subtitle intact (no EEex, sidecar, or runtime TTS required).\n");
    s.push_str("  - Confirm WeiDU.log lists the pack and override/ gained the staged WAVs.\n\n");

    s.push_str("UNINSTALL\n");
    s.push_str("  - Run the pack's WeiDU setup again and UNINSTALL the component. WeiDU\n");
    s.push_str("    restores dialog.tlk and removes the staged audio from its BACKUP folder.\n");
    s.push_str("  - dialog.tlk is NEVER edited outside WeiDU, so the uninstall is clean; the\n");
    s.push_str("    full folder backup from step 1 is the last-resort fallback.\n");
    s.push_str("  - If replacing a pack made by an older BG2 Voice Generator version, uninstall\n");
    s.push_str("    that component first, then install the rebuilt pack so its backups and audio\n");
    s.push_str("    collision guards are applied cleanly.\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::manifest::{sha256_hex, DeferredLine, PackFingerprint, PackLine};

    fn manifest() -> Manifest {
        Manifest {
            pack_name: "BG2VG_Voices".into(),
            generator_version: "0.1.0".into(),
            export_version: "1".into(),
            audio_format: crate::audio::vorbis::AUDIO_FORMAT.into(),
            created_at: "now".into(),
            fingerprint: PackFingerprint {
                edition: "bg2ee".into(),
                edition_version: "2.6".into(),
                language: "en_US".into(),
                mod_state_hash: "abc123".into(),
                tlk_entry_count: 103_778,
            },
            lines: vec![PackLine {
                line_id: 1,
                strref: 22570,
                resref: "Z0H6A00".into(),
                text: "Hi.".into(),
                text_sha256: sha256_hex(b"Hi."),
                speaker_resref: "XZAR".into(),
                binding_source: "default".into(),
            }],
            deferred: vec![DeferredLine {
                line_id: 2,
                strref: 3,
                reason: "line has dynamic tokens".into(),
            }],
        }
    }

    #[test]
    fn readme_covers_install_verify_uninstall_and_the_fingerprint() {
        let r = readme(&manifest());
        assert!(r.contains("INSTALL"));
        assert!(r.contains("VERIFY"));
        assert!(r.contains("UNINSTALL"));
        assert!(r.contains("mod_state_hash: abc123"));
        assert!(r.contains("1 voiced line(s), 1 deferred line(s)"));
        assert!(r.contains("NEVER edited outside WeiDU"));
    }
}
