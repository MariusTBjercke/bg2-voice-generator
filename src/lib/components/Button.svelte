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
    font: inherit;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: var(--space-2) var(--space-4);
    cursor: pointer;
    transition: background 0.12s ease, border-color 0.12s ease;
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
  }
  .ghost {
    background: var(--panel-2);
    color: var(--text);
  }
  .ghost:hover:not(:disabled) {
    border-color: var(--accent);
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
