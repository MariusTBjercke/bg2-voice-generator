# Validation Checklist - BG2 Voice Generator

The validation strategy has three tiers. The first two are automated and CI-runnable; the third is a
manual, opt-in integration run against a **backed-up copy** of a real modded install. This document is
the source of truth for validation gates.

## Tier 1 - Automated unit + contract tests (must pass before merging)

Deterministic, no game copy, no network, no copyrighted audio. Hand-built binary fixtures only.

- **Rust unit tests** (`cd src-tauri && cargo test --lib`):
  - Extractor parsers over deterministic fixtures: `extractor::tlk` (header, sound resref, OOB reject),
    `extractor::dlg` (state vs transition), `extractor::cre` (factual fields, signature reject),
    `extractor::bif` / `extractor::key` (resource locate), `extractor::restype` (round-trip).
  - Attribution: unique-owner confidence + state-line assignment, shared-DLG ambiguity, token detection.
  - Export: `resref` (8-char, stable, collision probe), `tp2` (tilde escape, guards, TLK bounds),
    `manifest` (stable JSON), `plan` (defer-with-reason), `build` (full pack layout + Vorbis backstop),
    `zip` (one top-level folder, forward-slash entries, WeiDU staged as `setup-<pack>.exe`).
  - Fingerprint: BG2EE edition guard, TLK entry count, mod-state hash.
  - Audio/DB/generator plumbing (scoring, wav decode, harvest persistence, resumable +
    batched line output, metadata binding, speaker identity groups), including non-spoken
    dialogue filtering that preserves existing generation records during merge rescans,
    and voice-snapshot tracking that keeps rebound clips exportable but visibly stale.
  - Synthesis transcript resolution (pinned OmniVoice tag catalog, mapper, overrides, review
    markers, paged human-review queue, corpus audit, targeted transcript invalidation) and
    `bg2-synthesis` CLI validation.
- **TS<->Rust model contract** (both halves must agree):
  - Rust: `cargo test --lib contract_tests` - `models.rs` pins the serde JSON key set + enum tokens for
    every struct/enum mirrored in `src/lib/types/index.ts` (domain models + `extractor::views` +
    command result types incl. `ExportResult.pack_zip`).
  - TS: `npm run test` (`vitest`) - `src/lib/types/contract.test.ts` pins the SAME key sets on the TS
    interfaces (typed sample literals -> compile-time drift check + runtime `Object.keys` assertion) and
    the enum unions. Editing one side without the other fails the build.
- **Type check**: `npm run check` (svelte-check) - 0 errors / 0 warnings.

**Acceptance gate (Tier 1):** all of `cargo test --lib`, `npm run test`, `npm run check` green.

## Tier 2 - Real-install read-only tests (opt-in, no mutation)

Behind `#[ignore]`; exercise the readers against the real active resources without changing anything.

- Run: `cd src-tauri && $env:BG2_GAME_DIR="<install path>"; cargo test -- --ignored`
  (module `extractor::real_install`: active TLK parse, DLG + CRE resolution, speaker attribution).
- **Never** writes to the install. Safe to run against a live install (read-only).

**Acceptance gate (Tier 2):** the ignored tests pass against the current modded install, or are
explicitly skipped with a recorded reason (e.g. no install available in CI).

## Tier 3 - Manual disposable/backed-up-game integration checklist

Run ONLY against a **full backup first**. Change the install ONLY via WeiDU. The exported pack
itself needs no EEex, sidecar, or runtime TTS — native WeiDU `STRING_SET` + `override/` audio is
enough. If your test install happens to keep EEex/TNT or other mods for unrelated reasons, note
that in the evidence; do not treat EEex as a product requirement. This is where exported packs and
the portable app ZIP get their runtime proof. Record every result inline in this file.

Pre-flight
- [ ] Full backup of the install captured; restore path recorded.
- [ ] `WeiDU.log` and `override/` file count snapshotted (before).

M1 - Scan
- [ ] App scans the install; resolves active DLG/TLK/CRE/sound from the current mod state.
- [ ] At least one unvoiced, unique, high-confidence NPC DLG-state line is detected.
- [ ] Companion banter/interjection lines from `interdia.2da` appear in attribution counts
  (`companion_lines_added` > 0 on a full-party install).
- [ ] Companion side-chain DLGs (e.g. Jaheira `jaheiraj.dlg`, strref 49599 Harper line)
  appear after scan (`companion_side_lines_added` > 0; line attributed to Jaheira's
  dominant long-name identity rather than whichever `jahei*` CRE sorts first).
- Evidence: line strref + speaker resref + confidence recorded.

M2 - Generate one line
- [ ] OmniVoice generates a clip for the chosen line; run is resumable (re-run skips done-on-disk).
- [ ] Persistent output is mono 22.05 kHz Ogg Vorbis q6; temporary PCM is removed and
      a re-run resumes from the `.ogg` file.
- Evidence: `generation_id`, output path, `resumed` on re-run.

M3 - Export the pack
- [ ] `build_export` writes the guarded pack folder (tp2/tra/audio/manifest/README/backup).
- [ ] `pack_zip` is produced with the bundled `setup-<pack>.exe` (portable mode with vendored WeiDU).
- [ ] Pack contains only this project's generated audio (no game originals or third-party mod files).
- [ ] Each staged `<RESREF>.wav` contains the generated Ogg Vorbis bytes unchanged;
      `manifest.json` records `ogg_vorbis_q6_22050_mono`.
- Evidence: `export_id`, `pack_dir`, `pack_zip`, `patched_lines`, `deferred_lines`, `mod_state_hash`.

M4 - Install via WeiDU
- [ ] Extract the pack ZIP; run the bundled `setup-<pack>.exe`; component installs cleanly.
- [ ] Fingerprint guard matches the install (edition/language/mod-state); mismatch would warn/refuse.
- Evidence: WeiDU install log excerpt; `override/` now shows exactly +1 `<RESREF>.wav`.

M5 - Verify native playback (no runtime dependency on this app)
- [ ] Launch the game; the chosen line plays the generated audio in-game.
- [ ] Subtitles/text intact; active-language behavior unchanged; no unexpected override collisions.
- Evidence: in-game confirmation note; subtitle text observed.

M6 - Clean uninstall / diff
- [ ] WeiDU uninstall of the pack restores prior state (line back to unvoiced, added WAV removed).
- [ ] `override/` count and `WeiDU.log` match the M1 snapshot; unrelated mod files byte-match backup.
- [ ] `dialog.tlk` was never irreversibly mutated (WeiDU BACKUP/uninstall only).
- Evidence: before/after `override/` diff + `WeiDU.log` diff; restore-from-backup path if needed.

## Tier 3b - Portable app ZIP smoke test

- [ ] `build-portable.ps1` produces `BG2VoiceGenerator-<version>.zip`.
- [ ] On a clean second machine (no dev toolchain), unzip and launch; app starts, `engine/` runtime is
      provisioned, `/health` reachable.
- Evidence: machine description + first-run result.

## Milestone -> evidence-gate mapping

| Milestone | Scope | Gate |
| --- | --- | --- |
| Tier 1 | Rust unit/contract tests over fixtures; TS ↔ Rust model contract checks | green before merging |
| Tier 2 | Resolution against current mod state | ignored tests pass or skipped-with-reason |
| M1-M6 | End-to-end integration on a backed-up modded install (scan → generate → export → uninstall) | recorded pass on the backed-up install |
| Tier 3b | Portable ZIP smoke test on a clean second machine | recorded pass |

## Notes

- Tiers 1-2 are the CI/local gate; there is no hosted CI yet (`build-portable.ps1` runs the same gates
  locally). Tier 3/3b are manual and their evidence is recorded here.
- An initial feasibility run on a real modded install (strref 22570, Xzar) already proved native
  WeiDU playback and clean uninstall; Tier 3 is the repeatable, generator-driven form for future packs.

### OmniVoice quality-improvements validation record (2026-07-14)

- Tier 1 passed: `cargo check`, 355 Rust unit tests (5 real-install tests ignored),
  `npm run check`, 16 Vitest tests, and 36 Playwright tests.
- The local GPU is an NVIDIA GeForce RTX 5070 Ti (16,303 MiB, driver 610.47). The
  quality harness successfully planned its complete 360-case matrix (4 voices, 20 lines),
  and its seven Python harness/server tests passed.
- Tier 2 and Tier 3 were not run in this validation pass: `BG2_GAME_DIR` was unset and
  no backed-up BG2EE installation was placed in scope. Tier 3b was also not run on a
  clean second machine. Keep the boxes above unchecked until those runs are recorded.
- No real audio-quality benchmark or blind listening pass was run. The checked-in corpus
  is intentionally audio-free and uses placeholder reference paths. Consequently this
  pass does not justify changing automatic pacing, 32-step defaults, or composite opt-in.
  Reproduce the empirical quality matrix and blind scoring with the commands in
  `docs/testing/omnivoice-quality-benchmark.md`; keep all game-derived audio local.
