<script lang="ts">
  import { invoke } from "$lib/utils/invoke";
  import type { SynthesisWriteResult } from "$lib/types";
  import Button from "$lib/components/Button.svelte";
  import ErrorNotice from "$lib/components/ErrorNotice.svelte";

  type Props = {
    lineId: number;
    initialText: string;
    sharedLineCount?: number;
    hasOverride?: boolean;
    onsaved?: (result: SynthesisWriteResult, text: string) => void | Promise<void>;
    oncleared?: (result: SynthesisWriteResult) => void | Promise<void>;
    oncancel?: () => void;
  };

  let {
    lineId,
    initialText,
    sharedLineCount = 1,
    hasOverride = false,
    onsaved,
    oncleared,
    oncancel,
  }: Props = $props();

  let text = $state("");
  let initialized = false;
  let saving = $state(false);
  let clearing = $state(false);
  let error = $state<string | null>(null);

  $effect.pre(() => {
    if (initialized) return;
    text = initialText;
    initialized = true;
  });

  async function save() {
    saving = true;
    error = null;
    try {
      const value = text.trim();
      const result = await invoke<SynthesisWriteResult>("set_line_synthesis_override", {
        lineId,
        synthesisText: value,
      });
      await onsaved?.(result, value);
    } catch (e) {
      error = String(e);
    } finally {
      saving = false;
    }
  }

  async function clear() {
    clearing = true;
    error = null;
    try {
      const result = await invoke<SynthesisWriteResult>("clear_line_synthesis_override", {
        lineId,
      });
      await oncleared?.(result);
    } catch (e) {
      error = String(e);
    } finally {
      clearing = false;
    }
  }
</script>

<div class="editor">
  <label for={`synthesis-${lineId}`}>Generation text</label>
  <textarea
    id={`synthesis-${lineId}`}
    rows="3"
    bind:value={text}
    disabled={saving || clearing}
  ></textarea>
  <p class="hint">
    Preserve the subtitle's spoken words. You may remove bad markup, reposition cues, or use
    supported OmniVoice tags. Subtitle and export text stay unchanged.
  </p>
  {#if sharedLineCount > 1}
    <p class="shared">This decision applies to {sharedLineCount} matching lines.</p>
  {/if}
  <ErrorNotice message={error} />
  <div class="actions">
    <Button onclick={save} disabled={saving || clearing || text.trim().length === 0}>
      {saving ? "Saving…" : "Save override"}
    </Button>
    {#if hasOverride}
      <Button variant="ghost" onclick={clear} disabled={saving || clearing}>
        {clearing ? "Clearing…" : "Clear override"}
      </Button>
    {/if}
    <Button variant="ghost" onclick={() => oncancel?.()} disabled={saving || clearing}>Cancel</Button>
  </div>
</div>

<style>
  .editor {
    display: grid;
    gap: var(--space-2);
    margin-top: var(--space-2);
    padding: var(--space-3);
    border: 1px solid var(--accent);
    border-radius: var(--radius-sm);
    background: var(--panel-2);
  }
  label {
    font-weight: 600;
  }
  textarea {
    width: 100%;
    box-sizing: border-box;
    resize: vertical;
    min-height: 5rem;
    padding: var(--space-2);
    color: var(--text);
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    font: 0.9rem/1.45 ui-monospace, "Cascadia Code", monospace;
  }
  textarea:focus {
    outline: 1px solid var(--accent);
    border-color: var(--accent);
  }
  .hint,
  .shared {
    margin: 0;
    font-size: 0.8rem;
    color: var(--text-muted);
  }
  .shared {
    color: var(--warn);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }
</style>
