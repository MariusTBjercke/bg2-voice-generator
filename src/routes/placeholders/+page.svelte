<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";

  onMount(() => {
    void goto("/dictionary", { replaceState: true });
  });
</script>

<p>Opening Dictionary…</p>
