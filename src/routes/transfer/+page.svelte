<script lang="ts">
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { invoke } from "$lib/utils/invoke";
  import { profiles, adoptActiveProfile } from "$lib/stores/profiles";
  import Section from "$lib/components/Section.svelte";
  import Card from "$lib/components/Card.svelte";
  import Button from "$lib/components/Button.svelte";
  import StatusBadge from "$lib/components/StatusBadge.svelte";
  import ErrorNotice from "$lib/components/ErrorNotice.svelte";
  import ProgressBar from "$lib/components/ProgressBar.svelte";
  import WorkflowCallout from "$lib/components/WorkflowCallout.svelte";
  import { progress } from "$lib/stores/progress";
  import type { ProfileExportResult, ProfileImportResult } from "$lib/types";

  // Profile backup/restore: full ZIP of the profile folder (DB + workspaces audio
  // + agent workspace). For personal machine moves and demos — not for public
  // redistribution of game-derived audio. WeiDU Export packs remain the shareable path.

  const active = $derived($profiles.active);
  const transferProgress = $derived($progress.transfer ?? null);
  const transferBusy = $derived(transferProgress !== null);

  let exporting = $state(false);
  let exportResult = $state<ProfileExportResult | null>(null);
  let exportError = $state<string | null>(null);

  let importing = $state(false);
  let importResult = $state<ProfileImportResult | null>(null);
  let importError = $state<string | null>(null);

  async function exportActiveProfile() {
    if (!active || exporting) return;
    exportError = null;
    let destPath: string | null;
    try {
      destPath = await save({
        title: "Save profile backup",
        defaultPath: `bg2vg-profile-${active.name.replace(/[^\w\-]+/g, "_")}.zip`,
        filters: [{ name: "Profile backup", extensions: ["zip"] }],
      });
    } catch (e) {
      exportError = String(e);
      return;
    }
    if (!destPath) return;
    exporting = true;
    exportResult = null;
    try {
      exportResult = await invoke<ProfileExportResult>("export_profile", {
        destPath,
        profileId: active.id,
      });
    } catch (e) {
      exportError = String(e);
    } finally {
      exporting = false;
    }
  }

  async function importProfileBundle() {
    if (importing) return;
    importError = null;
    let selected: string | string[] | null;
    try {
      selected = await open({
        title: "Choose a profile backup",
        multiple: false,
        filters: [{ name: "Profile backup", extensions: ["zip"] }],
      });
    } catch (e) {
      importError = String(e);
      return;
    }
    const bundlePath = Array.isArray(selected) ? (selected[0] ?? null) : selected;
    if (!bundlePath) return;
    importing = true;
    importResult = null;
    try {
      importResult = await invoke<ProfileImportResult>("import_profile", {
        bundlePath,
        name: null,
        switchTo: true,
      });
      // Import already switched AppState; refresh UI + drop caches for the new profile.
      await adoptActiveProfile();
    } catch (e) {
      importError = String(e);
    } finally {
      importing = false;
    }
  }
</script>

<Section
  title="Transfer"
  description="Back up or restore a complete working profile when moving between machines or preparing a private demo environment."
>
  {#if !active}
    <WorkflowCallout tone="warn" title="No active profile" message="A default profile is normally created automatically. Return to Setup to initialize the workspace." href="/" action="Open Setup" />
  {:else}
    <WorkflowCallout tone="warn" title="Keep profile backups private" message="Transfer archives can contain game-derived reference clips. Use Export when you want a shareable WeiDU voice pack." href="/export" action="Open Export" />
    <Card>
      <div class="panel-head">
        <h3>Export profile</h3>
        <Button onclick={exportActiveProfile} disabled={exporting || transferBusy}>
          {exporting ? "Exporting…" : "Export profile…"}
        </Button>
      </div>
      <p class="hint">
        Writes a ZIP of <strong>{active.name}</strong> including local audio under
        workspaces. Keep backups private — they can contain game-derived reference clips.
      </p>
      {#if exporting || (transferBusy && !importing)}
        <ProgressBar
          value={transferProgress?.done ?? 0}
          max={transferProgress?.total ?? null}
          label="Export profile"
          message={transferProgress?.message ?? "Writing profile backup…"}
        />
      {/if}
      {#if exportError}
        <ErrorNotice message={exportError} />
      {/if}
      {#if exportResult}
        <p class="ok">
          <StatusBadge tone="success">Saved</StatusBadge>
          {exportResult.dest_path}
          ({exportResult.bytes} bytes)
        </p>
      {/if}
    </Card>

    <Card>
      <div class="panel-head">
        <h3>Import profile</h3>
        <Button onclick={importProfileBundle} disabled={importing || transferBusy}>
          {importing ? "Importing…" : "Import profile…"}
        </Button>
      </div>
      <p class="hint">
        Creates a <em>new</em> profile from a backup ZIP and switches to it. Existing
        profiles are left untouched. You may need to confirm the game folder path on Setup
        if this machine uses a different install location.
      </p>
      {#if importing || (transferBusy && !exporting)}
        <ProgressBar
          value={transferProgress?.done ?? 0}
          max={transferProgress?.total ?? null}
          label="Import profile"
          message={transferProgress?.message ?? "Importing profile…"}
        />
      {/if}
      {#if importError}
        <ErrorNotice message={importError} />
      {/if}
      {#if importResult}
        <p class="ok">
          <StatusBadge tone="success">Imported</StatusBadge>
          {importResult.profile.name} (id {importResult.profile.id})
          {#if importResult.switched}— switched{/if}
        </p>
      {/if}
    </Card>
  {/if}
</Section>

<style>
  .panel-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
    margin-bottom: var(--space-2);
  }
  .panel-head h3 {
    margin: 0;
    font-size: 1rem;
  }
  .hint {
    margin: 0 0 var(--space-3);
    color: var(--text-muted);
    font-size: 0.9rem;
    line-height: 1.45;
  }
  .ok {
    margin: var(--space-3) 0 0;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    font-size: 0.9rem;
  }
</style>
