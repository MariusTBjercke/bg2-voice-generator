<script lang="ts">
  import { onMount } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { invoke } from "$lib/utils/invoke";
  import { project } from "$lib/stores/project";
  import Section from "$lib/components/Section.svelte";
  import Card from "$lib/components/Card.svelte";
  import Button from "$lib/components/Button.svelte";
  import StatusBadge from "$lib/components/StatusBadge.svelte";
  import ErrorNotice from "$lib/components/ErrorNotice.svelte";
  import type { GameLanguages } from "$lib/types";

  // Install setup: pick the game folder, persist it as the `game_dir` setting,
  // then read its languages. The active locale is held in the shared `project`
  // store (the backend takes `locale` per-call on scan/harvest; there is no
  // persisted locale setting), so downstream screens reuse it without re-asking.
  const GAME_DIR_KEY = "game_dir";

  let gameDir = $state<string | null>(null);
  let languages = $state<GameLanguages | null>(null);
  let locale = $state<string | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);

  onMount(async () => {
    try {
      gameDir = (await invoke<string | null>("get_setting", { key: GAME_DIR_KEY })) ?? null;
    } catch (e) {
      error = String(e);
    }
    // Publish game_dir immediately so other tabs can load while languages resolve.
    syncStore();
    if (gameDir) await loadLanguages(gameDir);
    else syncStore();
  });

  function syncStore() {
    project.set({ gameDir, locale });
  }

  async function loadLanguages(dir: string) {
    loading = true;
    error = null;
    languages = null;
    try {
      const langs = await invoke<GameLanguages>("get_game_languages", { gameDir: dir });
      languages = langs;
      // Adopt the backend-detected active locale unless the user's pick is still valid.
      if (!locale || !langs.locales.includes(locale)) {
        locale = langs.active ?? langs.locales[0] ?? null;
      }
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
      syncStore();
    }
  }

  async function chooseFolder() {
    error = null;
    let selected: string | string[] | null;
    try {
      selected = await open({
        directory: true,
        multiple: false,
        title: "Select your Baldur's Gate II install folder",
      });
    } catch (e) {
      error = String(e);
      return;
    }
    const dir = Array.isArray(selected) ? (selected[0] ?? null) : selected;
    if (!dir) return; // user cancelled
    try {
      await invoke<void>("set_setting", { key: GAME_DIR_KEY, value: dir });
    } catch (e) {
      error = String(e);
      return;
    }
    gameDir = dir;
    locale = null; // reset so the new install's detected locale wins
    await loadLanguages(dir);
    syncStore();
  }
</script>

<Section
  title="Setup"
  description="Choose your Baldur's Gate II: Enhanced Edition install folder to begin."
>
  <Card>
    <div class="row">
      <Button onclick={chooseFolder}>Choose game folder…</Button>
      {#if gameDir}
        <StatusBadge tone="success">Folder selected</StatusBadge>
      {:else}
        <StatusBadge tone="neutral">No folder chosen yet</StatusBadge>
      {/if}
    </div>
    {#if gameDir}
      <p class="path">{gameDir}</p>
    {:else}
      <p class="hint">Pick the folder that contains your game — the one with a <code>lang/</code> directory.</p>
    {/if}
  </Card>

  <ErrorNotice message={error} />

  {#if loading}
    <Card><p>Reading install languages…</p></Card>
  {:else if languages}
    <Card>
      <h3>Language</h3>
      {#if languages.locales.length === 0}
        <p class="hint">
          No languages found under this folder's <code>lang/</code> directory. Is this a
          valid BG2EE install?
        </p>
      {:else}
        <label class="field">
          <span>Active language</span>
          <select bind:value={locale} onchange={syncStore}>
            {#each languages.locales as loc (loc)}
              <option value={loc}>{loc}{loc === languages.active ? " (detected)" : ""}</option>
            {/each}
          </select>
        </label>
      {/if}
    </Card>
  {/if}
</Section>

<style>
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  .path {
    margin: var(--space-3) 0 0;
    font-family: ui-monospace, "Cascadia Code", monospace;
    color: var(--text-muted);
    word-break: break-all;
  }
  .hint {
    margin: var(--space-3) 0 0;
    color: var(--text-muted);
  }
  h3 {
    margin: 0 0 var(--space-3);
    font-size: 1rem;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    max-width: 20rem;
  }
  select {
    font: inherit;
    background: var(--panel-2);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: var(--space-2) var(--space-3);
  }
</style>
