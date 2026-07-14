<script lang="ts">
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { invoke } from "$lib/utils/invoke";
  import { project } from "$lib/stores/project";
  import { results, ensureGameDir, setExportResult } from "$lib/stores/results";
  import Section from "$lib/components/Section.svelte";
  import Card from "$lib/components/Card.svelte";
  import Button from "$lib/components/Button.svelte";
  import StatusBadge from "$lib/components/StatusBadge.svelte";
  import ErrorNotice from "$lib/components/ErrorNotice.svelte";
  import ProgressBar from "$lib/components/ProgressBar.svelte";
  import { progress } from "$lib/stores/progress";
  import type { CompletedGeneration, ExportResult } from "$lib/types";

  // Export: build a native WeiDU voice pack from the project's completed
  // generations (build_export). No engine is needed here - it consumes the WAVs
  // item-06 wrote. build_export gathers every `done` generation, lets the pure
  // planner decide which lines are safe to patch (deferring tokens/transitions/
  // script/shared-diff/already-voiced/missing-clip with reasons), writes the pack
  // folder, and bundles it (+ vendored WeiDU) into a self-contained ZIP. pack_zip
  // is null only when no WeiDU was vendored (dev run) - the folder is still valid.
  // If every candidate is deferred (or the install was never scanned) the backend
  // returns a plain-string error, which we surface as-is.

  const dir = $derived($project.gameDir);
  const cached = $derived($results.export.result);
  // Live backend progress for the pack build (coarse/indeterminate).
  const exportProgress = $derived($progress.export ?? null);

  let packName = $state("");
  let building = $state(false);
  let error = $state<string | null>(null);
  let revealError = $state<string | null>(null);
  let voiceChangedCount = $state(0);

  // The local `building` flag dies with the component on tab switch while the build
  // keeps running in the backend; OR in the surviving progress entry so a return to
  // this tab still shows Building… and keeps the button locked.
  const buildBusy = $derived(building || exportProgress !== null);

  // Keep the cache tagged to the active install so a different game folder never
  // shows a stale pack result.
  $effect(() => {
    ensureGameDir(dir);
    void loadVoiceChangedCount(dir);
  });

  async function loadVoiceChangedCount(gameDir: string | null) {
    if (!gameDir) {
      voiceChangedCount = 0;
      return;
    }
    try {
      const completed = await invoke<CompletedGeneration[]>("list_completed_generations", {
        gameDir,
      });
      if (dir === gameDir) voiceChangedCount = completed.filter((clip) => clip.voice_changed).length;
    } catch {
      if (dir === gameDir) voiceChangedCount = 0;
    }
  }

  async function buildExport() {
    if (!dir) return;
    building = true;
    error = null;
    revealError = null;
    const trimmed = packName.trim();
    try {
      const result = await invoke<ExportResult>("build_export", {
        gameDir: dir,
        locale: $project.locale ?? undefined,
        packName: trimmed === "" ? undefined : trimmed,
      });
      setExportResult(result);
    } catch (e) {
      error = String(e);
    } finally {
      building = false;
    }
  }

  // Reveal the pack folder / ZIP in the OS file explorer. revealItemInDir is
  // covered by the opener:default capability (allow-reveal-item-in-dir), so no
  // extra permission is needed; failures surface inline instead of throwing.
  async function reveal(path: string) {
    revealError = null;
    try {
      await revealItemInDir(path);
    } catch (e) {
      revealError = String(e);
    }
  }
</script>

<Section
  title="Export"
  description="Bundle your completed generations into a native WeiDU voice pack (a self-contained ZIP you can install over the game). No engine needed."
>
  {#if !dir}
    <Card>
      <p class="hint">Choose your game folder on the <a href="/">Setup</a> screen first.</p>
    </Card>
  {:else}
    <Card>
      <div class="build-row">
        <label class="field">
          <span class="label">Pack name</span>
          <input
            type="text"
            bind:value={packName}
            placeholder="BG2VG_Voices"
            disabled={buildBusy}
            spellcheck="false"
          />
        </label>
        <Button onclick={buildExport} disabled={buildBusy}>
          {buildBusy ? "Building…" : "Build export"}
        </Button>
      </div>
      {#if exportProgress}
        <div class="progress-row">
          <ProgressBar
            label="Building export"
            value={exportProgress.done}
            max={exportProgress.total}
            message={exportProgress.message}
          />
        </div>
      {/if}
      <p class="hint">
        Exports every line whose audio is generated; lines that can't be safely patched
        (script tokens, transitions, already-voiced, shared-text conflicts, missing clips)
        are deferred and reported below. Spoken placeholder stand-ins are used only in the
        generated audio: the installed pack preserves the original dialogue text and tokens.
        Running with nothing to export is not harmful.
      </p>
      {#if voiceChangedCount > 0}
        <div class="warn-box" role="status">
          {voiceChangedCount} generated clip{voiceChangedCount === 1 ? " uses" : "s use"} an earlier
          speaker binding. Otherwise eligible clips will still be included in this export;
          regenerate or remove them on the <a href="/generation">Generation</a> screen if needed.
        </div>
      {/if}
      <ErrorNotice message={error} />
    </Card>

    {#if cached}
      {@const r = cached}
      <Card>
        <div class="result-head">
          <h3>Pack built</h3>
          <StatusBadge tone="success">{r.patched_lines} patched</StatusBadge>
          <StatusBadge tone={r.deferred_lines > 0 ? "warn" : "neutral"}>
            {r.deferred_lines} deferred
          </StatusBadge>
          {#if r.voice_changed_lines > 0}
            <StatusBadge tone="warn">{r.voice_changed_lines} voice changed</StatusBadge>
          {/if}
        </div>

        <dl class="meta">
          <div><dt>Edition</dt><dd>{r.edition}</dd></div>
          <div><dt>Fingerprint</dt><dd class="mono">{r.mod_state_hash}</dd></div>
          <div><dt>Export id</dt><dd class="mono">#{r.export_id}</dd></div>
        </dl>

        <div class="artifact">
          <div class="artifact-main">
            <span class="label">Pack folder</span>
            <p class="path mono" title={r.pack_dir}>{r.pack_dir}</p>
          </div>
          <Button variant="ghost" onclick={() => reveal(r.pack_dir)}>Open folder</Button>
        </div>

        {#if r.pack_zip}
          <div class="artifact">
            <div class="artifact-main">
              <span class="label">Self-contained ZIP</span>
              <p class="path mono" title={r.pack_zip}>{r.pack_zip}</p>
            </div>
            <Button variant="ghost" onclick={() => reveal(r.pack_zip!)}>Reveal ZIP</Button>
          </div>
        {:else}
          <div class="warn-box" role="status">
            No self-contained ZIP was produced because no WeiDU installer is vendored. The
            pack folder above is still a valid WeiDU mod; run <span class="mono">fetch-tools.ps1</span>
            to bundle WeiDU and get a one-file <span class="mono">setup-*.exe</span> ZIP.
          </div>
        {/if}

        <ErrorNotice message={revealError} />
      </Card>
    {/if}
  {/if}
</Section>

<style>
  h3 {
    margin: 0;
    font-size: 1rem;
  }
  .hint {
    margin: var(--space-3) 0 0;
    color: var(--text-muted);
  }
  .build-row {
    display: flex;
    align-items: flex-end;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  .progress-row {
    margin-top: var(--space-4);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    flex: 1 1 16rem;
  }
  .label {
    font-size: 0.8rem;
    color: var(--text-muted);
  }
  .field input {
    font: inherit;
    background: var(--panel-2);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: var(--space-2) var(--space-3);
  }
  .field input:focus {
    outline: none;
    border-color: var(--accent);
  }
  .result-head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  .result-head h3 {
    margin-right: auto;
  }
  .meta {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(14rem, 1fr));
    gap: var(--space-3);
    margin: var(--space-4) 0;
  }
  .meta dt {
    font-size: 0.8rem;
    color: var(--text-muted);
  }
  .meta dd {
    margin: 0;
    word-break: break-all;
  }
  .artifact {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: var(--space-4);
    border-top: 1px solid var(--border);
    padding-top: var(--space-3);
    margin-top: var(--space-3);
  }
  .artifact-main {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }
  .path {
    margin: 0;
    font-size: 0.85rem;
    color: var(--text);
    overflow-wrap: anywhere;
  }
  .warn-box {
    background: var(--panel-2);
    border: 1px solid var(--warn);
    border-radius: var(--radius-sm);
    padding: var(--space-3) var(--space-4);
    color: var(--warn);
    margin-top: var(--space-3);
  }
  .mono {
    font-family: ui-monospace, "Cascadia Code", monospace;
  }
</style>
