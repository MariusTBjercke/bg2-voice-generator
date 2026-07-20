<script lang="ts">
  import { onMount } from "svelte";
  import { get } from "svelte/store";
  import { invoke } from "$lib/utils/invoke";
  import { invalidateGeneration, invalidateReview } from "$lib/stores/results";
  import {
    uiPreferences,
    updateDictionaryUiPreferences,
    type DictionaryTab,
  } from "$lib/stores/uiPreferences";
  import Section from "$lib/components/Section.svelte";
  import Card from "$lib/components/Card.svelte";
  import Button from "$lib/components/Button.svelte";
  import ErrorNotice from "$lib/components/ErrorNotice.svelte";
  import WorkflowCallout from "$lib/components/WorkflowCallout.svelte";
  import PlaceholderSettings from "$lib/components/PlaceholderSettings.svelte";
  import type {
    DictionaryPreview,
    DictionaryRule,
    DictionaryWriteResult,
    TagMatchKind,
    TagRule,
    TagRulesPreview,
    TagRuleWriteResult,
  } from "$lib/types";
  import { localeText, resolveSort, sortItems, sortOptionsFromSpecs, type SortSpec } from "$lib/filters";

  let tab = $state<DictionaryTab>("placeholders");
  let rules = $state<DictionaryRule[]>([]);
  let tagRules = $state<TagRule[]>([]);
  let tagCatalog = $state<string[]>([]);
  let loading = $state(true);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let note = $state<string | null>(null);
  let search = $state("");
  let tagSearch = $state("");
  let pronunciationSort = $state("find_asc");
  let tagSort = $state("find_asc");
  let testText = $state("B-b-b-but... I... I... wwaaAAAAHHHH!");
  let tagTestText = $state("Bah! *sigh* This is annoying.");
  let preview = $state<DictionaryPreview | null>(null);
  let tagPreview = $state<TagRulesPreview | null>(null);
  let editingId = $state<number | null>(null);
  let editFind = $state("");
  let editSpeakAs = $state("");
  let tagEditingId = $state<number | null>(null);
  let tagEditFind = $state("");
  let tagEditTag = $state("[dissatisfaction-hnn]");
  let tagEditMatch = $state<TagMatchKind>("whole_word");
  let preferencesHydrated = $state(false);

  const pronunciationSortSpecs: SortSpec<DictionaryRule>[] = [
    {
      key: "find_asc",
      label: "Find text A–Z",
      compare: (a, b) => localeText(a.find_text, b.find_text),
    },
    {
      key: "replacement_asc",
      label: "Speak-as A–Z",
      compare: (a, b) => localeText(a.speak_as, b.speak_as) || localeText(a.find_text, b.find_text),
    },
    {
      key: "enabled_first",
      label: "Enabled first",
      compare: (a, b) => Number(b.enabled) - Number(a.enabled) || localeText(a.find_text, b.find_text),
    },
  ];
  const tagSortSpecs: SortSpec<TagRule>[] = [
    {
      key: "find_asc",
      label: "Find text A–Z",
      compare: (a, b) => localeText(a.find_text, b.find_text),
    },
    {
      key: "replacement_asc",
      label: "Tag A–Z",
      compare: (a, b) => localeText(a.tag, b.tag) || localeText(a.find_text, b.find_text),
    },
    {
      key: "enabled_first",
      label: "Enabled first",
      compare: (a, b) => Number(b.enabled) - Number(a.enabled) || localeText(a.find_text, b.find_text),
    },
  ];
  const pronunciationSortOptions = $derived(sortOptionsFromSpecs(pronunciationSortSpecs));
  const tagSortOptions = $derived(sortOptionsFromSpecs(tagSortSpecs));

  const enabledCount = $derived(rules.filter((rule) => rule.enabled).length);
  const tagEnabledCount = $derived(tagRules.filter((rule) => rule.enabled).length);
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
  const sortedRules = $derived(
    sortItems(filteredRules, resolveSort(pronunciationSortSpecs, pronunciationSort)),
  );
  const filteredTagRules = $derived(
    tagRules.filter((rule) => {
      const query = tagSearch.trim().toLowerCase();
      return (
        !query ||
        rule.find_text.toLowerCase().includes(query) ||
        rule.tag.toLowerCase().includes(query) ||
        rule.match_kind.includes(query)
      );
    }),
  );
  const sortedTagRules = $derived(
    sortItems(filteredTagRules, resolveSort(tagSortSpecs, tagSort)),
  );

  onMount(async () => {
    const preferences = get(uiPreferences).dictionary;
    tab = preferences.tab;
    search = preferences.pronunciationSearch;
    tagSearch = preferences.tagSearch;
    pronunciationSort = preferences.pronunciationSort || "find_asc";
    tagSort = preferences.tagSort || "find_asc";
    testText = preferences.pronunciationTestText;
    tagTestText = preferences.tagTestText;
    preferencesHydrated = true;
    await Promise.all([loadRules(), loadTagRules()]);
  });

  $effect(() => {
    const snapshot = {
      tab,
      search,
      tagSearch,
      pronunciationSort,
      tagSort,
      testText,
      tagTestText,
    };
    if (!preferencesHydrated) return;
    updateDictionaryUiPreferences((current) => ({
      ...current,
      tab: snapshot.tab,
      pronunciationSearch: snapshot.search,
      tagSearch: snapshot.tagSearch,
      pronunciationSort: snapshot.pronunciationSort,
      tagSort: snapshot.tagSort,
      pronunciationTestText: snapshot.testText,
      tagTestText: snapshot.tagTestText,
    }));
  });

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

  async function loadTagRules() {
    try {
      const [rows, catalog] = await Promise.all([
        invoke<TagRule[]>("list_tag_rules"),
        invoke<string[]>("list_supported_inline_tags"),
      ]);
      tagRules = rows;
      tagCatalog = catalog;
      if (!tagCatalog.includes(tagEditTag) && tagCatalog.length) {
        tagEditTag = tagCatalog[0];
      }
    } catch (cause) {
      error = String(cause);
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

  async function testTagRules() {
    error = null;
    try {
      tagPreview = await invoke<TagRulesPreview>("preview_tag_rules_text", { text: tagTestText });
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

  function startAddTag() {
    tagEditingId = 0;
    tagEditFind = "";
    tagEditTag = tagCatalog[0] ?? "[dissatisfaction-hnn]";
    tagEditMatch = "whole_word";
  }

  function startEditTag(rule: TagRule) {
    tagEditingId = rule.id;
    tagEditFind = rule.find_text;
    tagEditTag = rule.tag;
    tagEditMatch = rule.match_kind;
  }

  function matchLabel(kind: TagMatchKind): string {
    return kind === "stage_cue" ? "stage cue" : "spoken word";
  }

  function markTextMappingChanged() {
    invalidateGeneration("critical", "synthesis");
    invalidateReview();
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
      note = `Saved pronunciation rule. Marked ${result.reset_generations} clip(s) as text changed (still playable).`;
      markTextMappingChanged();
      await loadRules();
      await testRules();
    } catch (cause) {
      error = String(cause);
    } finally {
      busy = false;
    }
  }

  async function saveTagRule() {
    busy = true;
    error = null;
    try {
      const result = await invoke<TagRuleWriteResult>("upsert_tag_rule", {
        id: tagEditingId === 0 ? null : tagEditingId,
        findText: tagEditFind,
        tag: tagEditTag,
        matchKind: tagEditMatch,
        enabled: true,
      });
      tagEditingId = null;
      note = `Saved tag rule. Marked ${result.reset_generations} clip(s) as text changed (still playable).`;
      markTextMappingChanged();
      await loadTagRules();
      await testTagRules();
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
      note = `${rule.enabled ? "Disabled" : "Enabled"} rule. Marked ${result.reset_generations} clip(s) as text changed (still playable).`;
      markTextMappingChanged();
      await loadRules();
      await testRules();
    } catch (cause) {
      error = String(cause);
    } finally {
      busy = false;
    }
  }

  async function toggleTagRule(rule: TagRule) {
    busy = true;
    error = null;
    try {
      const result = await invoke<TagRuleWriteResult>("set_tag_rule_enabled", {
        id: rule.id,
        enabled: !rule.enabled,
      });
      note = `${rule.enabled ? "Disabled" : "Enabled"} tag rule. Marked ${result.reset_generations} clip(s) as text changed (still playable).`;
      markTextMappingChanged();
      await loadTagRules();
      await testTagRules();
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
      note = `Deleted rule. Marked ${result.reset_generations} clip(s) as text changed (still playable).`;
      markTextMappingChanged();
      await loadRules();
      await testRules();
    } catch (cause) {
      error = String(cause);
    } finally {
      busy = false;
    }
  }

  async function deleteTagRule(rule: TagRule) {
    if (!confirm(`Delete the tag rule “${rule.find_text}”?`)) return;
    busy = true;
    error = null;
    try {
      const result = await invoke<TagRuleWriteResult>("delete_tag_rule", { id: rule.id });
      note = `Deleted tag rule. Marked ${result.reset_generations} clip(s) as text changed (still playable).`;
      markTextMappingChanged();
      await loadTagRules();
      await testTagRules();
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
      note = `Restored pronunciation defaults. Marked ${result.reset_generations} clip(s) as text changed (still playable).`;
      markTextMappingChanged();
      await loadRules();
      await testRules();
    } catch (cause) {
      error = String(cause);
    } finally {
      busy = false;
    }
  }

  async function resetTagDefaults() {
    busy = true;
    error = null;
    try {
      const result = await invoke<TagRuleWriteResult>("reset_tag_rule_defaults");
      note = `Restored tag-rule defaults. Marked ${result.reset_generations} clip(s) as text changed (still playable).`;
      markTextMappingChanged();
      await loadTagRules();
      await testTagRules();
    } catch (cause) {
      error = String(cause);
    } finally {
      busy = false;
    }
  }
</script>

<Section
  title="Dictionary"
  description="Choose how dynamic game tokens and difficult words should sound without changing the original dialogue text."
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
      Pronunciation
    </button>
    <button
      type="button"
      role="tab"
      aria-selected={tab === "tags"}
      class:active={tab === "tags"}
      onclick={() => (tab = "tags")}
    >
      Tag rules
    </button>
  </div>

  {#if tab === "placeholders"}
    <PlaceholderSettings />
  {:else if tab === "rules"}
    <ErrorNotice message={error} />
    {#if note}<p class="note">{note}</p>{/if}

    {#if loading}
      <Card><p class="muted">Loading pronunciation rules…</p></Card>
    {:else}
      <p class="blurb">
        Pronunciation rules change how words are spoken. They cannot insert OmniVoice tags — use Tag rules for that.
      </p>
      <div class="summary">
        <Card><span>Rules</span><strong>{rules.length}</strong></Card>
        <Card><span>Enabled</span><strong>{enabledCount}</strong></Card>
      </div>

      <Card>
        <h3>Test pronunciation</h3>
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
          <label class="sort-field">
            <span class="field-label">Sort</span>
            <select aria-label="Sort pronunciation rules" bind:value={pronunciationSort}>
              {#each pronunciationSortOptions as option (option.key)}
                <option value={option.key}>{option.label}</option>
              {/each}
            </select>
          </label>
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
          {#each sortedRules as rule (rule.id)}
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
  {:else}
    <ErrorNotice message={error} />
    {#if note}<p class="note">{note}</p>{/if}

    <p class="blurb">
      Tag rules convert stage cues (<code>*sigh*</code>) and optional spoken words (<code>Bah</code>) into OmniVoice non-verbal tags. Defaults mirror the built-in mapper; add spoken-word rules for cases Review would otherwise handle one-by-one.
    </p>
    <div class="summary">
      <Card><span>Rules</span><strong>{tagRules.length}</strong></Card>
      <Card><span>Enabled</span><strong>{tagEnabledCount}</strong></Card>
    </div>

    <Card>
      <h3>Test tag rules</h3>
      <label>
        Before
        <input class="test-input" bind:value={tagTestText} />
      </label>
      <Button onclick={testTagRules}>Test tags</Button>
      {#if tagPreview}
        <div class="preview-grid">
          <div><span>Before</span><p>{tagPreview.before}</p></div>
          <div><span>After (with tags)</span><p>{tagPreview.after}</p></div>
        </div>
        {#if tagPreview.applied_rules.length}
          <p class="chips">
            {#each tagPreview.applied_rules as applied (applied.id)}
              <code>{applied.find_text} → {applied.tag}</code>
            {/each}
          </p>
        {/if}
      {/if}
    </Card>

    <Card>
      <div class="toolbar">
        <input class="search" aria-label="Search tag rules" placeholder="Search tag rules…" bind:value={tagSearch} />
        <label class="sort-field">
          <span class="field-label">Sort</span>
          <select aria-label="Sort tag rules" bind:value={tagSort}>
            {#each tagSortOptions as option (option.key)}
              <option value={option.key}>{option.label}</option>
            {/each}
          </select>
        </label>
        <Button variant="ghost" onclick={resetTagDefaults} disabled={busy}>Reset defaults</Button>
        <Button onclick={startAddTag} disabled={busy}>+ Add tag rule</Button>
      </div>

      {#if tagEditingId !== null}
        <div class="editor">
          <label>Find <input aria-label="Tag find text" bind:value={tagEditFind} /></label>
          <label>
            Match
            <select aria-label="Tag match kind" bind:value={tagEditMatch}>
              <option value="whole_word">Spoken word</option>
              <option value="stage_cue">Stage cue (*...*)</option>
            </select>
          </label>
          <label>
            Tag
            <select aria-label="OmniVoice tag" bind:value={tagEditTag}>
              {#each tagCatalog as tag (tag)}
                <option value={tag}>{tag}</option>
              {/each}
            </select>
          </label>
          <Button onclick={saveTagRule} disabled={busy || !tagEditFind.trim() || !tagEditTag}>
            Save
          </Button>
          <Button variant="ghost" onclick={() => (tagEditingId = null)}>Cancel</Button>
        </div>
      {/if}

      <div class="rule-table tag-table">
        <div class="rule-head"><span>Find</span><span>Tag</span><span>Match</span><span>Enabled</span><span>Actions</span></div>
        {#each sortedTagRules as rule (rule.id)}
          <div class="rule-row">
            <span>{rule.find_text}{#if rule.is_default}<small>default</small>{/if}</span>
            <span class="mono">{rule.tag}</span>
            <span class="match">{matchLabel(rule.match_kind)}</span>
            <button
              class="switch"
              type="button"
              aria-label={`${rule.enabled ? "Disable" : "Enable"} ${rule.find_text}`}
              aria-pressed={rule.enabled}
              onclick={() => toggleTagRule(rule)}
              disabled={busy}
            >{rule.enabled ? "On" : "Off"}</button>
            <span class="row-actions">
              {#if !rule.is_default}
                <button type="button" onclick={() => startEditTag(rule)}>Edit</button>
                <button type="button" onclick={() => deleteTagRule(rule)}>Delete</button>
              {/if}
            </span>
          </div>
        {/each}
      </div>
    </Card>
    <p class="muted">
      Tag rules affect generated audio only. Overrides still win for one-off lines.
    </p>
  {/if}
  <WorkflowCallout title="Dictionary changes are optional" message="The included defaults cover common BG2 tokens and cues. When they look right, scan the installation to attribute each line." href="/attribution" action="Continue to Attribution" />
</Section>

<style>
  .tabs {
    display: flex;
    gap: 0.25rem;
    width: fit-content;
    max-width: 100%;
    padding: var(--space-1);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--panel-deep);
    overflow-x: auto;
  }
  .tabs button {
    background: transparent;
    border: 0;
    border-radius: var(--radius-sm);
    color: var(--text-muted);
    padding: 0.65rem 0.85rem;
    cursor: pointer;
  }
  .tabs button.active {
    color: var(--accent-ink);
    border-color: var(--accent);
    background: var(--accent);
  }
  .blurb {
    color: var(--text-muted);
    margin-bottom: 1rem;
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
    color: var(--text-muted);
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
  input,
  select {
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
  .chips code,
  .mono {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
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
  .sort-field {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    min-width: 9rem;
  }
  .sort-field .field-label {
    font-size: 0.8rem;
    color: var(--text-muted);
  }
  .sort-field select {
    font: inherit;
    background: var(--panel-2);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 0.35rem 0.5rem;
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
