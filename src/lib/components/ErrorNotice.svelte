<script lang="ts">
  let { message }: { message?: string | null } = $props();
</script>

{#if message}
  <div class="error-notice" role="alert">{message}</div>
{/if}

<style>
  .error-notice {
    background: var(--danger-wash);
    border: 1px solid var(--danger);
    border-radius: var(--radius);
    padding: var(--space-4);
    color: color-mix(in srgb, var(--danger) 82%, white);
    box-shadow: inset 3px 0 0 var(--danger);
  }
</style>
