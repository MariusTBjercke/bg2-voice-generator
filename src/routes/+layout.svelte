<script lang="ts">
  import "../app.css";
  import { onMount } from "svelte";
  import { page } from "$app/state";
  import { invoke } from "$lib/utils/invoke";
  import { project } from "$lib/stores/project";
  import { getInstallUiPreferences, updateInstallUiPreferences } from "$lib/stores/uiPreferences";
  import { startProgressListener } from "$lib/stores/progress";
  import StatusBadge from "$lib/components/StatusBadge.svelte";
  import type { GameLanguages, HealthReport } from "$lib/types";

  let { children } = $props();

  // How often the footer re-checks backend health so it self-heals after a busy
  // run or a dev reload (item-06b).
  const HEALTH_POLL_MS = 4000;

  // Human labels for the operation ids the progress stream reports, so the footer
  // can say WHICH long-running op is keeping the backend busy.
  const OP_LABELS: Record<string, string> = {
    harvest: "harvesting",
    attribution: "scanning",
    generation: "generating",
    export: "exporting",
    transfer: "transferring",
  };

  // Pipeline stages, in dependency order. Later items fill these routes; the
  // nav already points at them so the shell is complete from the start.
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

  // Footer health indicator: polls the backend health check from every screen so a
  // busy run or a dev reload never freezes it on "Contacting backend…". UI-only —
  // the backend reports its own state via a command. `lastGood` is kept so a failed
  // poll degrades to "reconnecting" (with the last known version) rather than
  // flipping straight to "unreachable".
  let health = $state<HealthReport | null>(null);
  let lastGood = $state<HealthReport | null>(null);
  let healthError = $state<string | null>(null);

  // Any in-flight long operation (from the shared progress store) makes the footer
  // report "busy" with the op label instead of a plain "reachable" state.
  const progressStore = startProgressListener();
  let ops = $state<string[]>([]);
  progressStore.subscribe((m) => (ops = Object.keys(m)));
  const busyLabel = $derived(
    ops.length > 0 ? (OP_LABELS[ops[0]] ?? ops[0]) : null,
  );

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

  // Hydrate the shared project store on every cold start so pipeline screens can
  // load data without waiting for the Setup route's onMount (which also resolves
  // languages). game_dir is synced immediately; locale follows best-effort.
  onMount(async () => {
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
  </header>

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
