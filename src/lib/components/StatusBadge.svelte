<script lang="ts">
  import type { Snippet } from "svelte";

  type Props = {
    tone?: "neutral" | "success" | "danger" | "warn" | "info";
    title?: string;
    children: Snippet;
  };

  let { tone = "neutral", title, children }: Props = $props();
</script>

<span class="badge {tone}" {title}>{@render children()}</span>

<style>
  .badge {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    font-size: 0.8rem;
    line-height: 1;
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border);
    border-radius: 999px;
    color: var(--text-muted);
  }
  .success {
    color: var(--success);
    border-color: var(--success);
  }
  .danger {
    color: var(--danger);
    border-color: var(--danger);
  }
  .warn {
    color: var(--warn);
    border-color: var(--warn);
  }
  .info {
    color: var(--info);
    border-color: var(--info);
  }
</style>
