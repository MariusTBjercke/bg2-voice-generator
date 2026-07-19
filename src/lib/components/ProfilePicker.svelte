<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "$lib/components/Icon.svelte";
  import type { ProfileInfo } from "$lib/types";

  type Props = {
    profiles: ProfileInfo[];
    activeId: string | null;
    disabled?: boolean;
    open?: boolean;
    onselect: (id: string) => void | Promise<void>;
  };

  let {
    profiles,
    activeId,
    disabled = false,
    open = $bindable(false),
    onselect,
  }: Props = $props();

  let root: HTMLDivElement;
  let trigger: HTMLButtonElement;
  let optionButtons = $state<HTMLButtonElement[]>([]);
  let highlighted = $state(0);

  const activeProfile = $derived(profiles.find((profile) => profile.id === activeId) ?? null);

  function activeIndex() {
    const index = profiles.findIndex((profile) => profile.id === activeId);
    return index >= 0 ? index : 0;
  }

  function focusOption(index: number) {
    if (profiles.length === 0) return;
    highlighted = Math.max(0, Math.min(index, profiles.length - 1));
    optionButtons[highlighted]?.focus();
    optionButtons[highlighted]?.scrollIntoView({ block: "nearest" });
  }

  function showPicker(index = activeIndex()) {
    if (disabled || profiles.length === 0) return;
    open = true;
    highlighted = index;
    requestAnimationFrame(() => focusOption(index));
  }

  function dismiss(returnFocus = true) {
    if (!open) return;
    open = false;
    if (returnFocus) requestAnimationFrame(() => trigger?.focus());
  }

  function togglePicker() {
    if (open) dismiss();
    else showPicker();
  }

  function choose(id: string) {
    dismiss();
    if (id !== activeId) void onselect(id);
  }

  function onTriggerKeydown(event: KeyboardEvent) {
    if (disabled) return;
    if (event.key === "ArrowDown") {
      event.preventDefault();
      showPicker(open ? highlighted + 1 : activeIndex());
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      showPicker(open ? highlighted - 1 : activeIndex());
    } else if (event.key === "Home") {
      event.preventDefault();
      showPicker(0);
    } else if (event.key === "End") {
      event.preventDefault();
      showPicker(profiles.length - 1);
    } else if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      togglePicker();
    } else if (event.key === "Escape" && open) {
      event.preventDefault();
      dismiss();
    } else if (event.key === "Tab" && open) {
      open = false;
    }
  }

  function onListKeydown(event: KeyboardEvent) {
    const current = optionButtons.findIndex((button) => button === document.activeElement);
    if (event.key === "ArrowDown") {
      event.preventDefault();
      focusOption(current < 0 ? activeIndex() : current + 1);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      focusOption(current < 0 ? activeIndex() : current - 1);
    } else if (event.key === "Home") {
      event.preventDefault();
      focusOption(0);
    } else if (event.key === "End") {
      event.preventDefault();
      focusOption(profiles.length - 1);
    } else if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      const profile = profiles[current >= 0 ? current : highlighted];
      if (profile) choose(profile.id);
    } else if (event.key === "Escape") {
      event.preventDefault();
      dismiss();
    } else if (event.key === "Tab") {
      open = false;
    }
  }

  onMount(() => {
    const onPointerDown = (event: PointerEvent) => {
      if (open && !root.contains(event.target as Node)) dismiss(false);
    };
    document.addEventListener("pointerdown", onPointerDown);
    return () => document.removeEventListener("pointerdown", onPointerDown);
  });
</script>

<div class="picker" bind:this={root}>
  <button
    bind:this={trigger}
    type="button"
    class="trigger"
    aria-label="Active profile"
    aria-haspopup="listbox"
    aria-expanded={open}
    aria-controls="profile-picker-list"
    {disabled}
    onclick={togglePicker}
    onkeydown={onTriggerKeydown}
  >
    <span class="trigger-label" title={activeProfile?.name ?? "No profiles"}>
      {activeProfile?.name ?? "No profiles"}
    </span>
    <Icon name="chevron-down" size={15} />
  </button>

  {#if open}
    <div
      id="profile-picker-list"
      class="list"
      role="listbox"
      aria-label="Profiles"
      tabindex="-1"
      onkeydown={onListKeydown}
    >
      {#each profiles as profile, index (profile.id)}
        <button
          bind:this={optionButtons[index]}
          type="button"
          class="option"
          class:active={profile.id === activeId}
          role="option"
          aria-selected={profile.id === activeId}
          tabindex={index === highlighted ? 0 : -1}
          onclick={() => choose(profile.id)}
        >
          <span class="check" aria-hidden="true">
            {#if profile.id === activeId}<Icon name="check" size={16} />{/if}
          </span>
          <span class="option-label" title={profile.name}>{profile.name}</span>
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .picker {
    position: relative;
    min-width: 0;
  }
  .trigger {
    display: inline-flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--control-icon-gap);
    width: 12rem;
    min-height: var(--control-height);
    padding: 0 var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--control-radius);
    background: var(--panel-2);
    color: var(--text);
    font-size: var(--control-font-size);
    font-weight: var(--control-font-weight);
    cursor: pointer;
  }
  .trigger:hover:not(:disabled),
  .trigger[aria-expanded="true"] { border-color: var(--accent); }
  .trigger:disabled { opacity: 0.5; cursor: not-allowed; }
  .trigger-label,
  .option-label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .list {
    position: absolute;
    top: calc(100% + var(--space-2));
    right: 0;
    z-index: 35;
    width: max(100%, 15rem);
    max-width: min(22rem, calc(100vw - 2rem));
    max-height: 18rem;
    overflow-y: auto;
    padding: var(--space-2);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius);
    background: var(--panel-raised);
    box-shadow: var(--shadow-lg);
    scrollbar-width: thin;
  }
  .option {
    display: grid;
    grid-template-columns: 1.25rem minmax(0, 1fr);
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    min-height: var(--control-height);
    padding: 0 var(--space-3) 0 var(--space-2);
    border: 0;
    border-radius: var(--control-radius);
    background: transparent;
    color: var(--text);
    font-size: var(--control-font-size);
    text-align: left;
    cursor: pointer;
  }
  .option:hover,
  .option:focus-visible { background: var(--panel-2); }
  .option.active { color: var(--accent-light); font-weight: var(--control-font-weight); }
  .check { display: grid; place-items: center; color: var(--accent); }
  @media (max-width: 760px) {
    .picker { flex: 1; }
    .trigger { width: 100%; }
    .list { left: 0; right: auto; width: 100%; }
  }
</style>
