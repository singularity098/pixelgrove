<script lang="ts">
  let { title, milestone }: { title: string; milestone: string } = $props();
</script>

<section class="page">
  <h2>{title}</h2>
  <p class="muted">Coming in milestone {milestone}.</p>
</section>
