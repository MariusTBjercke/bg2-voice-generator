<script lang="ts">
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { invoke } from "$lib/utils/invoke";
  import { project } from "$lib/stores/project";
  import { ensureGameDir } from "$lib/stores/results";
  import Section from "$lib/components/Section.svelte";
  import Card from "$lib/components/Card.svelte";
  import Button from "$lib/components/Button.svelte";
  import StatusBadge from "$lib/components/StatusBadge.svelte";
  import ErrorNotice from "$lib/components/ErrorNotice.svelte";
  import ProgressBar from "$lib/components/ProgressBar.svelte";
  import { progress } from "$lib/stores/progress";
  import type { TransferExportResult, TransferImportResult } from "$lib/types";

  // Transfer (item-12 backend): move a project's STATE between machines. The
  // bundle is a JSON-only ZIP - NO game-derived audio ever travels (copyright),
  // so an imported project always needs a LOCAL re-scan -> re-harvest -> generate
  // to rebuild its audio. Export needs a scanned project for the active game
  // folder; import is create-only and refuses if a project already exists for
  // this install (surfaced as a plain-string error). All I/O via invoke (ADR 0003).

  const dir = $derived($project.gameDir);
  // Live backend progress for the transfer op (coarse/indeterminate). Both panels
  // share the "transfer" op id, so the local exporting/importing flag decides which
  // panel shows the bar.
  const transferProgress = $derived($progress.transfer ?? null);
  // A transfer surviving in the progress store (this component remounts on tab
  // switch) still locks both panels so a second transfer can't start mid-run.
  const transferBusy = $derived(transferProgress !== null);

  let exporting = $state(false);
  let exportResult = $state<TransferExportResult | null>(null);
  let exportError = $state<string | null>(null);

  let importing = $state(false);
  let importResult = $state<TransferImportResult | null>(null);
  let importError = $state<string | null>(null);

  async function exportProject() {
    if (!dir || exporting) return;
    exportError = null;
    let destPath: string | null;
    try {
      destPath = await save({
        title: "Save transfer bundle",
        defaultPath: "bg2vg-transfer.zip",
        filters: [{ name: "Transfer bundle", extensions: ["zip"] }],
      });
    } catch (e) {
      exportError = String(e);
      return;
    }
    if (!destPath) return; // user cancelled
    exporting = true;
    exportResult = null;
    try {
      exportResult = await invoke<TransferExportResult>("export_project", {
        gameDir: dir,
        destPath,
      });
    } catch (e) {
      exportError = String(e);
    } finally {
      exporting = false;
    }
  }

  async function importProject() {
    if (!dir || importing) return;
    importError = null;
    let selected: string | string[] | null;
    try {
      selected = await open({
        title: "Choose a transfer bundle",
        multiple: false,
        filters: [{ name: "Transfer bundle", extensions: ["zip"] }],
      });
    } catch (e) {
      importError = String(e);
      return;
    }
    const bundlePath = Array.isArray(selected) ? (selected[0] ?? null) : selected;
    if (!bundlePath) return; // user cancelled
    importing = true;
    importResult = null;
    try {
      importResult = await invoke<TransferImportResult>("import_project", {
        bundlePath,
        gameDir: dir,
      });
      ensureGameDir(null);
      ensureGameDir(dir);
    } catch (e) {
      importError = String(e);
    } finally {
      importing = false;
    }
  }
</script>

<Section
  title="Transfer"
  description="Move a project's state between machines with an audio-free bundle. Only your scan/harvest decisions and generation plan travel - audio is rebuilt locally after import."
>
  {#if !dir}
    <Card>
      <p class="hint">Choose your game folder on the <a href="/">Setup</a> screen first.</p>
    </Card>
  {:else}
    <Card>
      <div class="panel-head">
        <h3>Export project</h3>
        <Button onclick={exportProject} disabled={exporting || transferBusy}>
          {exporting ? "Exporting…" : "Export project…"}
        </Button>
      </div>
      <p class="hint">
        Writes a self-contained ZIP (config + attribution/harvest decisions + generation
        plan) for the current install. No game audio is included.
      </p>
      {#if transferProgress && exporting}
        <div class="progress-row">
          <ProgressBar
            label="Exporting bundle"
            value={transferProgress.done}
            max={transferProgress.total}
            message={transferProgress.message}
          />
        </div>
      {/if}
      <ErrorNotice message={exportError} />

      {#if exportResult}
        {@const r = exportResult}
        <div class="result">
          <div class="badges">
            <StatusBadge tone="success">Bundle written</StatusBadge>
            <StatusBadge tone="info">{r.speakers} speakers</StatusBadge>
            <StatusBadge tone="info">{r.lines} lines</StatusBadge>
            <StatusBadge tone="info">{r.decisions} decisions</StatusBadge>
          </div>
          <p class="path mono" title={r.path}>{r.path}</p>
        </div>
      {/if}
    </Card>

    <Card>
      <div class="panel-head">
        <h3>Import project</h3>
        <Button variant="ghost" onclick={importProject} disabled={importing || transferBusy}>
          {importing ? "Importing…" : "Choose bundle…"}
        </Button>
      </div>
      <p class="hint">
        Reconstructs a bundle as a fresh project bound to the game folder from Setup.
        Import is create-only: it refuses if this install already has a project.
      </p>
      {#if transferProgress && importing}
        <div class="progress-row">
          <ProgressBar
            label="Importing bundle"
            value={transferProgress.done}
            max={transferProgress.total}
            message={transferProgress.message}
          />
        </div>
      {/if}
      <ErrorNotice message={importError} />

      {#if importResult}
        {@const r = importResult}
        <div class="result">
          <div class="badges">
            <StatusBadge tone="success">Imported (project #{r.project_id})</StatusBadge>
            <StatusBadge tone="info">{r.speakers} speakers</StatusBadge>
            <StatusBadge tone="info">{r.lines} lines</StatusBadge>
            <StatusBadge tone="info">{r.decisions} decisions</StatusBadge>
            <StatusBadge tone="info">{r.clones} clones</StatusBadge>
          </div>
          {#if r.needs_local_rescan}
            <div class="rescan" role="status">
              <strong>Local rebuild required.</strong> No audio was transferred (by design).
              On this machine, run <a href="/attribution">Attribution</a> →
              <a href="/harvest">Harvest</a> → <a href="/generation">Generation</a> to
              rebuild the voices before <a href="/export">exporting</a> a pack.
            </div>
          {/if}
        </div>
      {/if}
    </Card>
  {/if}
</Section>

<style>
  h3 {
    margin: 0;
    font-size: 1rem;
  }
  .panel-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  .hint {
    margin: var(--space-3) 0 0;
    color: var(--text-muted);
  }
  .progress-row {
    margin-top: var(--space-4);
  }
  .result {
    margin-top: var(--space-4);
    border-top: 1px solid var(--border);
    padding-top: var(--space-3);
  }
  .badges {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .path {
    margin: var(--space-3) 0 0;
    font-size: 0.85rem;
    color: var(--text);
    overflow-wrap: anywhere;
  }
  .rescan {
    background: var(--panel-2);
    border: 1px solid var(--warn);
    border-radius: var(--radius-sm);
    padding: var(--space-3) var(--space-4);
    color: var(--text);
    margin-top: var(--space-3);
  }
  .rescan strong {
    color: var(--warn);
  }
  .mono {
    font-family: ui-monospace, "Cascadia Code", monospace;
  }
</style>
