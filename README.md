# BG2 Voice Generator

A Windows desktop app that gives the unvoiced dialogue in **Baldur's Gate II: Enhanced
Edition (BG2EE)** a voice. Point it at your install (in its current modded state), and it scans the game text, works out who says each
line, harvests short reference clips of each speaker's *existing* official audio, clones
that voice locally, generates the missing lines, and exports a **native WeiDU voice pack**
you install like any other mod.

The exported pack is self-contained: it copies audio into `override/` and attaches it to
the dialogue via WeiDU `STRING_SET`, so it plays through the game's own dialogue-audio
mechanism with **no EEex, no sidecar, no runtime TTS, and no background process** required
at play time.

## Copyright stance

This tool never redistributes game audio as a public mod. Voice **clones** are learned
locally from the reference clips already present in *your* installation. **WeiDU export
packs** contain only generated derivatives for in-game use. **Profile Transfer** backups
may include local workspace audio so you can move your own work between machines or keep
a demo sandbox — keep those ZIPs private; they are not a redistribution channel.

## Prerequisites

Everything below is Windows 10/11, 64-bit.

**To run the portable build** (see [Install](#install)): nothing — a stripped CPython,
`ffmpeg`/`ffprobe`, and WeiDU all ship next to the exe, and the voice model downloads on
first use. A recent NVIDIA GPU is strongly recommended for the generation step; the
exported packs need no GPU (or anything else) to play.

**To build from source** you also need:

- [Rust](https://rustup.rs/) **1.85 or newer** (stable, not nightly) — check with
  `rustc --version`. A `feature edition2024 is required` build error means your toolchain
  is older than 1.85; `rustup update` fixes it.
- [Node.js](https://nodejs.org/) 18+ with npm — check with `node --version` /
  `npm --version`.
- The [Tauri v2 prerequisites](https://v2.tauri.app/start/prerequisites/): the Visual
  Studio C++ build tools and the WebView2 runtime (WebView2 ships with current Windows).
- The vendored tools (WeiDU, ffmpeg/ffprobe, CPython), materialised by `fetch-tools.ps1`
  (below). Harvesting reference clips needs `ffmpeg`; exporting a Zip-packaged installer
  needs WeiDU.
- For the **generation** step: the local OmniVoice engine (a Python venv + a model of
  roughly a couple of GB), installed automatically on first run into a writable
  `engine-runtime/` folder. A GPU is strongly recommended.

## Install

Grab the portable build (or produce one yourself — see [Build the portable app](#build-the-portable-app)),
unzip the whole `BG2VoiceGenerator-<version>/` folder somewhere you can write to, and run
`bg2-voice-generator.exe`. Keep `bg2-voice-generator.exe`, `bg2-synthesis.exe`, `engine/`,
and `tools/` together — that sibling layout is what switches the app into portable mode.
On first launch it creates `engine-runtime/` next to the exe and installs the local engine
there. The bundled `README.txt` is the first-run guide.

## Quick start (from source)

```powershell
npm install                 # pinned dependency versions
.\fetch-tools.ps1           # download + verify WeiDU, ffmpeg/ffprobe, CPython into tools/
npm run tauri dev           # build the Rust backend and open the app in dev
```

Useful gates while developing (also run by the portable build):

```powershell
npm run check                                          # frontend type-check (svelte-check)
npm run test                                           # frontend tests (vitest)
npm run test:e2e                                       # browser UI E2E (Playwright; see docs/testing/e2e.md)
cargo check --manifest-path src-tauri\Cargo.toml       # backend compile
cargo test  --lib --manifest-path src-tauri\Cargo.toml # backend unit tests
```

`fetch-tools.ps1` is checksum-pinned and idempotent; pass `-Force` to re-fetch. WeiDU can
be skipped for an offline/CI run with `-SkipWeidu` (the app then generates but can't build
a packaged installer ZIP).

## Build the portable app

```powershell
.\build-portable.ps1        # runs the gates, builds the exe, stages tools/, zips + deploys
```

This produces `dist\BG2VoiceGenerator-<version>.zip` and also deploys an unzipped, ready-
to-run copy into `dist\portable\` (whose `engine-runtime/` survives rebuilds). Handy
switches: `-SkipBuild` re-stages an already-built exe, `-NoDeploy` builds the zip only,
`-Force` re-fetches the vendored tools, `-CleanRuntime` wipes the deployed
`engine-runtime/` to force a fresh engine install.

## How to use

The app is a top-to-bottom pipeline; the header nav walks the same order. Each screen
reads from the one before it, so run them in sequence the first time.

1. **Setup** (the landing screen). Click **Choose game folder…** and pick your BG2EE
   install (the folder with `chitin.key`, `override/`, and `lang/`). The choice is
   remembered across restarts. The app then lists the installed languages under `lang/`
   and lets you pick the active locale; that per-install choice is also remembered and is
   used for the scan/harvest calls.
   An invalid folder shows a friendly "no languages found" message.

2. **Dictionary** (optional, but recommended before the first scan). The **Placeholders** tab
   configures spoken
   stand-ins for dynamic dialogue tokens like `<CHARNAME>` and `<PRO_HISHER>` used only for
   generated audio. The exported pack preserves the original in-game text and tokens. Pick a **PC
   profile** (neutral / male / female) for gendered protagonist tokens; advanced overrides
   let you customise individual tokens. **Save + Apply** persists the settings and, on an
   existing project, re-applies stand-ins to tokenized lines. New scans pick up placeholder
   settings automatically. The **Pronunciation** tab applies machine-wide `find → speak as`
   rules immediately before generation. The **Tag rules** tab seeds the stage-cue → OmniVoice
   tag mapper as editable defaults and lets you add spoken-word → tag rules (for example
   `Bah` → `[dissatisfaction-hnn]`). Both affect generated audio only; subtitles remain unchanged.

3. **Attribution.** Click **Scan attribution** to read the game text and work out which
   speaker each line belongs to. You get count cards (speakers, lines, ready lines,
   blocked lines, non-spoken lines, shared groups, deferred groups, companion lines from `interdia.2da`)
   and a paged, filterable table of the **blocked** lines the app can't safely attribute
   yet (read-only). Filter by blocked reason to see how much is already voiced vs
   shared-strref deferrals vs unresolved tokens. See [Dialogue coverage](#dialogue-coverage)
   for how to read these counts when describing a finished pack. A progress bar and
   **Cancel** button appear while a scan runs. **Re-scan** merges new lines by default and
   keeps harvest approvals, bindings, demographic pools, and completed generations for
   lines that still exist; check **Wipe harvest, bindings, and generation state** only
   when you want a clean slate.

4. **Harvest & approve.** Click **Harvest references** to pull short clips of each
   speaker's existing official audio from uniquely owned main CRE dialogue, companion
   banter/post/join/side dialogue (capped to the best usable clips per speaker), and
   sound-slot barks (this step needs `ffmpeg`; a clear warning appears if it's missing).
   Speakers who still have Ready lines but few automatic samples also get an Attribution
   **gap-fill** pass (uniquely attributed official VO only, capped). Laughs, grunts, and
   clips whose duration cannot match the TLK transcript are skipped.
   **Re-harvest is additive**: existing samples, approvals, bindings, and voice-profile
   links are kept; only newly discovered sound resrefs are decoded and saved as pending.
   The speaker list groups variant CREs into **identity groups** (e.g. every
   `jahei*` form of Jaheira). Pick a group to see candidate samples with quality scores,
   press ▶ to audition a clip in-app, and **Approve** or **Reject** each one. Use
   **Auto-approve best for all speakers** (or the per-group **Approve best**) to accept
   the top-scored pending sample everywhere at once — already-decided groups are left
   untouched. **Verify speech (optional VAD)** runs a neural voice-activity check over
   pending samples (engine required). Long lists are paged and filterable; progress/cancel
   work as on Attribution.

5. **Bind.** The collapsible **Voice library** holds reusable, project-scoped voice
   profiles. A profile may come from approved harvested game audio, one to four audio
   files you import with exact manual transcripts, or a structured OmniVoice design
   audition. Imported audio is copied and normalized locally. Voice design renders three
   seeded candidates; saving one freezes that audition as local reference audio, so later
   dialogue uses ordinary voice cloning and does not drift between lines.

   Build demographic **Voice pools** from any mixture of harvested, imported, and designed
   profiles, then **Apply defaults** for stable per-speaker distribution across each pool.
   A personal profile or approved harvested sample overrides the pool and applies to the
   full identity group. Profiles report their origin and local availability. Use **Exclude from pack** on a character to keep Generate all/missing and
   Export from voicing them (useful for mute companions). If they already have generated
   clips, you are asked whether to delete those too; declining still excludes them from
   packs while keeping the files for a later re-include.

6. **Generate.** This is the only screen that uses the OmniVoice engine. On a fresh
   install the engine card shows **Install engine** — click it once to provision the local
   engine into `engine-runtime/` (a Python venv, the torch/OmniVoice dependencies, and the
   model weights). This is a one-time, network-dependent download of several GB; a
   determinate progress bar and **Cancel** button track it, and cancelling leaves a clean
   state so you can retry. Before installing you can pick the compute target
   (**Auto-detect** / **CPU only** / **NVIDIA GPU**); a GPU is strongly recommended. Once
   installed, use **Start** / **Stop** to control the engine and watch its status
   (starting → up → ready; the model loads on the first line). Tune **batch size** and
   **character budget** (defaults: 8 lines / 800 chars) to balance speed and VRAM — see
   [`docs/OMNIVOICE-PERF.md`](docs/OMNIVOICE-PERF.md). Then generate lines one at a time
   (**Generate** / **Re-generate**) or use the filtered batch actions (with per-line
   fallback on failure), and audition finished lines in-app. If you later change a
   speaker's binding, the earlier clips remain playable and exportable but are marked
   **Voice changed**. Dictionary / override / stand-in transcript drifts are marked
   **Text changed** the same way. Use **Re-generate voice-changed** or
   **Re-generate text-changed** within the current filter, or remove
   individual/all-filtered generated clips when you do not want them in the pack.
   Blocked or skipped lines that still have a clip also appear here (filter **Line
   state → Blocked/Skipped**, or follow the Export warning links) so you can preview
   or remove them — they are not included in **Re-generate all**. To reset a
   broken install, delete `engine-runtime/` (or rebuild the portable copy with
   `-CleanRuntime`) and install again. Generation is the GPU-heavy, model-dependent step;
   everything else works without it.

   OmniVoice renders temporary mono PCM, which the app immediately compresses once to
   22.05 kHz Ogg Vorbis at the conservative q6 quality level. The workspace keeps the
   resulting `.ogg`; temporary PCM is removed. Export copies those same compressed bytes
   without another lossy encode, naming them `.wav` because that is BG2EE's dialogue
   resource extension. Harvested reference samples remain lossless PCM for voice cloning.

   Generation keeps the original TLK text for display/export, but prepares a separate
   OmniVoice transcript. The **stage-direction mapper** always converts supported cues
   in place (`*sigh*` → `[sigh]`, laugh/grin/gasp variants likewise). Cues without a model
   control, including `*sniff*` and `*breath*`, are stripped along with unknown `*...*`
   and game `[...]` annotations. Upgrading from a build that emitted unsupported
   `[sniff]` / `[breath]` tags automatically returns affected clips to pending and clears only
   overrides containing those tags.

   Every line also has an **Edit generation text** action. Use it to remove invalid markup,
   reposition a cue, or add a supported OmniVoice tag when a line fails or sounds wrong.
   Overrides must preserve the subtitle's spoken words, apply to every identical string, and
   never change the subtitle or exported TLK text.

6a. **Dialogue review** (optional). The **Review** screen opens on deterministically flagged
   strings and also provides a paged **Remaining** queue for every undecided unique string.
   Accept the current mapper output as reviewed, or edit and save a generation-only override.
   Overrides and review progress persist across sessions but remain local to this machine.

   For AI-assisted review, the same screen stages a safe project workspace and launches Codex
   or Claude to review unique dialogue strings using the bundled
   `bg2-synthesis.exe` companion CLI. The workspace uses the cross-agent layout:
   `AGENTS.md` (Codex reads this directly), `CLAUDE.md` (imports `AGENTS.md` for Claude
   Code), and the same `set-synthesis` skill under `.agents/skills/` and
   `.claude/skills/`. The agent accepts the default mapper with `review` or writes a
   generation-only override with `tag`; it never edits TLK text directly. Overrides may
   fix mapper placement or, **very sparingly**, add allowed OmniVoice inline tags when
   delivery is unambiguous (`bg2-synthesis catalog` lists body/non-verbal tags plus
   English intonation tags like `[question-en]` and `[surprise-ah]`). Tags such as
   `[angry]` / `[sad]` are **not** supported by the base checkpoint. Progress persists
   across sessions. Overrides are local machine state and are not included in
   project-transfer bundles.

   The mapper always runs for lines without an override. Most agent work is `review` —
   marking a unique string as looked-at while the mapper turns supported cues such as
   `*sigh*` into OmniVoice tags and strips unsupported cues. Use `tag` only when you
   want a deliberate override; those always win over the mapper. Review markers do not
   lock in synthesis text — they only track workflow progress on the Review screen.

   **Typical session**

   1. Finish **Attribution** (and optional earlier pipeline steps) so the project exists.
   2. Open **Review** and note the progress counters (`remaining` is what still needs a
      decision).
   3. Click **Launch Codex** or **Launch Claude** (or **Reveal workspace** and start the
      CLI yourself). The app opens a terminal in a prepared folder with `AGENTS.md`, the
      `set-synthesis` skill, and the resolved `bg2-synthesis` path.
   4. Tell the agent something like: *“Follow the set-synthesis skill. Run
      `audit-corpus`, then `auto-review-plain`, then work `list-flagged`; `review` when
      mapper output is acceptable; `tag` only for mapper fixes or rare, high-confidence
      delivery tweaks with supported tags — stay conservative. Stop when flagged work is
      done and `progress` shows `remaining: 0`.”*
   5. The agent loops: `audit-corpus` → `auto-review-plain` → `list-flagged` (pages with
      `--after`) → for each flagged string, either `review --line <id>` (mapper output is
      fine) or `tag --batch` with final generation text (not `*stage directions*`).
      Large corpora can still be batched via `export` / `import`.
   6. Close the terminal anytime — progress is in the database. Back in the app, use the
      **Corpus audit** card and **Flagged** tab, or click **Refresh** for counts, then run
      **Generation** as usual. The Generation screen shows resolved synthesis text under
      each subtitle (Override / Mapper / Plain) without changing exported subtitles.

   Example (after launch, paths come from `AGENTS.md` in the workspace):

   ```powershell
   bg2-synthesis audit-corpus --project 1
   bg2-synthesis auto-review-plain --project 1
   bg2-synthesis list-flagged --project 1 --limit 500
   bg2-synthesis review --line 42
   bg2-synthesis tag --batch overrides.json
   bg2-synthesis progress --project 1
   ```

   Inspect one line at any time with `show --line <id>` (prints original text, resolved
   synthesis text, and whether it came from override / mapper / plain).

7. **Export.** Optionally name the pack, then click **Build export** to write a native
   WeiDU voice-pack folder (and, when WeiDU is vendored, a packaged `.zip` installer). The
   result shows the patched/deferred line counts, the install fingerprint, and buttons to
   open the pack folder or reveal the ZIP. If WeiDU wasn't fetched, the folder is still a
   valid mod — you just don't get the one-click installer ZIP.

   Pack audio is Ogg Vorbis carried in `.wav` resources, a format BG2EE plays natively;
   no decoder, EEex component, or runtime process is installed.

   If you installed a pack made by an older version that substituted placeholder text, uninstall
   that WeiDU component before installing a newly exported pack; the new pack keeps the original
   manuscript while attaching the generated audio.

8. **Transfer** (optional). Back up or restore a full **profile** (database + workspaces
   audio + agent workspace). **Export profile…** writes a `.zip` of the active profile;
   **Import profile…** creates a *new* profile from that ZIP and switches to it. Use this
   for personal machine moves and demo sandboxes — keep backups private. WeiDU packs on
   the Export screen remain the way to share a voice pack for the game.

   Profiles live under `%APPDATA%\com.bg2voicegen.desktop\profiles\<id>\`. The header
   Profile control can create, duplicate, rename, delete, and switch profiles; each may
   use the same or a different game install path.

### Headless synthesis review

Power users can run the same companion CLI directly:

```powershell
.\bg2-synthesis.exe --db "$env:APPDATA\com.bg2voicegen.desktop\profiles\1\bg2vg.db" audit-corpus --project 1
.\bg2-synthesis.exe --db "$env:APPDATA\com.bg2voicegen.desktop\profiles\1\bg2vg.db" auto-review-plain --project 1
.\bg2-synthesis.exe --db "$env:APPDATA\com.bg2voicegen.desktop\profiles\1\bg2vg.db" list-flagged --project 1
.\bg2-synthesis.exe --db "$env:APPDATA\com.bg2voicegen.desktop\profiles\1\bg2vg.db" review --line 42
.\bg2-synthesis.exe --db "$env:APPDATA\com.bg2voicegen.desktop\profiles\1\bg2vg.db" tag --batch overrides.json
```

(Or omit `--db` to use the active profile from `profiles.json` / `BG2_SYNTHESIS_PROFILE`.)
Use `export --dir <folder>` / `import <folder>` for chunked agent work. Every write
routes through the app's validation and invalidates generated clips that used the old
transcript.

### Dictionary rule curation

The companion CLI also provides `dict list`, `dict add`, `dict set`, `dict remove`,
`dict import`, `dict export`, `dict test`, and `dict scan` for pronunciation rules, plus
`tag-rule list|add|set|remove|test|reset` for OmniVoice tag rules, outside the per-line
Review workflow. See [`docs/dictionary-rules.md`](docs/dictionary-rules.md) for the JSON
format and agent-friendly workflow.

The footer status bar polls the backend and shows the current version, a "busy" indicator
while a long operation runs, and degrades to "Reconnecting…" rather than freezing.

## Dialogue coverage

The **Lines** total on the Attribution screen is **not** every spoken line in Baldur's
Gate II. It is every **NPC dialogue state** the tool finds on the path CRE →
`dialog_resref` → DLG actor state → TLK strref, **plus companion banter and
interjections** from `interdia.2da` (e.g. `BJAHEIR.dlg` for Jaheira), **plus
companion post/join dialogue** from `pdialog.2da` (e.g. `YOSHP.dlg` /
`YOSHJ.dlg` for party reform and in-party talk), **plus companion side-chain
DLGs** whose resref shares a party prefix (e.g. `jaheiraj.dlg` for Jaheira's
Harper line), in *your* install (mods included).
Player choices, journal text, combat soundsets, dream scripts from
`pdialog.2da`, and DLGs no creature or companion table points at are out of
scope.

After a full pass (Dictionary → scan → bind → generate → export), the count cards break
down like this:

- **Ready** — unvoiced NPC lines (or lines previously voiced by your own pack) that you
  can generate and export.
- **Non-spoken** — punctuation-only pauses and annotation-only states that intentionally
  receive no generated audio.
- **Blocked → already voiced** — the base game already has official VO; the pack correctly
  skips these.
- **Blocked → shared (different voice)** — the same TLK text is spoken by more than one
  character; the tool defers rather than assign one wrong voice.
- **Blocked → dynamic token** — only while Placeholders are unset. Configure stand-ins on
  the Dictionary screen and **Save + Apply** (or re-scan) to resolve tokens and move
  those lines into **Ready**.

### Real-world example

On a heavily modded **BG2EE** install tested with Placeholders configured, a full pipeline
run produced roughly:

| Bucket | Count | Share of attributed lines |
|--------|------:|--------------------------:|
| Generated by the pack (Ready) | ~22,100 | ~76% |
| Already voiced (vanilla) | ~6,200 | ~21% |
| Shared different-voice (deferred) | ~800 | ~3% |
| **Total attributed NPC lines** | **~29,100** | **100%** |

Together, the exported pack plus lines the base game already voiced covers **~97% of
in-scope NPC dialogue** for that install. The remaining ~3% is a small set of shared-strref
edge cases the tool deliberately does not auto-patch. Your totals will differ with mod load,
language, and Placeholder settings — always treat the Attribution counts as
**install-specific**, and note the fingerprint/mod setup when sharing a pack.

This is NPC conversation states plus companion banter/interjections from `interdia.2da`,
companion post/join files from `pdialog.2da`, and companion side-chain DLGs
(`jaheiraj`-style prefix orphans); it does not claim full voicing of player lines,
journals, dream scripts, or barks.

## Troubleshooting

- **Harvest warns that ffmpeg is missing / no samples appear.** The vendored tools aren't
  present. Run `.\fetch-tools.ps1` (or use the portable build, which ships them) and
  re-harvest.
- **Export produced a folder but no ZIP.** WeiDU wasn't vendored (e.g. `-SkipWeidu`, or
  `fetch-tools.ps1` hasn't run). The exported folder is still a valid WeiDU mod; run
  `.\fetch-tools.ps1` to get the packaged installer next time.
- **The engine won't start / generation is unavailable.** Make sure the tools are fetched,
  allow the first-run model download to finish, and use a machine with a supported NVIDIA
  GPU. The Generation screen surfaces any load error; the rest of the pipeline still works
  without the engine.
- **A pack warns about a fingerprint / install mismatch.** Every pack is tied to a
  game-install fingerprint (edition/version, language, mod state, source hashes). If your
  install changed, re-run the pipeline against the current state and rebuild the pack.
- **Nothing to export ("all deferred").** No lines have both a `ready` clone and generated
  audio yet — go back and bind/generate first.

## How it works

Data flows one direction: game text → SQLite → speaker attribution → harvested reference
clips → local voice clone → generated audio → a native WeiDU pack. The frontend
(SvelteKit / Svelte 5) is UI-only; every read, write, and side effect goes through a Rust
Tauri command (see `AGENTS.md` for the command-boundary rule).

**Stack:** Tauri v2, SvelteKit (Svelte 5 runes), Rust, and SQLite (bundled `rusqlite`),
with a local OmniVoice Python engine driven as a managed subprocess for generation.

## Legal & attribution

This app does not include or redistribute any Baldur's Gate assets — no game text, no game
audio, no derived voice clips. You must own a legitimate BG2EE installation; the tool
reads it locally and its output stays on your machine.

The portable build bundles a few third-party tools, whose full license texts ship under
`tools/THIRD-PARTY-LICENSES/`:

- **WeiDU 251.00** — GPLv2. Its author expressly permits redistributing an unmodified
  `WeiDU.exe` alongside a mod (the standard `setup-<mod>.exe` pattern).
- **ffmpeg 8.1.2** (gyan.dev static build) — GPLv3, with the written source offer bundled.
  Used only as a separate external process for audio decode/probe.
- **CPython** (python-build-standalone) — PSF license.

The OmniVoice engine and its Python dependencies are downloaded into `engine-runtime/` on
first run under their own upstream licenses and are not redistributed inside this build.
