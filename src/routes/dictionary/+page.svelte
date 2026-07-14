<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "$lib/utils/invoke";
  import Section from "$lib/components/Section.svelte";
  import Card from "$lib/components/Card.svelte";
  import Button from "$lib/components/Button.svelte";
  import ErrorNotice from "$lib/components/ErrorNotice.svelte";
  import PlaceholderSettings from "$lib/components/PlaceholderSettings.svelte";
  import type {
    DictionaryPreview,
    DictionaryRule,
    DictionaryWriteResult,
  } from "$lib/types";

  type Tab = "placeholders" | "rules";

  let tab = $state<Tab>("placeholders");
  let rules = $state<DictionaryRule[]>([]);
  let loading = $state(true);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let note = $state<string | null>(null);
  let search = $state("");
  let testText = $state("B-b-b-but... I... I... wwaaAAAAHHHH!");
  let preview = $state<DictionaryPreview | null>(null);
  let editingId = $state<number | null>(null);
  let editFind = $state("");
  let editSpeakAs = $state("");

  const enabledCount = $derived(rules.filter((rule) => rule.enabled).length);
  const filteredRules = $derived(
    rules.filter((rule) => {
      const query = search.trim().toLowerCase();
      return (
        !query ||
        rule.find_text.toLowerCase().includes(query) ||
        rule.speak_as.toLowerCase().includes(query)
      );
    }),
  );

  onMount(loadRules);

  async function loadRules() {
    loading = true;
    try {
      rules = await invoke<DictionaryRule[]>("list_dictionary_rules");
    } catch (cause) {
      error = String(cause);
    } finally {
      loading = false;
    }
  }

  async function testRules() {
    error = null;
    try {
      preview = await invoke<DictionaryPreview>("preview_dictionary_text", { text: testText });
    } catch (cause) {
      error = String(cause);
    }
  }

  function startAdd() {
    editingId = 0;
    editFind = "";
    editSpeakAs = "";
  }

  function startEdit(rule: DictionaryRule) {
    editingId = rule.id;
    editFind = rule.find_text;
    editSpeakAs = rule.speak_as;
  }

  async function saveRule() {
    busy = true;
    error = null;
    try {
      const result = await invoke<DictionaryWriteResult>("upsert_dictionary_rule", {
        id: editingId === 0 ? null : editingId,
        findText: editFind,
        speakAs: editSpeakAs,
        matchKind: "whole_word",
        enabled: true,
      });
      editingId = null;
      note = `Saved rule. Reset ${result.reset_generations} generated clip(s).`;
      await loadRules();
      await testRules();
    } catch (cause) {
      error = String(cause);
    } finally {
      busy = false;
    }
  }

  async function toggleRule(rule: DictionaryRule) {
    busy = true;
    error = null;
    try {
      const result = await invoke<DictionaryWriteResult>("set_dictionary_rule_enabled", {
        id: rule.id,
        enabled: !rule.enabled,
      });
      note = `${rule.enabled ? "Disabled" : "Enabled"} rule. Reset ${result.reset_generations} generated clip(s).`;
      await loadRules();
      await testRules();
    } catch (cause) {
      error = String(cause);
    } finally {
      busy = false;
    }
  }

  async function deleteRule(rule: DictionaryRule) {
    if (!confirm(`Delete the rule “${rule.find_text}”?`)) return;
    busy = true;
    error = null;
    try {
      const result = await invoke<DictionaryWriteResult>("delete_dictionary_rule", {
        id: rule.id,
      });
      note = `Deleted rule. Reset ${result.reset_generations} generated clip(s).`;
      await loadRules();
      await testRules();
    } catch (cause) {
      error = String(cause);
    } finally {
      busy = false;
    }
  }

  async function resetDefaults() {
    busy = true;
    error = null;
    try {
      const result = await invoke<DictionaryWriteResult>("reset_dictionary_defaults");
      note = `Restored default rules. Reset ${result.reset_generations} generated clip(s).`;
      await loadRules();
      await testRules();
    } catch (cause) {
      error = String(cause);
    } finally {
      busy = false;
    }
  }
</script>

<Section
  title="Dictionary"
  description="Configure generation-only text substitutions. Placeholders resolve game tokens; global rules make difficult spellings easier for OmniVoice to pronounce."
>
  <div class="tabs" role="tablist" aria-label="Dictionary sections">
    <button
      type="button"
      role="tab"
      aria-selected={tab === "placeholders"}
      class:active={tab === "placeholders"}
      onclick={() => (tab = "placeholders")}
    >
      Placeholders
    </button>
    <button
      type="button"
      role="tab"
      aria-selected={tab === "rules"}
      class:active={tab === "rules"}
      onclick={() => (tab = "rules")}
    >
      Global Rules
    </button>
  </div>

  {#if tab === "placeholders"}
    <PlaceholderSettings />
  {:else}
    <ErrorNotice message={error} />
    {#if note}<p class="note">{note}</p>{/if}

    {#if loading}
      <Card><p class="muted">Loading dictionary rules…</p></Card>
    {:else}
      <div class="summary">
        <Card><span>Rules</span><strong>{rules.length}</strong></Card>
        <Card><span>Enabled</span><strong>{enabledCount}</strong></Card>
      </div>

      <Card>
        <h3>Test rules</h3>
        <label>
          Before
          <input class="test-input" bind:value={testText} />
        </label>
        <Button onclick={testRules}>Test pronunciation</Button>
        {#if preview}
          <div class="preview-grid">
            <div><span>Before</span><p>{preview.before}</p></div>
            <div><span>After (spoken as)</span><p>{preview.after}</p></div>
          </div>
          {#if preview.applied_rules.length}
            <p class="chips">
              {#each preview.applied_rules as applied (applied.id)}
                <code>{applied.find_text} → {applied.speak_as}</code>
              {/each}
            </p>
          {/if}
        {/if}
      </Card>

      <Card>
        <div class="toolbar">
          <input class="search" aria-label="Search rules" placeholder="Search rules…" bind:value={search} />
          <Button variant="ghost" onclick={resetDefaults} disabled={busy}>Reset defaults</Button>
          <Button onclick={startAdd} disabled={busy}>+ Add rule</Button>
        </div>

        {#if editingId !== null}
          <div class="editor">
            <label>Find <input aria-label="Find text" bind:value={editFind} /></label>
            <label>Speak as <input aria-label="Speak as" bind:value={editSpeakAs} /></label>
            <span class="match">whole word</span>
            <Button onclick={saveRule} disabled={busy || !editFind.trim() || !editSpeakAs.trim()}>
              Save
            </Button>
            <Button variant="ghost" onclick={() => (editingId = null)}>Cancel</Button>
          </div>
        {/if}

        <div class="rule-table">
          <div class="rule-head"><span>Find</span><span>Speak as</span><span>Match</span><span>Enabled</span><span>Actions</span></div>
          {#each filteredRules as rule (rule.id)}
            <div class="rule-row">
              <span>{rule.find_text}{#if rule.is_default}<small>default</small>{/if}</span>
              <span>{rule.speak_as}</span>
              <span class="match">whole word</span>
              <button
                class="switch"
                type="button"
                aria-label={`${rule.enabled ? "Disable" : "Enable"} ${rule.find_text}`}
                aria-pressed={rule.enabled}
                onclick={() => toggleRule(rule)}
                disabled={busy}
              >{rule.enabled ? "On" : "Off"}</button>
              <span class="row-actions">
                {#if !rule.is_default}
                  <button type="button" onclick={() => startEdit(rule)}>Edit</button>
                  <button type="button" onclick={() => deleteRule(rule)}>Delete</button>
                {/if}
              </span>
            </div>
          {/each}
        </div>
      </Card>
      <p class="muted">
        Rules affect generated audio only. In-game subtitles and exported dialogue text are unchanged.
      </p>
    {/if}
  {/if}
</Section>

<style>
  .tabs {
    display: flex;
    gap: 0.25rem;
    border-bottom: 1px solid var(--border);
    margin-bottom: 1rem;
  }
  .tabs button {
    background: transparent;
    border: 0;
    border-bottom: 2px solid transparent;
    color: var(--muted);
    padding: 0.65rem 0.85rem;
    cursor: pointer;
  }
  .tabs button.active {
    color: var(--text);
    border-color: var(--accent);
  }
  .summary {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 14rem));
    gap: 1rem;
  }
  .summary :global(.card) {
    display: grid;
    gap: 0.35rem;
  }
  .summary span,
  .preview-grid span,
  .muted,
  small {
    color: var(--muted);
  }
  .summary strong {
    font-size: 1.7rem;
  }
  .note {
    color: var(--accent);
  }
  h3 {
    margin-top: 0;
  }
  label {
    display: grid;
    gap: 0.35rem;
  }
  input {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--text);
    padding: 0.5rem 0.6rem;
  }
  .test-input {
    width: min(100%, 48rem);
    margin-bottom: 0.75rem;
  }
  .preview-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 1rem;
    margin-top: 1rem;
  }
  .preview-grid > div {
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 0.75rem;
  }
  .preview-grid p {
    margin-bottom: 0;
  }
  .chips code {
    display: inline-block;
    margin: 0 0.35rem 0.35rem 0;
    padding: 0.2rem 0.4rem;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
  }
  .toolbar,
  .editor {
    display: flex;
    flex-wrap: wrap;
    align-items: end;
    gap: 0.65rem;
    margin-bottom: 1rem;
  }
  .search {
    margin-right: auto;
  }
  .rule-table {
    overflow-x: auto;
  }
  .rule-head,
  .rule-row {
    min-width: 48rem;
    display: grid;
    grid-template-columns: 1.2fr 1.2fr 0.7fr 0.55fr 0.8fr;
    gap: 0.75rem;
    align-items: center;
    padding: 0.65rem 0.4rem;
    border-top: 1px solid var(--border);
  }
  .rule-head {
    color: var(--muted);
    font-size: 0.85rem;
  }
  .rule-row small {
    display: block;
  }
  .match {
    color: var(--muted);
    font-size: 0.85rem;
  }
  .switch,
  .row-actions button {
    background: transparent;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--text);
    cursor: pointer;
    padding: 0.3rem 0.45rem;
  }
  .switch[aria-pressed="true"] {
    border-color: var(--accent);
    color: var(--accent);
  }
  .row-actions {
    display: flex;
    gap: 0.35rem;
  }
  @media (max-width: 700px) {
    .preview-grid {
      grid-template-columns: 1fr;
    }
  }
</style>
