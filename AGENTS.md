# AGENTS.md

Guidance for AI coding agents working in this repository. Keep it accurate to THIS repo.

## What this is

**BG2 Voice Generator** — a Windows-first desktop app that voices the unvoiced dialogue in
**Baldur's Gate II: Enhanced Edition (BG2EE)**.
The pipeline: point it at an install (in its current modded state) → scan the game text and
attribute each line to a speaker → harvest short reference clips of that speaker's existing
official audio → bind a **local voice clone** → generate the missing lines → export a
**native WeiDU voice pack**. The exported pack copies audio into `override/` and attaches it
via WeiDU `STRING_SET`, so it plays through the game's own dialogue-audio mechanism with **no
EEex, no sidecar, no runtime TTS, and no background process** at play time. Generation uses a
**local OmniVoice engine only** (first release). Distribution is a portable ZIP (no installer).

**Stack:** Tauri v2 + Rust backend; SvelteKit (Svelte 5 runes) + Vite frontend; SQLite
(bundled `rusqlite`). A local OmniVoice Python engine is driven as a managed subprocess for
generation; `ffmpeg`/`ffprobe`, WeiDU, and a stripped CPython are vendored under `tools/`.

## Authoritative docs — read these first

- `README.md` — the user-facing guide (prereqs, build/run, how-to-use walkthrough).
- `docs/validation-checklist.md` — tiered validation gates (unit, contract, integration).
- `docs/OMNIVOICE-PERF.md` — OmniVoice batch tuning and performance notes.
- `docs/testing/e2e.md` — Playwright E2E setup and fixture rules.

## Local-only notes (not in git)

- `docs/plans/` — machine-readable implementation plans (`progress.json`, item files,
  handoffs). **Gitignored** — see `docs/plans/README.md`.
- `docs/adr/` — optional local architecture scratchpad. **Gitignored** — see
  `docs/adr/README.md`. Do not depend on ADRs being present; keep durable context in
  `README.md`, this file, and `docs/validation-checklist.md` instead.

## The pipeline (the big picture)

Data flows one direction; each GUI screen (and its route) maps to one stage:

```
profile + game install ─▶ Dictionary ─▶ Attribution ─▶ Harvest + approve ─▶ Bind ─▶ Generate ─▶ Review (opt) ─▶ Export ─▶ Transfer
   (Setup / switch)        reapply_token_   scan_attribution  harvest_references  bind_clone /   generate_line /   build_export   export_profile /
                           standins          list_blocked_*    list_reference_*    metadata_*     generate_lines_   (WeiDU)        import_profile
                                            get_attribution_  set_sample_decision  auto_bind_all  batched
                                            counts            auto_approve_*       apply_metadata_ install_engine
                                                              verify_speech (opt)  bindings
```

Setup persists the per-profile `game_dir` setting and picks the active locale; **locale is passed
per-call** (there is no persisted `active_language` key). The shell manages **folder-isolated
profiles** (`profiles.json` + `profiles/<id>/` under app data): each profile has its own DB,
workspaces, and agent workspace, and may point at the same or a different game install.
**Dictionary** configures spoken stand-ins for dynamic TLK tokens (`<CHARNAME>`, `<PRO_HISHER>`, etc.) plus
profile-scoped generation-only pronunciation rules.
**Attribution** scans CRE-owned DLGs plus companion banter/interjection DLGs from
`interdia.2da`, companion post/join DLGs from `pdialog.2da` (e.g. `yoshp.dlg`),
and prefix-matched side-chain companion DLGs (e.g. `jaheiraj.dlg`;
`scan_attribution` accepts optional `wipeDownstream` to reset
harvest/bindings/generation). **Harvest** pulls reference clips from uniquely owned main
CRE dialogue, the same companion DLG trees (quality-capped, text/duration gated), CRE
sound slots, and an Attribution **gap-fill** for speakers with Ready lines but few
automatic samples (uniquely attributed official VO only); re-running `harvest_references`
is **additive** (keeps approvals/bindings, inserts only new sound resrefs). **Binding** pairs approved reference clips with speakers via
per-speaker clones and/or demographic default pools (metadata binding). Transfer (`export_profile` /
`import_profile`) backs up or restores an entire profile folder **including local audio**
(personal machine-move / demo use). WeiDU Export packs remain the shareable in-game voice pack.

Generation resolves a separate synthesis transcript without changing `line.text`: an
enabled-by-default mapper converts supported `*...*` cues to base OmniVoice non-verbal
tags, then a string-keyed human/agent override may replace that result. The optional `/agent`
Review screen supports manual review and stages a workspace for Codex/Claude (`AGENTS.md` + `.agents/skills/` for Codex,
`CLAUDE.md` + `.claude/skills/` for Claude); agents must use `bg2-synthesis` rather than
writing SQLite directly. Review markers and overrides are local and are not transferred.

## Hard constraints (don't violate)

- **Command boundary.** Every filesystem, DB, game-resource, TTS, and export
  action goes through a `#[tauri::command]` registered in `src-tauri/src/lib.rs` and called
  from the frontend via `src/lib/utils/invoke.ts`. There is **no business logic in Svelte**
  and the frontend never touches Tauri/FS directly (asset-protocol URLs for `<audio>` go
  through `assetUrl`, gated by `assetProtocol.scope` in `tauri.conf.json`).
- **Profile backups vs public packs.** Profile Transfer ZIPs may include local
  workspace audio for personal backup/machine-move; do not treat them as a public
  distribution channel for game-derived audio. WeiDU Export packs remain the
  shareable in-game voice pack (generated derivatives only).
- **Native WeiDU export, EEex-independent packs.** Do not imply a runtime
  player or an EEex requirement for generated packs.
- **TS↔Rust mirror contract.** Every command return type has a mirrored interface in
  `src/lib/types/index.ts`; the vitest contract test (`src/lib/types/contract.test.ts`) and
  the Rust `models.rs` `contract_tests` pin the mirror. Do NOT diverge or cast command
  payloads to `any`. If a command's real shape differs, FIX the mirror (+ both anchors).
- **Param casing.** Frontend passes camelCase (`gameDir`, `speakerId`, `lineId`, `destPath`,
  `bundlePath`); Tauri maps to Rust snake_case. Enums pass as snake_case string tokens
  (e.g. `"approved"`). `AppError` serializes to a plain string; UI catches and shows
  `String(e)`.
- **Every pack is tied to an install fingerprint** (edition/version, language, mod/resource
  state, source hashes, generator/export version) and must warn/refuse on mismatch.
- **Local OmniVoice only** for the first release (no online providers).
- **No new npm/cargo deps without explicit approval** — use the package manager, never
  hand-edit manifests. Frontend styling is plain CSS on the existing dark palette; no UI
  framework.

## Code layout

**Backend (`src-tauri/src/`).** `lib.rs` owns `AppState`, plugin registration, the SQLite
bootstrap, and the full `invoke_handler` — the canonical registry of the **commands** the
backend exposes (add a command there or the UI can't reach it). `main.rs` is the thin binary
entry. `error.rs` = `AppError` (serializes to a plain string). `models.rs` = the shared
structs mirrored as TS, with the `contract_tests` anchor. `paths.rs` = portable-vs-dev
`ToolLayout` resolution. `fingerprint/` = the install fingerprint.

- `commands/` — thin Tauri wrappers that use the writer DB mutex or independent read-only
  WAL connections and delegate to the domain
  modules: `startup` (`health_check`), `settings`, `extractor` (languages + TLK/DLG/CRE
  inspectors), `attribution`, `harvest`, `generate` (engine + `install_engine` + `bind_clone`
  + `generate_line` + `generate_lines_batched` + `assign_fallback_voices`),
  `metadata_binding` (demographic pools + effective speaker bindings),
  `synthesis` (preview/override/review/corpus audit), `agent` (workspace + launcher), `export`,
  `transfer`, and `progress` (the `operation://progress` events + `cancel_operation` +
  `CancelRegistry`).
- `db/` — SQLite: `schema`, `queries`, `attribution`, `harvest`, `generation`, `export`,
  `speaker_groups` (identity-group bucketing + clone propagation).
- `extractor/` — native Infinity Engine parsing (`key`/`bif`/`tlk`/`dlg`/`cre`/`lang`/
  `resource`/`restype`/`bytes`/`tokens`/`attribution`/`views`). No subprocess/CSV/.NET.
- `audio/` — `ffmpeg` wrapper, `wav`, candidate selection + `scoring`.
- `voices/` — reference-clip harvesting (`harvest.rs`).
- `generator/` — `binding`, `clone`, `batch`, `run` (single-line + batched generation).
- `synthesis.rs` + `cli.rs` — generation-text precedence, string-keyed overrides/review
  state, and the shared implementation behind the `bg2-synthesis` companion binary.
- `agent_templates.rs` / `agent_templates/` — project workspace docs (`AGENTS.md`,
  `CLAUDE.md`) + embedded `set-synthesis` skill staged to `.agents/skills/` and
  `.claude/skills/` by the Review screen.
- `tts/` — OmniVoice engine supervisor (`engine.rs`, `install.rs`, `omnivoice.rs`): lazy
  boot, in-app provisioning, health gate, kill-on-exit (`AppState.omnivoice`, shut down
  from the `RunEvent` handler in `lib.rs`).
- `export/` — WeiDU pack build (`build`, `tp2`, `manifest`, `docs`, `plan`, `resref`, `zip`).
- `profile.rs` / `profile_transfer.rs` — folder-isolated profiles + full-profile ZIP
  backup/import (includes workspace audio). `transfer/` — zip-slip path sanitizer.
  `backup/` — backups.

**Frontend (`src/`).** One route per pipeline stage under `src/routes/`: `/` (Setup),
`/dictionary`, `/attribution`, `/harvest`, `/binding`, `/generation`, `/agent`, `/export`, `/transfer`.
`+layout.svelte` is the shell (header + pipeline nav with active-link highlight, centered
`<main>`, footer status bar that polls `health_check` and shows busy/progress);
`+layout.ts` sets the SPA config. Adapter = `adapter-static` (SPA, `ssr=false`, prerendered).

## Frontend (UI) architecture

The frontend is **UI-only**. Svelte 5 runes throughout (`$state`/`$derived`/`$effect`/
`$props`/`$bindable`). Key `src/lib/` pieces:

- `utils/invoke.ts` — the single backend chokepoint: `invoke<T>()`, `listen<T>()`, and
  `assetUrl()` (wraps `convertFileSrc`). Components never import Tauri APIs directly.
- `types/index.ts` — the interfaces mirroring the Rust command return types; `contract.test.ts`
  pins the mirror (keep it green).
- `components/` — the shared plain-CSS primitives: `Button`, `Card`, `Section`,
  `StatusBadge`, `ErrorNotice`, `Pager` (display-only paging), `ProgressBar` (determinate +
  indeterminate, respects `prefers-reduced-motion`), `SearchFilterBar`, `SearchableMultiSelect`.
- `stores/` — `project` (`{gameDir, locale}`), `results` (a UI-only cache keyed by `gameDir`,
  reset on install change — never a source of truth), `progress` (the lazy
  `operation://progress` listener, keyed by op, terminal phase clears the entry), `filters`
  (per-screen search/filter state).
- `filters/` — pure filter helpers + configs used by list screens (`generation.ts`, etc.).
- `app.css` — `:root` design tokens: the dark palette (`#0c0d0b` bg / `#e6e6e6` text /
  `#2a2d27` borders) + spacing/radii. Plain CSS, no framework.

Each screen must handle all four states: loading, empty, error (`catch` → `String(e)`), and
success. Long lists page (100/50 per view); long ops show a `ProgressBar` (+ Cancel where the
backend emits progress: harvest/attribution/speech-verify/generation/export/transfer/engine
install).

## Testing

Gate pyramid (run the smallest tier that covers your change):

| Tier | When | Command |
|------|------|---------|
| Types | Any frontend touch | `npm run check` |
| Contracts / pure TS | Types, mirrors, helpers | `npm run test` |
| Backend | `src-tauri/**` | `cargo test --lib` |
| **UI E2E** | Routes, components, `app.css`, E2E fixtures | `npm run test:e2e` |
| Full shell | Tauri-only behavior, asset URLs, dialogs | `npm run tauri dev` |

**Run `npm run test:e2e` after changing** `src/routes/**`, `src/lib/components/**`,
`src/app.css`, or anything under `e2e/`. Do **not** require E2E for Rust-only or
contract-only edits (`cargo test` / `npm run test` suffice).

E2E uses Playwright + Tauri `mockIPC` against `vite dev` (no desktop shell). See
[`docs/testing/e2e.md`](docs/testing/e2e.md) for setup, fixture rules, and how to add
screen tests. After intentional visual changes, run `npm run test:e2e:update-snapshots`
and commit the baseline PNGs. Cursor's browser is fine for exploratory checks;
Playwright is the repeatable gate.

E2E is **not** in `build-portable.ps1` (separate `npm run test:e2e`).

## Commands

```powershell
npm install                                            # pinned dependency versions
.\fetch-tools.ps1                                      # vendor WeiDU + ffmpeg/ffprobe + CPython into tools/ (-Force / -SkipWeidu)
npm run tauri dev                                      # dev: Vite + Tauri shell (opens the app)
npm run check                                          # frontend type-check (svelte-kit sync && svelte-check)
npm run test                                           # frontend tests (vitest, node env; contract test)
npm run test:e2e                                       # browser UI E2E (Playwright + mock IPC; see docs/testing/e2e.md)
cargo check --manifest-path src-tauri\Cargo.toml       # backend compile
cargo test  --lib --manifest-path src-tauri\Cargo.toml # backend unit tests
.\build-portable.ps1                                   # gates + build exe + stage tools/ + zip + deploy -> dist\
```

`npm run check` and `npm run test` are the gates after any UI change; add `cargo check` +
`cargo test --lib` when Rust is touched. `build-portable.ps1` runs all four gates before it
builds, produces `dist\BG2VoiceGenerator-<version>.zip`, and deploys `dist\portable\`.
Always build through `cargo tauri build` (via `build-portable.ps1`), never plain `cargo
build` — the latter won't embed the frontend.

## Versioning

**Single source of truth: `package.json` `version`** (currently `0.1.0`). When bumping, also
update `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml` to match; `build-portable.ps1`
names the ZIP from `package.json`.

## Git and commits

Use [Conventional Commits](https://www.conventionalcommits.org/) for every commit from the
initial public release onward.

- **Format:** `<type>[optional scope]: <description>`
- **Types:** `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`
- **Scope (optional):** area touched, e.g. `extractor`, `export`, `ui`, `engine`
- **Description:** imperative mood, lowercase, no trailing period
- **Body (optional):** explain *why* when the subject alone is not enough
- **Breaking changes:** add `BREAKING CHANGE:` in the body or use `type(scope)!:`

Examples:

- `feat(export): add fingerprint mismatch warning`
- `fix(harvest): clamp reference clip duration at 8s`
- `docs: update validation checklist for tier 2`
- `chore: ignore local execution plans`

Do not create commits unless the user asks. When asked, one logical change per commit; split
unrelated work.
