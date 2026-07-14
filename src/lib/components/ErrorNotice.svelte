<script lang="ts">
  let { message }: { message?: string | null } = $props();
</script>

{#if message}
  <div class="error-notice" role="alert">{message}</div>
{/if}

<style>
  .error-notice {
    background: var(--panel);
    border: 1px solid var(--danger);
    border-radius: var(--radius);
    padding: var(--space-3) var(--space-4);
    color: var(--danger);
  }
</style>
