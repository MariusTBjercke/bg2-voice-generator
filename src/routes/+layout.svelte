<script lang="ts">
  import "../app.css";
  import { onMount } from "svelte";
  import { page } from "$app/state";
  import { invoke } from "$lib/utils/invoke";
  import { project } from "$lib/stores/project";
  import {
    profiles,
    profileGeneration,
    refreshProfiles,
    switchToProfile,
    createProfile,
    renameProfile,
    duplicateProfile,
    deleteProfile,
  } from "$lib/stores/profiles";
  import { getInstallUiPreferences, updateInstallUiPreferences } from "$lib/stores/uiPreferences";
  import { startProgressListener } from "$lib/stores/progress";
  import { WORKFLOW_STAGES, WORKFLOW_UTILITIES } from "$lib/navigation/workflow";
  import Icon from "$lib/components/Icon.svelte";
  import ProfilePicker from "$lib/components/ProfilePicker.svelte";
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
  let profileMenuOpen = $state(false);
  let profilePickerOpen = $state(false);

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

  async function onProfileSelect(id: string) {
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

  $effect(() => {
    if (profilePickerOpen) profileMenuOpen = false;
  });

  function onProfileMenuToggle(event: Event) {
    const details = event.currentTarget as HTMLDetailsElement;
    if (details.open) profilePickerOpen = false;
  }

  async function onCreateProfile() {
    profileBusy = true;
    profileError = null;
    try {
      const created = await createProfile();
      await switchToProfile(created.id);
      profileMenuOpen = false;
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
      profileMenuOpen = false;
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
      profileMenuOpen = false;
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
      profileMenuOpen = false;
    } catch (e) {
      profileError = String(e);
    } finally {
      profileBusy = false;
    }
  }
</script>

<div class="shell">
  <a class="skip-link" href="#main-content">Skip to content</a>
  <header class="topbar">
    <div class="topbar-main">
      <a class="brand" href="/" aria-label="BG2 Voice Generator home" data-sveltekit-preload-data="off">
        <img src="/app-icon.png" alt="" />
        <span class="brand-copy">
          <h1 class="brand-title">BG2 Voice Generator</h1>
          <span class="brand-tagline">Native voices for the Forgotten Realms</span>
        </span>
      </a>

      <div class="profile-bar">
        <span class="profile-label">Active profile</span>
        <ProfilePicker
          profiles={profileList}
          activeId={activeProfile?.id ?? null}
          disabled={profileBusy || profileList.length === 0}
          bind:open={profilePickerOpen}
          onselect={onProfileSelect}
        />
        <details class="profile-menu" bind:open={profileMenuOpen} ontoggle={onProfileMenuToggle}>
          <summary aria-label="Manage profiles">
            <Icon name="settings" size={16} />
            <span>Manage</span>
            <Icon name="chevron-down" size={15} />
          </summary>
          <div class="profile-menu-panel">
            {#if renameOpen && activeProfile}
              <form class="rename-form" onsubmit={(event) => { event.preventDefault(); void submitRename(); }}>
                <label for="rename-input">Profile name</label>
                <input id="rename-input" bind:value={renameValue} disabled={profileBusy} />
                <div class="profile-menu-actions">
                  <button type="submit" class="profile-btn primary" disabled={profileBusy || !renameValue.trim()}>Save</button>
                  <button type="button" class="profile-btn" disabled={profileBusy} onclick={() => (renameOpen = false)}>Cancel</button>
                </div>
              </form>
            {:else}
              <button type="button" class="profile-menu-item" disabled={profileBusy} onclick={onCreateProfile}>New empty profile</button>
              <button type="button" class="profile-menu-item" disabled={profileBusy || !activeProfile} onclick={onDuplicateProfile}>Duplicate profile</button>
              <button type="button" class="profile-menu-item" disabled={profileBusy || !activeProfile} onclick={openRename}>Rename profile</button>
              <div class="menu-separator"></div>
              <button type="button" class="profile-menu-item danger" disabled={profileBusy || !activeProfile || profileList.length <= 1} onclick={onDeleteProfile}>Delete profile…</button>
            {/if}
          </div>
        </details>
      </div>
    </div>

    <nav aria-label="Workflow">
      <div class="workflow-links">
        {#each WORKFLOW_STAGES as item (item.href)}
          <a
            href={item.href}
            class:active={pathname === item.href}
            aria-current={pathname === item.href ? "page" : undefined}
            data-sveltekit-preload-data="off"
          >
            <span class="step-number">{item.step}</span>
            <span>{item.label}</span>
            {#if item.optional}<span class="optional-label">Optional</span>{/if}
          </a>
        {/each}
      </div>
      <div class="utility-links">
        {#each WORKFLOW_UTILITIES as item (item.href)}
          <a
            href={item.href}
            class:active={pathname === item.href}
            aria-current={pathname === item.href ? "page" : undefined}
            data-sveltekit-preload-data="off"
          >{item.label}</a>
        {/each}
      </div>
    </nav>
  </header>

  {#if profileError}
    <div class="profile-error">{profileError}</div>
  {/if}

  {#key $profileGeneration}
    <main id="main-content" tabindex="-1">{@render children()}</main>
  {/key}

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
    position: sticky;
    top: 0;
    z-index: 20;
    border-bottom: 1px solid var(--border);
    background: color-mix(in srgb, var(--panel) 94%, transparent);
    box-shadow: var(--shadow-sm);
    backdrop-filter: blur(12px);
  }
  .skip-link {
    position: fixed;
    top: var(--space-2);
    left: var(--space-2);
    z-index: 100;
    transform: translateY(-160%);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    background: var(--accent);
    color: var(--accent-ink);
    text-decoration: none;
  }
  .skip-link:focus { transform: translateY(0); }
  .topbar-main {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-5);
    min-height: 4rem;
    padding: var(--space-2) clamp(var(--space-4), 3vw, var(--space-6));
  }
  .brand {
    display: inline-flex;
    align-items: center;
    gap: var(--space-3);
    min-width: 0;
    color: var(--text);
    text-decoration: none;
  }
  .brand img {
    width: 2.75rem;
    height: 2.75rem;
    border-radius: 50%;
    box-shadow: 0 0 0 1px color-mix(in srgb, var(--accent) 35%, transparent), var(--shadow-sm);
  }
  .brand-copy { display: flex; flex-direction: column; min-width: 0; }
  .brand-title {
    margin: 0;
    font-family: var(--font-display);
    font-size: 1.22rem;
    font-weight: 700;
    letter-spacing: 0.015em;
    white-space: nowrap;
  }
  .brand-tagline {
    color: var(--text-faint);
    font-size: 0.72rem;
    letter-spacing: 0.04em;
    white-space: nowrap;
  }
  nav {
    display: flex;
    align-items: stretch;
    min-width: 0;
    padding-left: clamp(var(--space-4), 3vw, var(--space-6));
    border-top: 1px solid color-mix(in srgb, var(--border) 70%, transparent);
    background: var(--panel-deep);
    overflow: hidden;
  }
  .workflow-links,
  .utility-links { display: flex; align-items: stretch; }
  .workflow-links {
    flex: 1 1 auto;
    min-width: 0;
    gap: var(--space-1);
    overflow-x: auto;
    scrollbar-width: thin;
  }
  .utility-links {
    flex: 0 0 auto;
    border-left: 1px solid var(--border);
    background: var(--panel-deep);
    box-shadow: -8px 0 14px rgba(0, 0, 0, 0.16);
  }
  .utility-links a {
    justify-content: center;
    min-width: 7.5rem;
    padding-inline: clamp(var(--space-4), 3vw, var(--space-6));
  }
  nav a {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--text-muted);
    text-decoration: none;
    padding: 0.7rem var(--space-3);
    border-bottom: 2px solid transparent;
    font-size: 0.82rem;
    white-space: nowrap;
  }
  nav a:hover {
    color: var(--text);
    background: var(--panel-2);
  }
  nav a.active {
    color: var(--accent-light);
    border-bottom-color: var(--accent);
    background: linear-gradient(180deg, transparent, var(--accent-wash));
  }
  .step-number {
    display: grid;
    place-items: center;
    width: 1.25rem;
    height: 1.25rem;
    border: 1px solid var(--border-strong);
    border-radius: 50%;
    color: var(--text-faint);
    font-size: 0.68rem;
    font-variant-numeric: tabular-nums;
  }
  nav a.active .step-number {
    border-color: var(--accent);
    background: var(--accent);
    color: var(--accent-ink);
  }
  .optional-label {
    color: var(--text-faint);
    font-size: 0.62rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
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
    font-size: var(--control-font-size);
    color: var(--text-muted);
  }
  .profile-menu { position: relative; }
  .profile-menu summary {
    display: inline-flex;
    align-items: center;
    gap: var(--control-icon-gap);
    min-height: var(--control-height);
    list-style: none;
    cursor: pointer;
    padding: 0 var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--control-radius);
    background: var(--panel-2);
    color: var(--text);
    font-size: var(--control-font-size);
    font-weight: var(--control-font-weight);
  }
  .profile-menu summary::-webkit-details-marker { display: none; }
  .profile-menu[open] summary,
  .profile-menu summary:hover { border-color: var(--accent); }
  .profile-menu-panel {
    position: absolute;
    top: calc(100% + var(--space-2));
    right: 0;
    z-index: 30;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    width: 15rem;
    padding: var(--space-2);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius);
    background: var(--panel-raised);
    box-shadow: var(--shadow-lg);
  }
  .profile-menu-item {
    width: 100%;
    min-height: var(--control-height);
    padding: 0 var(--space-3);
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text);
    font-size: var(--control-font-size);
    text-align: left;
    cursor: pointer;
  }
  .profile-menu-item:hover:not(:disabled) { background: var(--panel-2); }
  .profile-menu-item.danger { color: var(--danger); }
  .profile-menu-item:disabled { opacity: 0.45; cursor: not-allowed; }
  .menu-separator { height: 1px; margin: var(--space-1) 0; background: var(--border); }
  .rename-form { display: flex; flex-direction: column; gap: var(--space-2); padding: var(--space-1); }
  .rename-form label { color: var(--text-muted); font-size: 0.76rem; }
  .rename-form input { width: 100%; box-sizing: border-box; }
  .profile-menu-actions { display: flex; gap: var(--space-2); }
  .profile-btn {
    background: var(--panel-2);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--control-radius);
    min-height: var(--control-height);
    padding: 0 var(--space-3);
    font-size: var(--control-font-size);
    font-weight: var(--control-font-weight);
    cursor: pointer;
  }
  .profile-btn.primary { background: var(--accent); color: var(--accent-ink); border-color: var(--accent); }
  .profile-btn:hover:not(:disabled) {
    border-color: var(--accent);
  }
  .profile-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .profile-error {
    padding: var(--space-2) var(--space-5);
    background: #3a1a1a;
    color: #f0c0c0;
    font-size: 0.85rem;
  }
  main {
    flex: 1;
    width: 100%;
    max-width: 1360px;
    margin: 0 auto;
    padding: clamp(var(--space-5), 3vw, 2.5rem) clamp(var(--space-4), 3vw, var(--space-6));
    box-sizing: border-box;
  }
  .statusbar {
    position: sticky;
    bottom: 0;
    z-index: 15;
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-5);
    border-top: 1px solid var(--border);
    background: color-mix(in srgb, var(--panel-deep) 95%, transparent);
    backdrop-filter: blur(10px);
    font-size: 0.8rem;
    color: var(--text-muted);
  }
  .detail {
    color: var(--text-muted);
  }
  @media (max-width: 1080px) {
    .brand-tagline,
    .optional-label { display: none; }
    nav a { padding-inline: var(--space-2); }
  }
  @media (max-width: 760px) {
    .topbar-main { align-items: flex-start; flex-direction: column; gap: var(--space-2); }
    .profile-bar { width: 100%; margin-left: 0; }
    .brand img { width: 2.25rem; height: 2.25rem; }
    .brand-tagline { display: none; }
  }
</style>
