<script lang="ts">
  // A reusable progress indicator (item-06b). Determinate when `max` is a positive
  // number (fills to value/max and shows a %); indeterminate otherwise (an animated
  // sweep for operations with no known total). Uses only the shared CSS variables.
  type Props = {
    /** A short label for the operation (e.g. "Harvesting references"). */
    label: string;
    /** Items processed so far (determinate mode). */
    value?: number;
    /** Total items; omit/null for an indeterminate bar. */
    max?: number | null;
    /** Optional detail line under the bar (e.g. the current speaker/line). */
    message?: string | null;
  };

  let { label, value = 0, max = null, message = null }: Props = $props();

  const determinate = $derived(typeof max === "number" && max > 0);
  const pct = $derived(
    determinate ? Math.max(0, Math.min(100, Math.round((value / (max as number)) * 100))) : 0,
  );
</script>

<div class="progress" role="status" aria-live="polite">
  <div class="head">
    <span class="label">{label}</span>
    {#if determinate}
      <span class="count">{value} / {max} · {pct}%</span>
    {:else}
      <span class="count">{value > 0 ? `${value}…` : "working…"}</span>
    {/if}
  </div>
  <div
    class="track"
    class:indeterminate={!determinate}
    role="progressbar"
    aria-valuemin={0}
    aria-valuemax={determinate ? (max as number) : undefined}
    aria-valuenow={determinate ? value : undefined}
  >
    <div class="fill" style={determinate ? `width:${pct}%` : ""}></div>
  </div>
  {#if message}
    <p class="message" title={message}>{message}</p>
  {/if}
</div>

<style>
  .progress {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    width: 100%;
  }
  .head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-3);
    font-size: 0.85rem;
  }
  .label {
    font-weight: 600;
  }
  .count {
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .track {
    position: relative;
    height: 0.5rem;
    background: var(--panel-2);
    border: 1px solid var(--border);
    border-radius: 999px;
    overflow: hidden;
  }
  .fill {
    height: 100%;
    background: var(--accent);
    border-radius: 999px;
    transition: width 0.15s ease;
  }
  /* Indeterminate: a fixed-width sweep animating across the track. */
  .indeterminate .fill {
    width: 35%;
    animation: sweep 1.1s ease-in-out infinite;
  }
  @keyframes sweep {
    0% {
      transform: translateX(-120%);
    }
    100% {
      transform: translateX(320%);
    }
  }
  .message {
    margin: 0;
    font-size: 0.78rem;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  @media (prefers-reduced-motion: reduce) {
    .indeterminate .fill {
      animation: none;
      width: 100%;
      opacity: 0.5;
    }
  }
</style>
