<script lang="ts">
  import { invoke } from "$lib/utils/invoke";
  import { invalidateGeneration } from "$lib/stores/results";
  import type { SynthesisWriteResult } from "$lib/types";
  import {
    findUnknownInlineTags,
    SUPPORTED_INLINE_TAGS,
  } from "$lib/omnivoiceTags";
  import Button from "$lib/components/Button.svelte";
  import ErrorNotice from "$lib/components/ErrorNotice.svelte";

  type Props = {
    lineId: number;
    initialText: string;
    sharedLineCount?: number;
    hasOverride?: boolean;
    /** Optional resolved preview from get_line_synthesis_preview. */
    previewText?: string | null;
    onsaved?: (result: SynthesisWriteResult, text: string) => void | Promise<void>;
    oncleared?: (result: SynthesisWriteResult) => void | Promise<void>;
    oncancel?: () => void;
  };

  let {
    lineId,
    initialText,
    sharedLineCount = 1,
    hasOverride = false,
    previewText = null,
    onsaved,
    oncleared,
    oncancel,
  }: Props = $props();

  let text = $state("");
  let initialized = false;
  let saving = $state(false);
  let clearing = $state(false);
  let error = $state<string | null>(null);
  let textareaEl = $state<HTMLTextAreaElement | null>(null);

  const unknownTags = $derived(findUnknownInlineTags(text));

  $effect.pre(() => {
    if (initialized) return;
    text = initialText;
    initialized = true;
  });

  function insertTag(tag: string) {
    const el = textareaEl;
    if (!el) {
      text = `${text}${tag}`;
      return;
    }
    const start = el.selectionStart ?? text.length;
    const end = el.selectionEnd ?? text.length;
    text = `${text.slice(0, start)}${tag}${text.slice(end)}`;
    queueMicrotask(() => {
      const pos = start + tag.length;
      el.focus();
      el.setSelectionRange(pos, pos);
    });
  }

  async function save() {
    saving = true;
    error = null;
    try {
      const value = text.trim();
      const result = await invoke<SynthesisWriteResult>("set_line_synthesis_override", {
        lineId,
        synthesisText: value,
      });
      invalidateGeneration("synthesis", "critical");
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
      invalidateGeneration("synthesis", "critical");
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
  {#if previewText}
    <p class="preview">Resolved preview: <code>{previewText}</code></p>
  {/if}
  <textarea
    id={`synthesis-${lineId}`}
    rows="3"
    bind:this={textareaEl}
    bind:value={text}
    disabled={saving || clearing}
  ></textarea>
  <div class="tag-row" role="group" aria-label="Insert OmniVoice tag">
    <span class="tag-label">Insert tag</span>
    {#each SUPPORTED_INLINE_TAGS as tag (tag)}
      <button
        type="button"
        class="tag-chip"
        disabled={saving || clearing}
        onclick={() => insertTag(tag)}
      >{tag}</button>
    {/each}
  </div>
  <p class="hint">
    Preserve the subtitle's spoken words. You may remove bad markup, reposition cues, or use
    supported OmniVoice tags. Subtitle and export text stay unchanged.
  </p>
  {#if unknownTags.length > 0}
    <p class="soft-warn">
      Unknown tag(s) {unknownTags.join(", ")} — the base OmniVoice model may speak them as
      ordinary words. Save will still validate on the server.
    </p>
  {/if}
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
  .tag-row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    align-items: center;
  }
  .tag-label {
    font-size: 0.75rem;
    color: var(--text-muted);
    margin-right: var(--space-1);
  }
  .tag-chip {
    font: 0.75rem/1.2 ui-monospace, "Cascadia Code", monospace;
    padding: 0.15rem 0.4rem;
    color: var(--text);
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    cursor: pointer;
  }
  .tag-chip:hover:not(:disabled) {
    border-color: var(--accent);
    color: var(--accent);
  }
  .tag-chip:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .hint,
  .shared,
  .preview,
  .soft-warn {
    margin: 0;
    font-size: 0.8rem;
    color: var(--text-muted);
  }
  .preview code {
    font-family: ui-monospace, "Cascadia Code", monospace;
    color: var(--text);
  }
  .soft-warn {
    color: var(--warn);
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
