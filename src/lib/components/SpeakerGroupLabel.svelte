<script lang="ts">
  import type { SpeakerGroup } from "$lib/types";
  import { groupSummary } from "$lib/speakers/groups";

  type Props = {
    group: SpeakerGroup;
    sublabel?: string | null;
  };

  let { group, sublabel = null }: Props = $props();

  const summary = $derived(sublabel ?? groupSummary(group));
</script>

<span class="wrap">
  <span class="name">{group.display_name}</span>
  {#if summary}
    <span class="sub">{summary}</span>
  {/if}
</span>

<style>
  .wrap {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }
  .name {
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sub {
    font-size: 0.8rem;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
