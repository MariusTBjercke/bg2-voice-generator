# Browser E2E tests (Playwright)

Playwright drives the SvelteKit UI in a real Chromium browser **without** the Tauri
shell, a BG2 install, or OmniVoice. Commands are mocked through Tauri's official
[`mockIPC`](https://v2.tauri.app/reference/javascript/api/namespacemocks/) API,
registered from [`src/hooks.client.ts`](../src/hooks.client.ts) when
`VITE_E2E_MOCK=1`.

This tier complements:

| Tier | Command | Proves |
|------|---------|--------|
| Types | `npm run check` | Svelte/TS correctness |
| Contracts + logic | `npm run test` | TS ↔ Rust mirrors, pure helpers |
| Backend | `cargo test --lib` | Pipeline / DB / export rules |
| **UI E2E** | **`npm run test:e2e`** | **Routes render, nav, filters, key badges/buttons** |
| Full desktop | `npm run tauri dev` | Real `invoke`, asset protocol, native dialogs |
| Game pipeline | Manual | Voice quality, WeiDU in a real install |

E2E is **not** part of `build-portable.ps1` (browser install + extra time). Run it
after UI/layout changes.

## One-time setup

```powershell
npm install
npx playwright install chromium
```

## Running

```powershell
npm run test:e2e                      # headless
npm run test:e2e:ui                  # interactive debugger
npm run test:e2e:update-snapshots      # after intentional visual changes
```

Playwright starts `npm run dev` with `VITE_E2E_MOCK=1` on **http://localhost:1420**
(must match [`vite.config.js`](../vite.config.js) `server.port`).

## Architecture

```
  e2e/
  fixtures/
    data.ts       # Typed fixture payloads (mirrors src/lib/types)
    commands.ts   # mockIPC handler (throw on unknown commands)
  helpers/
    bootstrap.ts  # Visit Setup, wait for mocked install hydration
  stubs/          # Vite aliases for plugin-dialog / plugin-opener
  shell.spec.ts       # App shell + nav
  attribution.spec.ts
  harvest.spec.ts     # Harvest identity groups + sample list
  binding.spec.ts
  generation.spec.ts  # Engine card + synthesis preview
  agent.spec.ts       # Human/AI review workspace + corpus audit cards
```

**Bootstrap flow:** most screens need `$project.gameDir`. Tests call
`bootstrapProject(page)` (visits `/`, waits for the fixture folder + locale), then
navigate via the pipeline nav.

**Plugin stubs:** Setup/Transfer/Export import `@tauri-apps/plugin-dialog` and
`@tauri-apps/plugin-opener` directly. When `VITE_E2E_MOCK=1`, Vite aliases those
packages to `e2e/stubs/*` (see [`vite.config.js`](../vite.config.js)).

## Adding a test for a new screen

1. List every `invoke(...)` the route calls on mount (and on the interaction you
   want to test).
2. Add handlers in [`e2e/fixtures/commands.ts`](../e2e/fixtures/commands.ts).
   Unknown commands **throw** so missing mocks fail loudly.
3. Add fixture data in [`e2e/fixtures/data.ts`](../e2e/fixtures/data.ts) aligned
   with [`src/lib/types/index.ts`](../src/lib/types/index.ts).
4. Write a spec under `e2e/`, starting with `bootstrapProject` + `goTo`.
5. Run `npm run test:e2e`.

## Snapshot policy

Layout snapshot baselines live under `e2e/*-snapshots/` (`shell.spec.ts` top bar;
`binding.spec.ts` desktop + narrow). Keep snapshots **minimal** (Windows font rendering
can differ across machines). After an intentional layout change:

```powershell
npm run test:e2e:update-snapshots
```

Commit the updated PNG with the UI change.

## What E2E does not cover

- Real Tauri `invoke`, SQLite, filesystem, game-resource parsing
- OmniVoice synthesis, harvest decode, WeiDU export IO
- Native folder/file dialogs (stubbed to fixture paths)

Use `cargo test` for pipeline logic and `npm run tauri dev` for full-shell smoke.
