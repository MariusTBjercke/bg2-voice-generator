<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLButtonAttributes } from "svelte/elements";

  type Props = HTMLButtonAttributes & {
    variant?: "primary" | "ghost" | "danger";
    children: Snippet;
  };

  let { variant = "primary", children, ...rest }: Props = $props();
</script>

<button class="btn {variant}" {...rest}>{@render children()}</button>

<style>
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--control-icon-gap);
    border: 1px solid var(--border);
    border-radius: var(--control-radius);
    padding: 0 var(--space-4);
    cursor: pointer;
    min-height: var(--control-height);
    font-size: var(--control-font-size);
    font-weight: var(--control-font-weight);
    line-height: 1.2;
    text-align: center;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.2);
    transition: background 0.12s ease, border-color 0.12s ease, transform 0.12s ease;
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .primary {
    background: var(--accent);
    color: var(--accent-ink);
    border-color: var(--accent);
  }
  .primary:hover:not(:disabled) {
    filter: brightness(1.08);
    transform: translateY(-1px);
  }
  .ghost {
    background: var(--panel-2);
    color: var(--text);
  }
  .ghost:hover:not(:disabled) {
    border-color: var(--accent);
    background: var(--panel-raised);
  }
  .danger {
    background: transparent;
    color: var(--danger);
    border-color: var(--danger);
  }
  .danger:hover:not(:disabled) {
    background: var(--danger);
    color: var(--accent-ink);
  }
</style>
