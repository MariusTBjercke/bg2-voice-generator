<script lang="ts">
  import { onMount } from "svelte";
  import { get } from "svelte/store";
  import { invoke } from "$lib/utils/invoke";
  import { project } from "$lib/stores/project";
  import { invalidateGeneration, invalidateReview } from "$lib/stores/results";
  import { uiPreferences, updateDictionaryUiPreferences } from "$lib/stores/uiPreferences";
  import Card from "./Card.svelte";
  import Button from "./Button.svelte";
  import ErrorNotice from "./ErrorNotice.svelte";
  import StatusBadge from "./StatusBadge.svelte";
  import {
    KEY_PC_PROFILE,
    PC_PROFILE_OPTIONS,
    PLACEHOLDER_SPECS,
    previewProfileToken,
    previewReplacement,
    type PcProfile,
  } from "$lib/utils/placeholderTokens";
  import type { ReapplyTokenResult } from "$lib/types";

  let loading = $state(true);
  let saving = $state(false);
  let error = $state<string | null>(null);
  let applyResult = $state<ReapplyTokenResult | null>(null);
  let showAdvanced = $state(false);
  let profile = $state<PcProfile>("neutral");
  let savedProfile = $state<PcProfile>("neutral");
  let values = $state<Record<string, string>>({});
  let savedValues = $state<Record<string, string>>({});
  let preferencesHydrated = $state(false);

  const dirty = $derived(
    profile !== savedProfile ||
      PLACEHOLDER_SPECS.some(
        (spec) => (values[spec.key] ?? "").trim() !== (savedValues[spec.key] ?? ""),
      ),
  );

  onMount(async () => {
    showAdvanced = get(uiPreferences).dictionary.placeholderAdvancedOpen;
    preferencesHydrated = true;
    try {
      const current =
        (await invoke<string | null>("get_setting", { key: KEY_PC_PROFILE })) ?? "neutral";
      profile = (current === "male" || current === "female" ? current : "neutral") as PcProfile;
      savedProfile = profile;
      const loaded: Record<string, string> = {};
      for (const spec of PLACEHOLDER_SPECS) {
        loaded[spec.key] =
          (await invoke<string | null>("get_setting", { key: spec.key })) ?? "";
      }
      values = { ...loaded };
      savedValues = { ...loaded };
    } catch (cause) {
      error = String(cause);
    } finally {
      loading = false;
    }
  });

  $effect(() => {
    const open = showAdvanced;
    if (!preferencesHydrated) return;
    updateDictionaryUiPreferences((current) => ({ ...current, placeholderAdvancedOpen: open }));
  });

  function effective(spec: (typeof PLACEHOLDER_SPECS)[number]): string {
    return (values[spec.key] ?? "").trim() || spec.fallback;
  }

  async function saveAndApply() {
    saving = true;
    error = null;
    applyResult = null;
    try {
      if (profile !== savedProfile) {
        await invoke<void>("set_setting", { key: KEY_PC_PROFILE, value: profile });
        savedProfile = profile;
      }
      for (const spec of PLACEHOLDER_SPECS) {
        const value = (values[spec.key] ?? "").trim();
        if (value === (savedValues[spec.key] ?? "")) continue;
        await invoke<void>("set_setting", { key: spec.key, value });
        savedValues[spec.key] = value;
      }
      if ($project.gameDir) {
        applyResult = await invoke<ReapplyTokenResult>("reapply_token_standins", {
          gameDir: $project.gameDir,
        });
      }
      invalidateGeneration("critical", "synthesis");
      invalidateReview();
    } catch (cause) {
      error = String(cause);
    } finally {
      saving = false;
    }
  }
</script>

<ErrorNotice message={error} />
{#if loading}
  <Card><p class="muted">Loading placeholder settings…</p></Card>
{:else}
  <Card>
    <h3>PC profile</h3>
    <p class="hint">
      Sets defaults for gendered protagonist tokens such as <code>&lt;PRO_HISHER&gt;</code>.
    </p>
    <div class="profile-row">
      {#each PC_PROFILE_OPTIONS as option (option.value)}
        <label>
          <input type="radio" name="pc-profile" value={option.value} bind:group={profile} />
          {option.label}
        </label>
      {/each}
    </div>
    <p class="preview">
      Example: We leave <strong>{previewProfileToken(profile, "PRO_HISHER")}</strong> chosen
      path, my <strong>{previewProfileToken(profile, "PRO_LADYLORD")}</strong>.
    </p>
  </Card>

  <Card>
    <button class="advanced" type="button" onclick={() => (showAdvanced = !showAdvanced)}>
      {showAdvanced ? "Hide" : "Show"} advanced overrides
    </button>
    {#if showAdvanced}
      <div class="rule-list">
        {#each PLACEHOLDER_SPECS as spec (spec.key)}
          <label class="placeholder-row">
            <span>
              <code>{spec.token}</code>
              <small>{spec.description}</small>
            </span>
            <input
              bind:value={values[spec.key]}
              placeholder={spec.fallback ? `${spec.fallback} (default)` : spec.suggestion}
            />
            <span class="preview">
              {previewReplacement(spec.example, spec.exampleToken, effective(spec)) ||
                "(nothing left to voice)"}
            </span>
          </label>
        {/each}
      </div>
    {/if}
  </Card>

  <Card>
    <div class="actions">
      <Button onclick={saveAndApply} disabled={!dirty || saving}>
        {saving ? "Saving…" : "Save + Apply"}
      </Button>
      {#if !$project.gameDir}
        <StatusBadge tone="warn">No game folder — settings saved only</StatusBadge>
      {/if}
    </div>
    {#if applyResult}
      <p>
        Updated {applyResult.updated} lines.
        {#if applyResult.reset_generations}
          Marked {applyResult.reset_generations} clip(s) as text changed (still playable).
        {/if}
      </p>
    {/if}
  </Card>
{/if}

<style>
  .muted,
  .hint,
  small {
    color: var(--muted);
  }
  h3,
  p {
    margin-top: 0;
  }
  .profile-row,
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 1rem;
    align-items: center;
  }
  .profile-row label {
    cursor: pointer;
  }
  .preview {
    font-size: 0.9rem;
  }
  .advanced {
    color: var(--accent);
    background: transparent;
    border: 0;
    padding: 0;
    cursor: pointer;
    text-decoration: underline;
  }
  .rule-list {
    display: grid;
    gap: 0.75rem;
    margin-top: 1rem;
  }
  .placeholder-row {
    display: grid;
    grid-template-columns: minmax(12rem, 1fr) minmax(10rem, 0.7fr) minmax(14rem, 1fr);
    gap: 0.75rem;
    align-items: center;
    border-top: 1px solid var(--border);
    padding-top: 0.75rem;
  }
  .placeholder-row small {
    display: block;
    margin-top: 0.2rem;
  }
  input:not([type="radio"]) {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--text);
    padding: 0.45rem 0.55rem;
  }
  @media (max-width: 760px) {
    .placeholder-row {
      grid-template-columns: 1fr;
    }
  }
</style>
