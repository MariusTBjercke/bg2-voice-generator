<script lang="ts">
  import "../app.css";
  import { onMount } from "svelte";
  import { page } from "$app/state";
  import { invoke } from "$lib/utils/invoke";
  import { project } from "$lib/stores/project";
  import {
    profiles,
    refreshProfiles,
    switchToProfile,
    createProfile,
    renameProfile,
    duplicateProfile,
    deleteProfile,
  } from "$lib/stores/profiles";
  import { getInstallUiPreferences, updateInstallUiPreferences } from "$lib/stores/uiPreferences";
  import { startProgressListener } from "$lib/stores/progress";
  import StatusBadge from "$lib/components/StatusBadge.svelte";
  import type { GameLanguages, HealthReport } from "$lib/types";

  let { children } = $props();

  const HEALTH_POLL_MS = 4000;

  const OP_LABELS: Record<string, string> = {
    harvest: "harvesting",
    attribution: "scanning",
    generation: "generating",
    export: "exporting",
    transfer: "transferring",
  };

  const nav = [
    { href: "/", label: "Setup" },
    { href: "/dictionary", label: "Dictionary" },
    { href: "/attribution", label: "Attribution" },
    { href: "/harvest", label: "Harvest" },
    { href: "/binding", label: "Binding" },
    { href: "/generation", label: "Generation" },
    { href: "/agent", label: "Review" },
    { href: "/export", label: "Export" },
    { href: "/transfer", label: "Transfer" },
  ];

  let health = $state<HealthReport | null>(null);
  let lastGood = $state<HealthReport | null>(null);
  let healthError = $state<string | null>(null);

  const progressStore = startProgressListener();
  let ops = $state<string[]>([]);
  progressStore.subscribe((m) => (ops = Object.keys(m)));
  const busyLabel = $derived(
    ops.length > 0 ? (OP_LABELS[ops[0]] ?? ops[0]) : null,
  );

  let profileBusy = $state(false);
  let profileError = $state<string | null>(null);
  let renameOpen = $state(false);
  let renameValue = $state("");

  const activeProfile = $derived($profiles.active);
  const profileList = $derived($profiles.registry?.profiles ?? []);

  async function pollHealth() {
    try {
      health = await invoke<HealthReport>("health_check");
      lastGood = health;
      healthError = null;
    } catch (e) {
      health = null;
      healthError = String(e);
    }
  }

  $effect(() => {
    void pollHealth();
    const t = setInterval(() => void pollHealth(), HEALTH_POLL_MS);
    return () => clearInterval(t);
  });

  let pathname = $derived(page.url.pathname);

  onMount(async () => {
    await refreshProfiles();
    try {
      const gameDir = (await invoke<string | null>("get_setting", { key: "game_dir" })) ?? null;
      if (!gameDir) return;
      project.update((p) => ({ ...p, gameDir }));
      try {
        const langs = await invoke<GameLanguages>("get_game_languages", { gameDir });
        const preferred = getInstallUiPreferences(gameDir).locale;
        const locale = preferred && langs.locales.includes(preferred)
          ? preferred
          : (langs.active ?? langs.locales[0] ?? null);
        project.update((p) => ({ ...p, locale }));
        updateInstallUiPreferences(gameDir, (current) => ({ ...current, locale }));
      } catch {
        // locale stays null; commands fall back to the install default
      }
    } catch {
      // Setup screen surfaces pick/read errors on visit
    }
  });

  async function onProfileSelect(event: Event) {
    const id = (event.currentTarget as HTMLSelectElement).value;
    if (!id || id === activeProfile?.id) return;
    profileBusy = true;
    profileError = null;
    try {
      await switchToProfile(id);
    } catch (e) {
      profileError = String(e);
      await refreshProfiles();
    } finally {
      profileBusy = false;
    }
  }

  async function onCreateProfile() {
    profileBusy = true;
    profileError = null;
    try {
      const created = await createProfile();
      await switchToProfile(created.id);
    } catch (e) {
      profileError = String(e);
    } finally {
      profileBusy = false;
    }
  }

  async function onDuplicateProfile() {
    if (!activeProfile) return;
    profileBusy = true;
    profileError = null;
    try {
      const dup = await duplicateProfile(activeProfile.id);
      await switchToProfile(dup.id);
    } catch (e) {
      profileError = String(e);
    } finally {
      profileBusy = false;
    }
  }

  function openRename() {
    renameValue = activeProfile?.name ?? "";
    renameOpen = true;
  }

  async function submitRename() {
    if (!activeProfile) return;
    profileBusy = true;
    profileError = null;
    try {
      await renameProfile(activeProfile.id, renameValue);
      renameOpen = false;
    } catch (e) {
      profileError = String(e);
    } finally {
      profileBusy = false;
    }
  }

  async function onDeleteProfile() {
    if (!activeProfile || profileList.length <= 1) return;
    const id = activeProfile.id;
    const other = profileList.find((p) => p.id !== id);
    if (!other) return;
    if (!confirm(`Delete profile “${activeProfile.name}”? This cannot be undone.`)) return;
    profileBusy = true;
    profileError = null;
    try {
      await switchToProfile(other.id);
      await deleteProfile(id);
    } catch (e) {
      profileError = String(e);
    } finally {
      profileBusy = false;
    }
  }
</script>

<div class="shell">
  <header class="topbar">
    <h1>BG2 Voice Generator</h1>
    <nav>
      {#each nav as item (item.href)}
        <a href={item.href} class:active={pathname === item.href} data-sveltekit-preload-data="off">
          {item.label}
        </a>
      {/each}
    </nav>
    <div class="profile-bar">
      <label class="profile-label" for="profile-select">Profile</label>
      <select
        id="profile-select"
        class="profile-select"
        disabled={profileBusy || profileList.length === 0}
        value={activeProfile?.id ?? ""}
        onchange={onProfileSelect}
      >
        {#each profileList as p (p.id)}
          <option value={p.id}>{p.name}</option>
        {/each}
      </select>
      <button type="button" class="profile-btn" disabled={profileBusy} onclick={onCreateProfile} title="New empty profile">
        New
      </button>
      <button type="button" class="profile-btn" disabled={profileBusy || !activeProfile} onclick={onDuplicateProfile} title="Duplicate active profile">
        Duplicate
      </button>
      <button type="button" class="profile-btn" disabled={profileBusy || !activeProfile} onclick={openRename} title="Rename active profile">
        Rename
      </button>
      <button
        type="button"
        class="profile-btn danger"
        disabled={profileBusy || !activeProfile || profileList.length <= 1}
        onclick={onDeleteProfile}
        title="Delete active profile"
      >
        Delete
      </button>
    </div>
  </header>

  {#if profileError}
    <div class="profile-error">{profileError}</div>
  {/if}

  {#if renameOpen && activeProfile}
    <div class="rename-bar">
      <label for="rename-input">Rename profile</label>
      <input id="rename-input" bind:value={renameValue} disabled={profileBusy} />
      <button type="button" class="profile-btn" disabled={profileBusy || !renameValue.trim()} onclick={submitRename}>
        Save
      </button>
      <button type="button" class="profile-btn" disabled={profileBusy} onclick={() => (renameOpen = false)}>
        Cancel
      </button>
    </div>
  {/if}

  <main>{@render children()}</main>

  <footer class="statusbar">
    {#if health}
      {#if busyLabel}
        <StatusBadge tone="info">Backend v{health.app_version} (busy)</StatusBadge>
        <span class="detail">{busyLabel}…</span>
      {:else}
        <StatusBadge tone="success">Backend v{health.app_version}</StatusBadge>
        <span class="detail">schema {health.schema_version}</span>
      {/if}
      {#if activeProfile}
        <span class="detail">· {activeProfile.name}</span>
      {/if}
    {:else if lastGood}
      <StatusBadge tone="warn">Reconnecting…</StatusBadge>
      <span class="detail">
        {busyLabel ? `backend busy (${busyLabel}…)` : `last seen v${lastGood.app_version}`}
      </span>
    {:else if healthError}
      <StatusBadge tone="danger">Backend unreachable</StatusBadge>
      <span class="detail">{healthError}</span>
    {:else}
      <StatusBadge tone="neutral">Contacting backend…</StatusBadge>
    {/if}
  </footer>
</div>

<style>
  .shell {
    display: flex;
    flex-direction: column;
    min-height: 100vh;
  }
  .topbar {
    display: flex;
    align-items: center;
    gap: var(--space-6);
    padding: var(--space-3) var(--space-5);
    border-bottom: 1px solid var(--border);
    background: var(--panel);
    flex-wrap: wrap;
    row-gap: var(--space-2);
  }
  h1 {
    margin: 0;
    font-size: 1.05rem;
    white-space: nowrap;
  }
  nav {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    flex: 1 1 36rem;
    min-width: 0;
  }
  nav a {
    color: var(--text-muted);
    text-decoration: none;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    font-size: 0.9rem;
  }
  nav a:hover {
    color: var(--text);
    background: var(--panel-2);
  }
  nav a.active {
    color: var(--accent-ink);
    background: var(--accent);
  }
  .profile-bar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
    margin-left: auto;
    flex: 0 1 auto;
  }
  .profile-label {
    font-size: 0.8rem;
    color: var(--text-muted);
  }
  .profile-select {
    background: var(--panel-2);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: var(--space-1) var(--space-2);
    font-size: 0.85rem;
    max-width: 12rem;
  }
  .profile-btn {
    background: var(--panel-2);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: var(--space-1) var(--space-2);
    font-size: 0.8rem;
    cursor: pointer;
  }
  .profile-btn:hover:not(:disabled) {
    border-color: var(--accent);
  }
  .profile-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .profile-btn.danger:hover:not(:disabled) {
    border-color: var(--danger, #c44);
    color: var(--danger, #c44);
  }
  .profile-error {
    padding: var(--space-2) var(--space-5);
    background: #3a1a1a;
    color: #f0c0c0;
    font-size: 0.85rem;
  }
  .rename-bar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-5);
    border-bottom: 1px solid var(--border);
    background: var(--panel);
    font-size: 0.85rem;
  }
  .rename-bar input {
    background: var(--panel-2);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: var(--space-1) var(--space-2);
    min-width: 12rem;
  }
  main {
    flex: 1;
    width: 100%;
    max-width: 1200px;
    margin: 0 auto;
    padding: var(--space-6) var(--space-5);
    box-sizing: border-box;
  }
  .statusbar {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-5);
    border-top: 1px solid var(--border);
    background: var(--panel);
    font-size: 0.8rem;
    color: var(--text-muted);
  }
  .detail {
    color: var(--text-muted);
  }
</style>
